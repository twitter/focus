/*
  FUSE: Filesystem in Userspace
  Copyright (C) 2001-2007  Miklos Szeredi <miklos@szeredi.hu>
  Copyright (C) 2017       Nikolaus Rath <Nikolaus@rath.org>
  Copyright (C) 2018       Valve, Inc
  Copyright (C) 2020       Twitter, Inc

  This program can be distributed under the terms of the GNU GPLv2.
  See the file COPYING.
*/

// TODO: Idea: instead of storing a radix tree, just serialize
//       a representation of the moniker table.

// TODO: Replace sprintf instances with fmtlib

/** @file
 *
 * This is a "high-performance" version of passthrough_ll.c. While
 * passthrough_ll.c is designed to be as simple as possible, this
 * example intended to be as efficient and correct as possible.
 *
 * passthrough_hp.cc mirrors a specified "source" directory under a
 * specified the mountpoint with as much fidelity and performance as
 * possible.
 *
 * If --nocache is specified, the source directory may be changed
 * directly even while mounted and the filesystem will continue
 * to work correctly.
 *
 * Without --nocache, the source directory is assumed to be modified
 * only through the passthrough filesystem. This enables much better
 * performance, but if changes are made directly to the source, they
 * may not be immediately visible under the mountpoint and further
 * access to the mountpoint may result in incorrect behavior,
 * including data-loss.
 *
 * On its own, this filesystem fulfills no practical purpose. It is
 * intended as a template upon which additional functionality can be
 * built.
 *
 * Unless --nocache is specified, is only possible to write to files
 * for which the mounting user has read permissions. This is because
 * the writeback cache requires the kernel to be able to issue read
 * requests for all files (which the passthrough filesystem cannot
 * satisfy if it can't read the file in the underlying filesystem).
 *
 * ## Source code ##
 * \include fs.cc
 */

// TODO: Replace all primitive string functions

#define FUSE_USE_VERSION 35

#ifdef HAVE_CONFIG_H
#include "config.h"
#endif

#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif

// C includes
#include <dirent.h>
#include <err.h>
#include <fuse3/fuse_lowlevel.h>
#include <limits.h>
#include <signal.h>
#include <sys/file.h>
#include <sys/resource.h>
#include <sys/xattr.h>
#include <time.h>
#include <unistd.h>

// C++ includes
#include <gflags/gflags.h>
#include <glog/logging.h>

#include <array>
#include <cerrno>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <iomanip>
#include <list>
#include <mutex>
#include <stack>
#include <thread>
#include <vector>

#include "absl/container/flat_hash_map.h"
#include "absl/container/flat_hash_set.h"
#include "absl/container/node_hash_set.h"
#include "absl/strings/str_format.h"
#include "absl/synchronization/mutex.h"

#include "annotations.h"
#include "moniker.h"
#include "tracker.h"
#include "tablet.h"

#define UNUSED(x) (void)x;

DEFINE_string(source_directory, "", "Source directory");
DEFINE_string(target_directory, "", "Target directory");
DEFINE_bool(record_file_access, false, "Record file access");
DEFINE_bool(debug, false, "Enable debug logging");
DEFINE_bool(cache, true, "Enable caching");
DEFINE_bool(splice, true, "Use splice(2) to transfer data");
DEFINE_bool(multithreaded, true, "Use multi-threaded processing");
DEFINE_string(access_log_directory, "",
              "Log accesses to files in the given directory");
DEFINE_string(pid_file, "", "Write the PID of the process to the given file");

using namespace std;

static void quiesce();

/* We are re-using pointers to our `struct sfs_inode` and `struct
   sfs_dirp` elements as inodes and file handles. This means that we
   must be able to store pointer a pointer in both a fuse_ino_t
   variable and a uint64_t variable (used for file handles). */
static_assert(sizeof(fuse_ino_t) >= sizeof(void*),
              "void* must fit into fuse_ino_t");
static_assert(sizeof(fuse_ino_t) >= sizeof(uint64_t),
              "fuse_ino_t must be at least 64 bits");

/* Forward declarations */
struct Inode;
static Inode& get_inode(fuse_ino_t ino);
static void forget_one(fuse_ino_t ino, uint64_t n);

// Uniquely identifies a file in the source directory tree. This could
// be simplified to just ino_t since we require the source directory
// not to contain any mountpoints. This hasn't been done yet in case
// we need to reconsider this constraint (but relaxing this would have
// the drawback that we can no longer re-use inode numbers, and thus
// readdir() would need to do a full lookup() in order to report the
// right inode number).
typedef pair<ino_t, dev_t> SrcId;

// Define a hash function for SrcId
namespace std {
template <>
struct hash<SrcId> {
  size_t operator()(const SrcId& id) const {
    return hash<ino_t>{}(id.first) ^ hash<dev_t>{}(id.second);
  }
};
}  // namespace std

// Maps files in the source directory tree to inodes
// N.B. NodeHashMap is needed because we need pointer stability
typedef absl::node_hash_map<SrcId, Inode> InodeMap;

// Annotates the cause of a lookup
enum LookupCause {
  LookupCauseDirect,
  LookupCauseMknodSymlink,
  LookupCauseReaddir,
};

constexpr static array<string_view, 3> kLookupCauseAsString = {
    "Direct",
    "MknodSymlink",
    "Readdir",
};

struct Inode {
  Inode() = default;
  ~Inode();
  DISALLOW_COPY_AND_ASSIGN(Inode);
  DISALLOW_MOVE(Inode);

  int fd{-1};
  dev_t src_dev{0};
  ino_t src_ino{0};
  uint64_t nlookup{0};
  mutex m;
};

Inode::~Inode() {
  if (fd > 0) {
    close(fd);
  }
}

struct Fs {
  Fs() = default;
  DISALLOW_COPY_AND_ASSIGN(Fs);
  DISALLOW_MOVE(Fs);

  // Must be acquired *after* any Inode.m locks.
  std::mutex mutex;
  InodeMap inodes;  // protected by mutex
  Inode root;
  double timeout;
  string source;
  size_t blocksize;
  dev_t src_dev;
};

static Fs fs{};

inline fuse_buf_copy_flags get_buffer_copy_flags() {
  if (FLAGS_splice) {
    return static_cast<fuse_buf_copy_flags>(0);
  } else {
    return FUSE_BUF_NO_SPLICE;
  }
}

static Inode& get_inode(fuse_ino_t ino) {
  if (ino == FUSE_ROOT_ID) return fs.root;

  Inode* inode = reinterpret_cast<Inode*>(ino);
  if (inode->fd == -1) {
    LOG(FATAL) << "Unknown inode " << ino;
  }
  return *inode;
}

enum UseAttribution {
  LookupUseAttribution,
  MkdirUseAttribution,
  MknodUseAttribution,
  SymlinkUseAttribution,
  LinkUseAttribution,
  UnlinkUseAttribution,
  RmdirUseAttribution,
  RenameUseAttribution,
  ForgetUseAttribution,
  ForgetOneUseAttribution,
  ForgetMultiUseAttribution,
  GetattrUseAttribution,
  SetattrUseAttribution,
  ReadlinkUseAttribution,
  OpendirUseAttribution,
  ReaddirUseAttribution,
  ReaddirplusUseAttribution,
  ReleasedirUseAttribution,
  FsyncdirUseAttribution,
  CreateUseAttribution,
  OpenUseAttribution,
  ReleaseUseAttribution,
  FlushUseAttribution,
  FsyncUseAttribution,
  ReadUseAttribution,
  WriteBufUseAttribution,
  StatfsUseAttribution,
  FallocateUseAttribution,
  FlockUseAttribution,
  SetxattrUseAttribution,
  GetxattrUseAttribution,
  ListxattrUseAttribution,
  RemovexattrUseAttribution,
  QuiescenceUseAttribution,
};

constexpr static array<const char* const, 33> UseAttributionStrings = {
    "lookup",       "mkdir",       "mknod",      "symlink",  "link",
    "unlink",       "rmdir",       "rename",     "forget",   "forget_one",
    "forget_multi", "getattr",     "setattr",    "readlink", "opendir",
    "readdir",      "readdirplus", "releasedir", "fsyncdir", "create",
    "open",         "release",     "flush",      "fsync",    "read",
    "write_buf",    "statfs",      "fallocate",  "flock",    "setxattr",
    "getxattr",     "listxattr",   "removexattr"};

using namespace eulefs;
struct FsContext {
  MonikerTable& Monikers();
  void PopulateMonikerTable(const std::string& path, const bool include_files);

 private:
  std::unique_ptr<MonikerTable> table_;
};

MonikerTable& FsContext::Monikers() { return *table_; }

void FsContext::PopulateMonikerTable(const std::string& path,
                                     const bool include_files) {
  struct stat sb;
  PCHECK(stat(path.c_str(), &sb) == 0);
  const auto root_inode = sb.st_ino;
  table_ = std::make_unique<MonikerTable>(root_inode);
  const auto result =
      eulefs::AddFilesystemContentToMonikerTable(path, *table_, include_files);
  LOG(INFO) << "Added " << result << " nodes to moniker table";
}

FsContext* GetFsContext() {
  static FsContext fs_context{};
  return &fs_context;
}

class AttributionFrame;

Tablets* GetTablets() {
  static Tablets tablets;
  return &tablets;
}

class Context {
 public:
  explicit Context(Context* const parent);
  ~Context();

  DISALLOW_COPY_AND_ASSIGN(Context);
  DISALLOW_MOVE(Context);

  void Add(fuse_ino_t parent, SrcId id, const char* name, struct stat* sb);
  void Trace(fuse_ino_t inode, UseAttribution attribution);
  void WriteLog(int fd);
  void PrintStats();
  void SetEnabled(bool val);

  // Write the top level logs
  static void WriteLogs();

 protected:
  friend class AttributionFrame;
  friend void sfs_destroy(void*);
  friend void quiesce();
  friend void SetEnabled(bool);
  // void PushUseAttribution(const UseAttribution use);
  // void PopUseAttribution();
  // void Unwinding(); // Called by an attribution token. Unwinding has begun.
  // If there are pending attributions, log them.
  void AddUseAttribution(const UseAttribution attribution);
  void AddInode(const uint64_t inode);
  void Flush();
  absl::Mutex* Mutex();

 private:
  bool enabled_;
  Context* const parent_;
  absl::Mutex mu_;
};

// Epoch numbers are for sequencing log file names
static uint64_t NextEpoch() {
  static std::atomic<uint64_t> epoch = 0;
  return epoch.fetch_add(1, std::memory_order_relaxed);
}

Context::Context(Context* const parent)
    : enabled_(!FLAGS_access_log_directory.empty()), parent_(parent) {
  VLOG(2) << "Context " << this << " created with parent " << parent;
}

Context::~Context() {
}

void Context::Add(fuse_ino_t parent, SrcId id, const char* name,
                  struct stat* sb) {
  if (!parent_) {
    LOG(FATAL) << "Cannot be called on the root instance";
  }
  if (!enabled_) {
    return;
  }

  AddInode(std::get<0>(id));
}

void Context::AddInode(const uint64_t inode) {
  std::shared_ptr<Tablet> tablet = GetTablets()->GetTabletForThisThread();
  tablet->Insert(inode);
}

absl::Mutex* Context::Mutex() { return &mu_; }

enum Endianness {
  ENDIANNESS_BIG,
  ENDIANNESS_LITTLE,
};

Endianness MachineEndianness(void) {
  union {
    uint32_t i;
    char c[4];
  } bint = {0x01020304};

  if (bint.c[0] == 1) {
    return ENDIANNESS_BIG;
  } else {
    return ENDIANNESS_LITTLE;
  }
}

void Context::Flush() {
  // TODO: Erase this once we confirm it's unnecessary
}

void Context::SetEnabled(bool val) { enabled_ = val; }

static ssize_t TryWrite(int fd, const void* buf, size_t buf_len) {
  ssize_t result;
  while (buf_len > 0) {
    do {
      result = write(fd, buf, buf_len);
    } while ((result < 0) &&
             (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK));

    if (result < 0) {
      return result;
    }

    buf_len -= result;
    buf += result;
  }
  return 0;
}

const size_t kLogWriterBufferSize = 4 * 1024 * 1024;

static bool TryFsync(int fd, size_t tries) {
  for (; tries > 0; --tries) {
    if (fsync(fd) == 0) {
      return true;
    }
  }

  LOG(ERROR) << "fsync failed after " << tries << " tries: " << strerror(errno);
  return false;
}

void Context::WriteLog(const int fd) {
  if (!enabled_) {
    LOG(INFO) << "Skipping log write because context (" << this << ") is disabled";
    return;
  }

  auto buf = std::string{};
  buf.reserve(kLogWriterBufferSize);

  // Aggregate tablets!
  Tablet aggregated;
  LOG(INFO) << "Starting to aggregate tablets";
  GetTablets()->Sweep(aggregated);
  LOG(INFO) << "Finished aggregating tablets";
  auto mutex = absl::MutexLock { aggregated.Mutex() }; // ::Data() prefers a mutex exist
  auto data = aggregated.Data();

  auto& monikers = GetFsContext()->Monikers();
  for (auto it = data->cbegin(); it != data->cend(); ++it) {
    const uint64_t inode = *it;
    const auto path = monikers.Get(inode, 1);
    if (path) {
      const auto size = path->size();
      if (buf.capacity() < size) {
        auto written = TryWrite(fd, buf.data(), buf.size());
        PCHECK(written == 0);
        buf.clear();
      }
      buf += *path;
      buf += "\n";
    } else {
      VLOG(2) << "Missing inode " << inode;
    }
  }

  auto written = TryWrite(fd, buf.data(), buf.size());
  PCHECK(written == 0);
  buf.clear();

  TryFsync(fd, 5);
}

void Context::PrintStats() {}

thread_local std::stack<AttributionFrame*> attribution_stack;

// TODO: Contexts need refactoring. The only thing thread contexts are logically
// doing right now is accumulating use attributions.
class AttributionFrame {
 public:
  AttributionFrame(Context* const context, const UseAttribution attribution);
  AttributionFrame(Context* const context, const UseAttribution attribution,
                   const fuse_ino_t inode);
  ~AttributionFrame();
  DISALLOW_COPY_AND_ASSIGN(AttributionFrame);

  static std::optional<AttributionFrame*> Current();

 private:
  Context* const context_;
  bool handled_;
};

AttributionFrame::AttributionFrame(Context* const context,
                                   const UseAttribution attribution)
    : context_(context) {
  // context_->AddUseAttribution(attribution);
  attribution_stack.push(this);
}

AttributionFrame::AttributionFrame(Context* const context,
                                   const UseAttribution attribution,
                                   const fuse_ino_t inode)
    : AttributionFrame(context, attribution) {
  context_->AddInode(inode);
}

AttributionFrame::~AttributionFrame() {
  attribution_stack.pop();
  if (attribution_stack.empty()) {
    context_->Flush();
  }
}

std::optional<AttributionFrame*> AttributionFrame::Current() {
  if (attribution_stack.empty()) {
    return nullopt;
  }

  return attribution_stack.top();
}

Context* GetRootContext() {
  static Context root_context{nullptr};
  return &root_context;
}

Context* GetThreadContext() {
  thread_local Context thread_context{GetRootContext()};
  return &thread_context;
}

void Context::WriteLogs() {
  if (FLAGS_access_log_directory.empty()) {
    LOG(INFO) << "Logging is not enabled (access_log_directory parameter is not set)";
    return;
  }
  
  auto ctx = GetRootContext();
  auto lock = absl::MutexLock{ctx->Mutex()};
  auto log_path = absl::StrFormat("%s/%u.%u.log", FLAGS_access_log_directory,
                                  getpid(), NextEpoch());
  int log_fd = open(log_path.c_str(), O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC,
                    S_IRUSR | S_IWUSR | S_IRGRP | S_IROTH);
  LOG(INFO) << "Begin writing log to " << log_path;
  PCHECK(log_fd >= 0);
  GetRootContext()->WriteLog(log_fd);
  PCHECK(close(log_fd) == 0);
  LOG(INFO) << "Finished writing log to " << log_path;
}

static int get_fs_fd(fuse_ino_t ino) {
  int fd = get_inode(ino).fd;
  return fd;
}

void SetEnabled(bool val) {
  auto ctx = GetRootContext();
  auto lock = absl::MutexLock{ctx->Mutex()};
  ctx->SetEnabled(val);
}

static void sfs_init(void* userdata, fuse_conn_info* conn) {
  UNUSED(userdata);
  if (conn->capable & FUSE_CAP_EXPORT_SUPPORT) {
    conn->want |= FUSE_CAP_EXPORT_SUPPORT;
  }

  if (fs.timeout && conn->capable & FUSE_CAP_WRITEBACK_CACHE) {
    conn->want |= FUSE_CAP_WRITEBACK_CACHE;
  }

  if (conn->capable & FUSE_CAP_FLOCK_LOCKS) {
    conn->want |= FUSE_CAP_FLOCK_LOCKS;
  }

  // Use splicing if supported. Since we are using writeback caching
  // and readahead, individual requests should have a decent size so
  // that splicing between fd's is well worth it.
  if (conn->capable & FUSE_CAP_SPLICE_WRITE && FLAGS_splice) {
    conn->want |= FUSE_CAP_SPLICE_WRITE;
  }
  if (conn->capable & FUSE_CAP_SPLICE_READ && FLAGS_splice) {
    conn->want |= FUSE_CAP_SPLICE_READ;
  }
}

void sfs_destroy(void* userdata) {
  UNUSED(userdata);
  // TODO: Use userdata to store our context?
  quiesce();
}

static void sfs_getattr(fuse_req_t req, fuse_ino_t ino, fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), GetattrUseAttribution, ino);
  UNUSED(fi);
  Inode& inode = get_inode(ino);
  struct stat attr;
  auto res = fstatat(inode.fd, "", &attr, AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW);
  if (res == -1) {
    fuse_reply_err(req, errno);
    return;
  }
  fuse_reply_attr(req, &attr, fs.timeout);
}

static void do_setattr(fuse_req_t req, fuse_ino_t ino, struct stat* attr,
                       int valid, struct fuse_file_info* fi) {
  Inode& inode = get_inode(ino);
  int ifd = inode.fd;
  int res;

  if (valid & FUSE_SET_ATTR_MODE) {
    if (fi) {
      res = fchmod(fi->fh, attr->st_mode);
    } else {
      char procname[64];
      sprintf(procname, "/proc/self/fd/%i", ifd);
      res = chmod(procname, attr->st_mode);
    }
    if (res == -1) {
      goto out_err;
    }
  }
  if (valid & (FUSE_SET_ATTR_UID | FUSE_SET_ATTR_GID)) {
    uid_t uid =
        (valid & FUSE_SET_ATTR_UID) ? attr->st_uid : static_cast<uid_t>(-1);
    gid_t gid =
        (valid & FUSE_SET_ATTR_GID) ? attr->st_gid : static_cast<gid_t>(-1);

    res = fchownat(ifd, "", uid, gid, AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW);
    if (res == -1) {
      goto out_err;
    }
  }
  if (valid & FUSE_SET_ATTR_SIZE) {
    if (fi) {
      res = ftruncate(fi->fh, attr->st_size);
    } else {
      char procname[64];
      sprintf(procname, "/proc/self/fd/%i", ifd);
      res = truncate(procname, attr->st_size);
    }
    if (res == -1) {
      goto out_err;
    }
  }
  if (valid & (FUSE_SET_ATTR_ATIME | FUSE_SET_ATTR_MTIME)) {
    struct timespec tv[2];

    tv[0].tv_sec = 0;
    tv[1].tv_sec = 0;
    tv[0].tv_nsec = UTIME_OMIT;
    tv[1].tv_nsec = UTIME_OMIT;

    if (valid & FUSE_SET_ATTR_ATIME_NOW) {
      tv[0].tv_nsec = UTIME_NOW;
    } else if (valid & FUSE_SET_ATTR_ATIME) {
      tv[0] = attr->st_atim;
    }

    if (valid & FUSE_SET_ATTR_MTIME_NOW) {
      tv[1].tv_nsec = UTIME_NOW;

    } else if (valid & FUSE_SET_ATTR_MTIME) {
      tv[1] = attr->st_mtim;
    }

    if (fi) {
      res = futimens(fi->fh, tv);
    } else {
#ifdef HAVE_UTIMENSAT
      char procname[64];
      sprintf(procname, "/proc/self/fd/%i", ifd);
      res = utimensat(AT_FDCWD, procname, tv, 0);
#else
      res = -1;
      errno = EOPNOTSUPP;
#endif
    }
    if (res == -1) {
      goto out_err;
    }
  }
  return sfs_getattr(req, ino, fi);

out_err:
  fuse_reply_err(req, errno);
}

static void sfs_setattr(fuse_req_t req, fuse_ino_t ino, struct stat* attr,
                        int valid, fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), SetattrUseAttribution, ino);
  do_setattr(req, ino, attr, valid, fi);
}

static int do_lookup(fuse_ino_t parent, const char* name, fuse_entry_param* e) {
  VLOG(4) << "lookup(): name=" << name << ", parent=" << parent;

  memset(e, 0, sizeof(*e));
  e->attr_timeout = fs.timeout;
  e->entry_timeout = fs.timeout;

  auto newfd = openat(get_fs_fd(parent), name, O_PATH | O_NOFOLLOW);
  if (newfd == -1) return errno;

  auto res = fstatat(newfd, "", &e->attr, AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW);
  if (res == -1) {
    auto saveerr = errno;
    close(newfd);
    VLOG(3) << "lookup(): fstatat failed";
    return saveerr;
  }

  if (e->attr.st_dev != fs.src_dev) {
    LOG(WARNING) << "Mountpoints in the source directory tree will be hidden.";
    return ENOTSUP;
  } else if (e->attr.st_ino == FUSE_ROOT_ID) {
    LOG(ERROR) << "Source directory tree must not include inode "
               << FUSE_ROOT_ID;
    return EIO;
  }

  SrcId id{e->attr.st_ino, e->attr.st_dev};
  unique_lock<mutex> fs_lock{fs.mutex};
  Inode* inode_p;
  try {
    inode_p = &fs.inodes[id];
  } catch (bad_alloc&) {
    return ENOMEM;
  }
  e->ino = reinterpret_cast<fuse_ino_t>(inode_p);
  Inode& inode = *inode_p;

  GetThreadContext()->Add(parent, id, name, &e->attr);

  if (inode.fd != -1) {  // found existing inode
    fs_lock.unlock();

    VLOG(4) << "lookup(): inode " << e->attr.st_ino
            << " (userspace) already known.";
    lock_guard<mutex> g{inode.m};
    inode.nlookup++;
    close(newfd);
  } else {  // no existing inode
    /* This is just here to make Helgrind happy. It violates the
       lock ordering requirement (inode.m must be acquired before
       fs.mutex), but this is of no consequence because at this
       point no other thread has access to the inode mutex */
    lock_guard<mutex> g{inode.m};
    inode.src_ino = e->attr.st_ino;
    inode.src_dev = e->attr.st_dev;
    inode.nlookup = 1;
    inode.fd = newfd;
    fs_lock.unlock();

    VLOG(4) << "lookup(): created userspace inode " << e->attr.st_ino;
  }

  return 0;
}

static void sfs_lookup(fuse_req_t req, fuse_ino_t parent, const char* name) {
  // TODO: PID filtering?
  AttributionFrame token(GetThreadContext(), LookupUseAttribution, parent);

  fuse_entry_param e{};
  auto err = do_lookup(parent, name, &e);
  if (err == ENOENT) {
    e.attr_timeout = fs.timeout;
    e.entry_timeout = fs.timeout;
    e.ino = e.attr.st_ino = 0;
    fuse_reply_entry(req, &e);
  } else if (err) {
    if (err == ENFILE || err == EMFILE)
      LOG(ERROR) << "Reached maximum number of file descriptors.";
    fuse_reply_err(req, err);
  } else {
    fuse_reply_entry(req, &e);
  }
}

static void mknod_symlink(fuse_req_t req, fuse_ino_t parent, const char* name,
                          mode_t mode, dev_t rdev, const char* link) {
  int res;
  Inode& inode_p = get_inode(parent);
  auto saverr = ENOMEM;

  if (S_ISDIR(mode)) {
    res = mkdirat(inode_p.fd, name, mode);
  } else if (S_ISLNK(mode)) {
    res = symlinkat(link, inode_p.fd, name);

  } else {
    res = mknodat(inode_p.fd, name, mode, rdev);
  }
  saverr = errno;
  if (res == -1) {
    goto out;
  }

  fuse_entry_param e;
  saverr = do_lookup(parent, name, &e);
  if (saverr) {
    goto out;
  }

  fuse_reply_entry(req, &e);
  return;

out:
  if (saverr == ENFILE || saverr == EMFILE) {
    DLOG(ERROR) << "Reached maximum number of file descriptors.";
  }
  fuse_reply_err(req, saverr);
}

static void sfs_mknod(fuse_req_t req, fuse_ino_t parent, const char* name,
                      mode_t mode, dev_t rdev) {
  AttributionFrame token(GetThreadContext(), MknodUseAttribution, parent);
  mknod_symlink(req, parent, name, mode, rdev, nullptr);
}

static void sfs_mkdir(fuse_req_t req, fuse_ino_t parent, const char* name,
                      mode_t mode) {
  AttributionFrame token(GetThreadContext(), MkdirUseAttribution, parent);
  mknod_symlink(req, parent, name, S_IFDIR | mode, 0, nullptr);
}

static void sfs_symlink(fuse_req_t req, const char* link, fuse_ino_t parent,
                        const char* name) {
  AttributionFrame token(GetThreadContext(), SymlinkUseAttribution, parent);
  mknod_symlink(req, parent, name, S_IFLNK, 0, link);
}

static void sfs_link(fuse_req_t req, fuse_ino_t ino, fuse_ino_t parent,
                     const char* name) {
  AttributionFrame token(GetThreadContext(), LinkUseAttribution, parent);
  Inode& inode = get_inode(ino);
  Inode& inode_p = get_inode(parent);
  fuse_entry_param e{};

  e.attr_timeout = fs.timeout;
  e.entry_timeout = fs.timeout;

  char procname[64];
  sprintf(procname, "/proc/self/fd/%i", inode.fd);
  auto res = linkat(AT_FDCWD, procname, inode_p.fd, name, AT_SYMLINK_FOLLOW);
  if (res == -1) {
    fuse_reply_err(req, errno);
    return;
  }

  res = fstatat(inode.fd, "", &e.attr, AT_EMPTY_PATH | AT_SYMLINK_NOFOLLOW);
  if (res == -1) {
    fuse_reply_err(req, errno);
    return;
  }
  e.ino = reinterpret_cast<fuse_ino_t>(&inode);
  {
    lock_guard<mutex> g{inode.m};
    inode.nlookup++;
  }

  fuse_reply_entry(req, &e);
  return;
}

static void sfs_rmdir(fuse_req_t req, fuse_ino_t parent, const char* name) {
  AttributionFrame token(GetThreadContext(), RmdirUseAttribution, parent);
  Inode& inode_p = get_inode(parent);
  lock_guard<mutex> g{inode_p.m};
  auto res = unlinkat(inode_p.fd, name, AT_REMOVEDIR);
  fuse_reply_err(req, res == -1 ? errno : 0);
}

static void sfs_rename(fuse_req_t req, fuse_ino_t parent, const char* name,
                       fuse_ino_t newparent, const char* newname,
                       unsigned int flags) {
  AttributionFrame token(GetThreadContext(), RenameUseAttribution, parent);
  Inode& inode_p = get_inode(parent);
  Inode& inode_np = get_inode(newparent);
  if (flags) {
    fuse_reply_err(req, EINVAL);
    return;
  }

  auto res = renameat(inode_p.fd, name, inode_np.fd, newname);
  fuse_reply_err(req, res == -1 ? errno : 0);
}

static void sfs_unlink(fuse_req_t req, fuse_ino_t parent, const char* name) {
  AttributionFrame token(GetThreadContext(), UnlinkUseAttribution);
  Inode& inode_p = get_inode(parent);
  auto res = unlinkat(inode_p.fd, name, 0);
  fuse_reply_err(req, res == -1 ? errno : 0);
}

static void forget_one(fuse_ino_t ino, uint64_t n) {
  AttributionFrame token(GetThreadContext(), ForgetOneUseAttribution, ino);
  Inode& inode = get_inode(ino);
  unique_lock<mutex> l{inode.m};

  if (n > inode.nlookup) {
    LOG(FATAL) << "Negative lookup count for inode " << inode.src_ino;
  }
  inode.nlookup -= n;
  if (!inode.nlookup) {
    VLOG(4) << "forget: cleaning up inode " << inode.src_ino;
    {
      lock_guard<mutex> g_fs{fs.mutex};
      l.unlock();
      fs.inodes.erase({inode.src_ino, inode.src_dev});
    }
  } else {
    VLOG(4) << "forget: inode " << inode.src_ino << " lookup count now "
            << inode.nlookup;
  }
}

static void sfs_forget(fuse_req_t req, fuse_ino_t ino, uint64_t nlookup) {
  AttributionFrame token(GetThreadContext(), ForgetUseAttribution);
  forget_one(ino, nlookup);
  fuse_reply_none(req);
}

static void sfs_forget_multi(fuse_req_t req, size_t count,
                             fuse_forget_data* forgets) {
  AttributionFrame token(GetThreadContext(), ForgetMultiUseAttribution);
  for (size_t i = 0; i < count; i++) {
    forget_one(forgets[i].ino, forgets[i].nlookup);
  }
  fuse_reply_none(req);
}

static void sfs_readlink(fuse_req_t req, fuse_ino_t ino) {
  AttributionFrame token(GetThreadContext(), ReadlinkUseAttribution, ino);
  Inode& inode = get_inode(ino);
  char buf[PATH_MAX + 1];
  auto res = readlinkat(inode.fd, "", buf, sizeof(buf));

  if (res == -1) {
    fuse_reply_err(req, errno);

  } else if (res == sizeof(buf)) {
    fuse_reply_err(req, ENAMETOOLONG);
  } else {
    buf[res] = '\0';
    fuse_reply_readlink(req, buf);
  }
}

struct DirHandle {
  DIR* dp{nullptr};
  off_t offset;

  DirHandle() = default;
  DISALLOW_COPY_AND_ASSIGN(DirHandle);

  ~DirHandle() {
    if (dp) {
      closedir(dp);
    }
  }
};

static DirHandle* get_dir_handle(fuse_file_info* fi) {
  return reinterpret_cast<DirHandle*>(fi->fh);
}

static void sfs_opendir(fuse_req_t req, fuse_ino_t ino, fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), OpendirUseAttribution, ino);
  Inode& inode = get_inode(ino);
  auto d = new (nothrow) DirHandle;
  if (d == nullptr) {
    fuse_reply_err(req, ENOMEM);
    return;
  }

  // Make Helgrind happy - it can't know that there's an implicit
  // synchronization due to the fact that other threads cannot
  // access d until we've called fuse_reply_*.
  lock_guard<mutex> g{inode.m};

  auto fd = openat(inode.fd, ".", O_RDONLY);
  if (fd == -1) {
    goto out_errno;
  }

  // On success, dir stream takes ownership of fd, so we
  // do not have to close it.
  d->dp = fdopendir(fd);
  if (d->dp == nullptr) {
    goto out_errno;
  }

  d->offset = 0;

  fi->fh = reinterpret_cast<uint64_t>(d);
  if (fs.timeout) {
    fi->keep_cache = 1;
    fi->cache_readdir = 1;
  }
  fuse_reply_open(req, fi);
  return;

out_errno:
  auto error = errno;
  delete d;
  if (error == ENFILE || error == EMFILE) {
    LOG(WARNING) << "Reached maximum number of file descriptors.";
  }
  fuse_reply_err(req, error);
}

static bool is_dot_or_dotdot(const char* name) {
  return name[0] == '.' &&
         (name[1] == '\0' || (name[1] == '.' && name[2] == '\0'));
}

static void do_readdir(fuse_req_t req, fuse_ino_t ino, size_t size,
                       off_t offset, fuse_file_info* fi, int plus) {
  AttributionFrame token(GetThreadContext(), ReaddirUseAttribution, ino);

  auto d = get_dir_handle(fi);
  Inode& inode = get_inode(ino);
  lock_guard<mutex> g{inode.m};
  char* p;
  auto rem = size;
  int err = 0, count = 0;

  VLOG(4) << "readdir(): started with offset " << offset;

  auto buf = new (nothrow) char[size];
  if (!buf) {
    fuse_reply_err(req, ENOMEM);
    return;
  }
  p = buf;

  if (offset != d->offset) {
    VLOG(4) << "readdir(): seeking to " << offset;
    seekdir(d->dp, offset);
    d->offset = offset;
  }

  while (1) {
    struct dirent* entry;
    errno = 0;
    entry = readdir(d->dp);
    if (!entry) {
      if (errno) {
        err = errno;
        LOG(WARNING) << "readdir(): readdir failed with " << strerror(errno);
        goto error;
      }
      break;  // End of stream
    }
    d->offset = entry->d_off;
    if (is_dot_or_dotdot(entry->d_name)) {
      continue;
    }

    fuse_entry_param e{};
    size_t entsize;
    if (plus) {
      err = do_lookup(ino, entry->d_name, &e);
      if (err) {
        goto error;
      }
      entsize =
          fuse_add_direntry_plus(req, p, rem, entry->d_name, &e, entry->d_off);

      if (entsize > rem) {
        VLOG(4) << "readdir(): buffer full, returning data. ";
        forget_one(e.ino, 1);
        break;
      }
    } else {
      e.attr.st_ino = entry->d_ino;
      e.attr.st_mode = entry->d_type << 12;
      entsize =
          fuse_add_direntry(req, p, rem, entry->d_name, &e.attr, entry->d_off);

      // Add to the index here since otherwise we will lose track.
      auto id = SrcId{entry->d_ino, 0};
      GetThreadContext()->Add(ino, id, entry->d_name, nullptr);

      if (entsize > rem) {
        VLOG(4) << "readdir(): buffer full, returning data. ";
        break;
      }
    }

    p += entsize;
    rem -= entsize;
    count++;
    {
      VLOG(4) << "readdir(): added to buffer: " << entry->d_name << ", ino "
              << e.attr.st_ino << ", offset " << entry->d_off;
    }
  }
  err = 0;
error:

  // If there's an error, we can only signal it if we haven't stored
  // any entries yet - otherwise we'd end up with wrong lookup
  // counts for the entries that are already in the buffer. So we
  // return what we've collected until that point.
  if (err && rem == size) {
    if (err == ENFILE || err == EMFILE) {
      LOG(WARNING) << "ERROR: Reached maximum number of file descriptors.";
    }
    fuse_reply_err(req, err);
  } else {
    VLOG(4) << "readdir(): returning " << count << " entries, curr offset "
            << d->offset;
    fuse_reply_buf(req, buf, size - rem);
  }
  delete[] buf;
  return;
}

static void sfs_readdir(fuse_req_t req, fuse_ino_t ino, size_t size,
                        off_t offset, fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), ReaddirUseAttribution, ino);
  // operation logging is done in readdir to reduce code duplication
  do_readdir(req, ino, size, offset, fi, 0);
}

static void sfs_readdirplus(fuse_req_t req, fuse_ino_t ino, size_t size,
                            off_t offset, fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), ReaddirplusUseAttribution, ino);
  // operation logging is done in readdir to reduce code duplication
  do_readdir(req, ino, size, offset, fi, 1);
}

static void sfs_releasedir(fuse_req_t req, fuse_ino_t ino, fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), ReleasedirUseAttribution, ino);
  auto d = get_dir_handle(fi);
  delete d;
  fuse_reply_err(req, 0);
}

static void sfs_create(fuse_req_t req, fuse_ino_t parent, const char* name,
                       mode_t mode, fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), CreateUseAttribution, parent);
  Inode& inode_p = get_inode(parent);

  auto fd = openat(inode_p.fd, name, (fi->flags | O_CREAT) & ~O_NOFOLLOW, mode);
  // LogPathOperation(req, name, CreateOperation);
  if (fd == -1) {
    auto err = errno;
    if (err == ENFILE || err == EMFILE) {
      LOG(WARNING) << "ERROR: Reached maximum number of file descriptors.";
    }
    fuse_reply_err(req, err);
    return;
  }

  fi->fh = fd;
  fuse_entry_param e;
  auto err = do_lookup(parent, name, &e);
  if (err) {
    if (err == ENFILE || err == EMFILE) {
      LOG(WARNING) << "ERROR: Reached maximum number of file descriptors.";
    }
    fuse_reply_err(req, err);
  } else
    fuse_reply_create(req, &e, fi);
}

static void sfs_fsyncdir(fuse_req_t req, fuse_ino_t ino, int datasync,
                         fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), FsyncdirUseAttribution, ino);
  int res;
  int fd = dirfd(get_dir_handle(fi)->dp);
  if (datasync) {
    res = fdatasync(fd);

  } else {
    res = fsync(fd);
  }
  fuse_reply_err(req, res == -1 ? errno : 0);
}

static void sfs_open(fuse_req_t req, fuse_ino_t ino, fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), OpenUseAttribution, ino);
  Inode& inode = get_inode(ino);

  /* With writeback cache, kernel may send read requests even
     when userspace opened write-only */
  if (fs.timeout && (fi->flags & O_ACCMODE) == O_WRONLY) {
    fi->flags &= ~O_ACCMODE;
    fi->flags |= O_RDWR;
  }

  /* With writeback cache, O_APPEND is handled by the kernel.  This
     breaks atomicity (since the file may change in the underlying
     filesystem, so that the kernel's idea of the end of the file
     isn't accurate anymore). However, no process should modify the
     file in the underlying filesystem once it has been read, so
     this is not a problem. */
  if (fs.timeout && fi->flags & O_APPEND) fi->flags &= ~O_APPEND;

  /* Unfortunately we cannot use inode.fd, because this was opened
     with O_PATH (so it doesn't allow read/write access). */
  char buf[64];
  sprintf(buf, "/proc/self/fd/%i", inode.fd);
  auto fd = open(buf, fi->flags & ~O_NOFOLLOW);
  if (fd == -1) {
    auto err = errno;
    if (err == ENFILE || err == EMFILE)
      DLOG(ERROR) << "Reached maximum number of file descriptors.";
    fuse_reply_err(req, err);
    return;
  }

  fi->keep_cache = (fs.timeout != 0);
  fi->fh = fd;
  fuse_reply_open(req, fi);
}

static void sfs_release(fuse_req_t req, fuse_ino_t ino, fuse_file_info* fi) {
  // AttributionFrame token(GetThreadContext(), ReleaseUseAttribution, ino);
  close(fi->fh);
  fuse_reply_err(req, 0);
}

static void sfs_flush(fuse_req_t req, fuse_ino_t ino, fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), FlushUseAttribution, ino);
  auto res = close(dup(fi->fh));
  fuse_reply_err(req, res == -1 ? errno : 0);
}

static void sfs_fsync(fuse_req_t req, fuse_ino_t ino, int datasync,
                      fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), FsyncUseAttribution, ino);
  int res;
  if (datasync)
    res = fdatasync(fi->fh);
  else
    res = fsync(fi->fh);
  fuse_reply_err(req, res == -1 ? errno : 0);
}

static void do_read(fuse_req_t req, size_t size, off_t off,
                    fuse_file_info* fi) {
  fuse_bufvec buf = FUSE_BUFVEC_INIT(size);
  buf.buf[0].flags =
      static_cast<fuse_buf_flags>(FUSE_BUF_IS_FD | FUSE_BUF_FD_SEEK);
  buf.buf[0].fd = fi->fh;
  buf.buf[0].pos = off;

  fuse_reply_data(req, &buf, get_buffer_copy_flags());
}

static void sfs_read(fuse_req_t req, fuse_ino_t ino, size_t size, off_t off,
                     fuse_file_info* fi) {
  // AttributionFrame token(GetThreadContext(), ReadUseAttribution, ino);
  do_read(req, size, off, fi);
}

static void do_write_buf(fuse_req_t req, size_t size, off_t off,
                         fuse_bufvec* in_buf, fuse_file_info* fi) {
  fuse_bufvec out_buf = FUSE_BUFVEC_INIT(size);
  out_buf.buf[0].flags =
      static_cast<fuse_buf_flags>(FUSE_BUF_IS_FD | FUSE_BUF_FD_SEEK);
  out_buf.buf[0].fd = fi->fh;
  out_buf.buf[0].pos = off;

  auto res = fuse_buf_copy(&out_buf, in_buf, get_buffer_copy_flags());
  if (res < 0)
    fuse_reply_err(req, -res);
  else
    fuse_reply_write(req, (size_t)res);
}

static void sfs_write_buf(fuse_req_t req, fuse_ino_t ino, fuse_bufvec* in_buf,
                          off_t off, fuse_file_info* fi) {
  // AttributionFrame token(GetThreadContext(), WriteBufUseAttribution, ino);
  const auto size = fuse_buf_size(in_buf);
  do_write_buf(req, size, off, in_buf, fi);
}

static void sfs_statfs(fuse_req_t req, fuse_ino_t ino) {
  AttributionFrame token(GetThreadContext(), StatfsUseAttribution, ino);
  struct statvfs stbuf;

  auto res = fstatvfs(get_fs_fd(ino), &stbuf);
  if (res == -1)
    fuse_reply_err(req, errno);
  else
    fuse_reply_statfs(req, &stbuf);
}

#ifdef HAVE_POSIX_FALLOCATE
static void sfs_fallocate(fuse_req_t req, fuse_ino_t ino, int mode,
                          off_t offset, off_t length, fuse_file_info* fi) {
  AttributionFrame token(GetThreadContext(), FallocateUseAttribution, ino);
  if (mode) {
    fuse_reply_err(req, EOPNOTSUPP);
    return;
  }

  auto err = posix_fallocate(fi->fh, offset, length);
  fuse_reply_err(req, err);
}
#endif

static void sfs_flock(fuse_req_t req, fuse_ino_t ino, fuse_file_info* fi,
                      int op) {
  AttributionFrame token(GetThreadContext(), FlockUseAttribution, ino);
  auto res = flock(fi->fh, op);
  fuse_reply_err(req, res == -1 ? errno : 0);
}

#ifdef HAVE_SETXATTR
static void sfs_getxattr(fuse_req_t req, fuse_ino_t ino, const char* name,
                         size_t size) {
  AttributionFrame token(GetThreadContext(), GetxattrUseAttribution, ino);
  char* value = nullptr;
  Inode& inode = get_inode(ino);
  ssize_t ret;
  int saverr;

  char procname[64];
  sprintf(procname, "/proc/self/fd/%i", inode.fd);

  if (size) {
    value = new (nothrow) char[size];
    if (value == nullptr) {
      saverr = ENOMEM;
      goto out;
    }

    ret = getxattr(procname, name, value, size);
    if (ret == -1) goto out_err;
    saverr = 0;
    if (ret == 0) goto out;

    fuse_reply_buf(req, value, ret);
  } else {
    ret = getxattr(procname, name, nullptr, 0);
    if (ret == -1) goto out_err;

    fuse_reply_xattr(req, ret);
  }
out_free:
  delete[] value;
  return;

out_err:
  saverr = errno;
out:
  fuse_reply_err(req, saverr);
  goto out_free;
}

static void sfs_listxattr(fuse_req_t req, fuse_ino_t ino, size_t size) {
  AttributionFrame token(GetThreadContext(), ListxattrUseAttribution, ino);
  char* value = nullptr;
  Inode& inode = get_inode(ino);
  ssize_t ret;
  int saverr;

  char procname[64];
  sprintf(procname, "/proc/self/fd/%i", inode.fd);

  if (size) {
    value = new (nothrow) char[size];
    if (value == nullptr) {
      saverr = ENOMEM;
      goto out;
    }

    ret = listxattr(procname, value, size);
    if (ret == -1) goto out_err;
    saverr = 0;
    if (ret == 0) goto out;

    fuse_reply_buf(req, value, ret);
  } else {
    ret = listxattr(procname, nullptr, 0);
    if (ret == -1) goto out_err;

    fuse_reply_xattr(req, ret);
  }
out_free:
  delete[] value;
  return;
out_err:
  saverr = errno;
out:
  fuse_reply_err(req, saverr);
  goto out_free;
}

static void sfs_setxattr(fuse_req_t req, fuse_ino_t ino, const char* name,
                         const char* value, size_t size, int flags) {
  AttributionFrame token(GetThreadContext(), SetxattrUseAttribution, ino);
  Inode& inode = get_inode(ino);
  ssize_t ret;
  int saverr;

  char procname[64];
  sprintf(procname, "/proc/self/fd/%i", inode.fd);

  ret = setxattr(procname, name, value, size, flags);
  saverr = ret == -1 ? errno : 0;

  fuse_reply_err(req, saverr);
}

static void sfs_removexattr(fuse_req_t req, fuse_ino_t ino, const char* name) {
  AttributionFrame token(GetThreadContext(), RemovexattrUseAttribution, ino);
  char procname[64];
  Inode& inode = get_inode(ino);
  ssize_t ret;
  int saverr;

  sprintf(procname, "/proc/self/fd/%i", inode.fd);
  ret = removexattr(procname, name);
  saverr = ret == -1 ? errno : 0;

  fuse_reply_err(req, saverr);
}
#endif

static void assign_operations(fuse_lowlevel_ops& sfs_oper) {
  sfs_oper.init = sfs_init;
  sfs_oper.destroy = sfs_destroy;
  sfs_oper.lookup = sfs_lookup;
  sfs_oper.mkdir = sfs_mkdir;
  sfs_oper.mknod = sfs_mknod;
  sfs_oper.symlink = sfs_symlink;
  sfs_oper.link = sfs_link;
  sfs_oper.unlink = sfs_unlink;
  sfs_oper.rmdir = sfs_rmdir;
  sfs_oper.rename = sfs_rename;
  sfs_oper.forget = sfs_forget;
  sfs_oper.forget_multi = sfs_forget_multi;
  sfs_oper.getattr = sfs_getattr;
  sfs_oper.setattr = sfs_setattr;
  sfs_oper.readlink = sfs_readlink;
  sfs_oper.opendir = sfs_opendir;
  sfs_oper.readdir = sfs_readdir;
  sfs_oper.readdirplus = sfs_readdirplus;
  sfs_oper.releasedir = sfs_releasedir;
  sfs_oper.fsyncdir = sfs_fsyncdir;
  sfs_oper.create = sfs_create;
  sfs_oper.open = sfs_open;
  sfs_oper.release = sfs_release;
  sfs_oper.flush = sfs_flush;
  sfs_oper.fsync = sfs_fsync;
  sfs_oper.read = sfs_read;
  sfs_oper.write_buf = sfs_write_buf;
  sfs_oper.statfs = sfs_statfs;
#ifdef HAVE_POSIX_FALLOCATE
  sfs_oper.fallocate = sfs_fallocate;
#endif
  sfs_oper.flock = sfs_flock;
#ifdef HAVE_SETXATTR
  sfs_oper.setxattr = sfs_setxattr;
  sfs_oper.getxattr = sfs_getxattr;
  sfs_oper.listxattr = sfs_listxattr;
  sfs_oper.removexattr = sfs_removexattr;
#endif
}

static void maximize_fd_limit() {
  // TODO: Warn if the limit is too below a threshold.
  struct rlimit lim {};
  auto res = getrlimit(RLIMIT_NOFILE, &lim);
  if (res != 0) {
    LOG(WARNING) << "getrlimit() failed with";
    return;
  }
  lim.rlim_cur = lim.rlim_max;
  res = setrlimit(RLIMIT_NOFILE, &lim);
  if (res != 0) {
    LOG(WARNING) << "setrlimit() failed with";
  }
}

static void quiesce() { GetRootContext()->WriteLogs(); }

void MaybeWritePidFile() {
  if (!FLAGS_pid_file.empty()) {
    int pid_file_fd = open(FLAGS_pid_file.c_str(), O_WRONLY | O_CREAT | O_TRUNC,
                           S_IRUSR | S_IWUSR | S_IRGRP | S_IROTH);
    std::string content = absl::StrFormat("%u\n", getpid());
    PCHECK(TryWrite(pid_file_fd, content.data(), content.size()) == 0);
    PCHECK(TryFsync(pid_file_fd, 5));
    PCHECK(close(pid_file_fd) == 0);
  }
}

static void catch_signal(int signo) {
  LOG(INFO) << "Caught signal " << signo << ": " << strsignal(signo);
  if (signo == SIGHUP) {
    quiesce();
    return;
  }

  LOG(WARNING) << "Unhandled signal " << signo << "!";
}

int main(int argc, char* argv[]) {
  google::SetCommandLineOption("GLOG_stderrthreshold", "1");
  google::SetCommandLineOption("GLOG_alsologtostderr", "true");
  google::SetCommandLineOption("GLOG_colorlogtostderr", "true");
  gflags::ParseCommandLineFlags(&argc, &argv, true);
  google::InitGoogleLogging(argv[0]);

  // TODO: Move these to flag validators
  // if (FLAGS_access_log_directory.empty()) {
  //   LOG(WARNING) << "No access log file specified!";
  //   exit(1);
  // }

  if (FLAGS_source_directory.empty()) {
    LOG(WARNING) << "No source directory specified!";
    exit(1);
  }
  
  if (FLAGS_target_directory.empty()) {
    LOG(WARNING) << "No target directory specified!";
    exit(1);
  }

  // Install a signal handler to write logs
  if (signal(SIGHUP, catch_signal) == SIG_ERR) {
    LOG(ERROR) << "Failed to install signal handler";
  }

  LOG(INFO) << "Projecting '" << FLAGS_source_directory << "' -> '"
            << FLAGS_target_directory << "'";

  // We need an fd for every dentry in our the filesystem that the
  // kernel knows about. This is way more than most processes need,
  // so try to get rid of any resource softlimit.
  maximize_fd_limit();

  // Initialize filesystem root
  fs.root.fd = -1;
  fs.root.nlookup = 9999;
  fs.timeout = FLAGS_cache ? 86400.0 : 0;

  struct stat stat;
  auto ret = lstat(FLAGS_source_directory.c_str(), &stat);
  if (ret == -1) {
    LOG(FATAL) << "failed to stat source '" << FLAGS_source_directory << "'";
  }
  if (!S_ISDIR(stat.st_mode)) {
    LOG(FATAL) << "source is not a directory";
  }
  fs.src_dev = stat.st_dev;

  fs.root.fd = open(FLAGS_source_directory.c_str(), O_PATH);
  if (fs.root.fd == -1) {
    LOG(FATAL) << "open(" << FLAGS_source_directory
               << ", O_PATH) failed: " << strerror(errno);
  }

  GetFsContext()->PopulateMonikerTable(FLAGS_source_directory,
                                       FLAGS_record_file_access);

  // Initialize fuse
  fuse_args args = FUSE_ARGS_INIT(0, nullptr);
  if (fuse_opt_add_arg(&args, argv[0]) || fuse_opt_add_arg(&args, "-o") ||
      fuse_opt_add_arg(&args, "default_permissions,fsname=hpps") ||
      (FLAGS_debug && fuse_opt_add_arg(&args, "-odebug"))) {
    LOG(FATAL) << "Out of memory";
  }

  fuse_lowlevel_ops sfs_oper{};
  assign_operations(sfs_oper);
  auto se = fuse_session_new(&args, &sfs_oper, sizeof(sfs_oper), &fs);
  if (se == nullptr) goto err_out1;

  if (fuse_set_signal_handlers(se) != 0) goto err_out2;

  // Don't apply umask, use modes exactly as specified
  umask(0);

  // Mount and run main loop
  struct fuse_loop_config loop_config;
  loop_config.clone_fd = 0;
  loop_config.max_idle_threads = 10;

  if (fuse_session_mount(se, FLAGS_target_directory.c_str()) != 0)
    goto err_out3;

  MaybeWritePidFile();

  if (!FLAGS_multithreaded) {
    ret = fuse_session_loop(se);
  } else {
    ret = fuse_session_loop_mt(se, &loop_config);
  }

  fuse_session_unmount(se);

err_out3:
  fuse_remove_signal_handlers(se);
err_out2:
  fuse_session_destroy(se);
err_out1:
  fuse_opt_free_args(&args);

  return ret ? 1 : 0;
}

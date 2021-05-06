#include "test_util.h"

#include <fts.h>
#include <gtest/gtest.h>

#include <mutex>

#include "glog/logging.h"

namespace test_util {

int RecursiveDelete(const std::string& dir) {
  bool ret = true;
  FTS* ftsp = NULL;
  FTSENT* curr;

  char* paths[] = {const_cast<char*>(dir.c_str()), nullptr};

  ftsp = fts_open(paths, FTS_NOCHDIR | FTS_PHYSICAL | FTS_XDEV, nullptr);
  if (!ftsp) {
    LOG(ERROR) << "fts_open error for path '" << dir << ": " << strerror(errno);
    ret = false;
    goto finish;
  }

  while ((curr = fts_read(ftsp))) {
    switch (curr->fts_info) {
      case FTS_NS:
      case FTS_DNR:
      case FTS_ERR:
        LOG(FATAL) << "fts_read error for path '" << curr->fts_accpath
                   << "': " << strerror(curr->fts_errno);
        break;

      case FTS_DC:
      case FTS_DOT:
      case FTS_NSOK:
        // Not reached unless FTS_LOGICAL, FTS_SEEDOT, or FTS_NOSTAT were
        // passed to fts_open()
        break;

      case FTS_D:
        // Do nothing. Need depth-first search, so directories are deleted
        // in FTS_DP
        break;

      case FTS_DP:
      case FTS_F:
      case FTS_SL:
      case FTS_SLNONE:
      case FTS_DEFAULT:
        if (remove(curr->fts_accpath) < 0) {
          LOG(ERROR) << "Could not remove '" << curr->fts_accpath
                     << "': " << strerror(curr->fts_errno);
          ret = false;
        }
    }
  }

finish:
  if (ftsp) {
    fts_close(ftsp);
  }

  return ret;
}

Dir::Dir(const std::string& path, DIR* dir) : path_(path), dir_(dir) {}

Dir::~Dir() {
  if (dir_ != nullptr) {
    closedir(dir_);
  }
}

File Dir::CreateFile(const std::string& name) {
  auto path = path_ + "/" + name;
  int fd = open(path.c_str(), O_CREAT | O_RDWR);
  PCHECK(fd > 0);
  FILE* f = fdopen(fd, "rw");
  PCHECK(f != nullptr);
  return File{path, f};
}

const std::string kPatternSuffix = ".XXXXXX";
const std::string_view kPatternSuffixView = {kPatternSuffix};

TempDir::TempDir(const std::string& prefix,
                 const bool schedule_recursive_removal)
    : Dir("", nullptr),
      schedule_recursive_removal_(schedule_recursive_removal) {
  char* bazel_test_dir = getenv("TEST_TMPDIR");
  // Use Bazel's preferred test dir, otherwise use the system-level temporary
  // directory
  auto root =
      (bazel_test_dir) ? std::string{bazel_test_dir} : ::testing::TempDir();
  auto pattern = root + "/" + prefix + kPatternSuffix;
  char pattern_cstr[PATH_MAX];
  PCHECK(strncpy(pattern_cstr, pattern.c_str(), PATH_MAX - 1) == pattern_cstr);
  const char* dest = mkdtemp(pattern_cstr);
  PCHECK(dest != nullptr);
  path_ = std::string{dest};
  dir_ = opendir(path_.c_str());
  PCHECK(dir_ != nullptr);
}

TempDir::~TempDir() {
  if (schedule_recursive_removal_) {
    RecursiveDelete(path_);
  }
}

Dir Dir::CreateSubdir(const std::string& name) {
  CHECK(!path_.empty());
  const auto subdir_name = std::string{path_ + "/" + name};
  PCHECK(mkdir(subdir_name.c_str(), S_IRWXU) == 0);
  DIR* dir = opendir(subdir_name.c_str());
  PCHECK(dir != nullptr);

  return Dir{subdir_name, dir};
}

std::string Dir::Path() const { return path_; }

File::File(const std::string& path, FILE* const file)
    : path_(path), file_(file) {
  CHECK_NE(file_, nullptr);
}

File::~File() {
  if (file_) {
    PCHECK(fclose(file_) == 0);
  }
}

std::string File::Path() const { return path_; }

std::FILE* File::Get() { return file_; }

void InitLogging() {
  static std::once_flag logging_init;

  std::call_once(logging_init, []() {
    google::InitGoogleLogging(
        ::testing::UnitTest::GetInstance()->current_test_info()->name());
  });
}

}  // namespace test_util

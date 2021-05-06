#include "tracker.h"

#include <sys/stat.h>

#include "glog/logging.h"

namespace eulefs {
namespace {

static int FtsNameCompare(const FTSENT** left, const FTSENT** right) {
  return (strcmp((*left)->fts_name, (*right)->fts_name));
}

}  // namespace

size_t AddFilesystemContentToMonikerTable(const std::string& path,
                                          MonikerTable& table,
                                          const bool include_files) {
  size_t addition_count = 0;

  char* const paths[] = {const_cast<char*>(path.c_str()), nullptr};

  FTS* ftsp = nullptr;
  FTSENT* entry = nullptr;
  FTSENT* parent = nullptr;

  ftsp = fts_open(paths, FTS_NOCHDIR | FTS_PHYSICAL | FTS_XDEV,
                  &FtsNameCompare);  // TODO: vs FTS_COMFOLLOW?

  auto pathbuf = std::string{};
  pathbuf.reserve(PATH_MAX);

  if (ftsp) {
    while ((parent = fts_read(ftsp))) {
      entry = fts_children(ftsp, 0);
      while (entry) {
        bool insert;

        switch (entry->fts_info) {
          case FTS_NS:
          case FTS_DNR:
          case FTS_ERR:
            LOG(FATAL) << "fts_read error for path '" << entry->fts_accpath
                       << "': " << strerror(entry->fts_errno);
            break;

          case FTS_DC:
          case FTS_DOT:
          case FTS_NSOK:
            // Don't apply because of args.
            break;

          case FTS_D:
            insert = true;
            break;
          case FTS_DP:
            break;

          case FTS_F:
          case FTS_SL:
          case FTS_SLNONE:
          case FTS_DEFAULT:
            insert = include_files;
        }

        if (insert) {
          pathbuf.clear();
          pathbuf += entry->fts_path;
          pathbuf += entry->fts_name;
          table.Insert(entry->fts_statp->st_ino, pathbuf.substr(path.size()));
          addition_count += 1;
        }

        entry = entry->fts_link;
      }
    }
    fts_close(ftsp);
  }

  return addition_count;
}

}  // namespace eulefs

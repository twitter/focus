#include <gflags/gflags.h>
#include <glog/logging.h>
#include <sys/stat.h>

#include <string>

#include "tracker.h"

DEFINE_string(source_directory, "", "Source directory");

using namespace eulefs;

int compare_names(const FTSENT** left, const FTSENT** right) {
  return (strcmp((*left)->fts_name, (*right)->fts_name));
}

int main(int argc, char* argv[]) {
  google::SetCommandLineOption("GLOG_stderrthreshold", "1");
  google::SetCommandLineOption("GLOG_alsologtostderr", "true");
  google::SetCommandLineOption("GLOG_colorlogtostderr", "true");
  gflags::ParseCommandLineFlags(&argc, &argv, true);
  google::InitGoogleLogging(argv[0]);

  if (FLAGS_source_directory.empty()) {
    LOG(ERROR) << "Source directory is required.";
    return 128;
  }

  struct stat sb;
  PCHECK(stat(FLAGS_source_directory.c_str(), &sb) == 0);
  auto monikers = MonikerTable{sb.st_ino};
  auto added = AddFilesystemContentToMonikerTable(FLAGS_source_directory,
                                                  monikers, true);
  LOG(INFO) << "Added " << added << " entries";

  // auto tracker = Tracker{FLAGS_source_directory};
  // LOG(INFO) << "Beginning scan of " << FLAGS_source_directory;
  // tracker.Scan();
  // LOG(INFO) << "Finished scan of " << FLAGS_source_directory;

  return 0;
}

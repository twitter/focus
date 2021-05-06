#include <dirent.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#include <string>

namespace test_util {

void InitLogging();

struct File;

struct Dir {
  Dir(const std::string& path, DIR* dir);
  ~Dir();

  std::string Path() const;

  Dir CreateSubdir(const std::string& name);
  File CreateFile(const std::string& name);

 protected:
  std::string path_;
  DIR* dir_;
};

struct TempDir : public Dir {
  TempDir(const std::string& prefix, const bool schedule_recursive_removal);
  ~TempDir();

 protected:
  bool schedule_recursive_removal_;
};

struct File {
  ~File();

  std::FILE* Get();
  std::string Path() const;

 protected:
  File(const std::string& path, FILE* const file);

 private:
  friend class Dir;
  std::string path_;
  std::FILE* const file_;
};

void InitLogging();

}  // namespace test_util

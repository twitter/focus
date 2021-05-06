#include "test_util.h"

#include <glog/logging.h>
#include <gtest/gtest.h>

#include <mutex>
#include <string>

using namespace test_util;

class TestUtilTest : public ::testing::Test {
  void SetUp() override { InitLogging(); }
};

TEST_F(TestUtilTest, Smoke) {
  std::string path;

  {
    auto dir = TempDir{"TestUtilTest", true};

    PCHECK(mkdir(path.c_str(), S_IXUSR) != 0);

    path = dir.Path();
    auto a = dir.CreateSubdir("a");
    auto a_1 = a.CreateSubdir("1");
    auto a_1_1 = a_1.CreateSubdir("1");

    auto foo = a_1_1.CreateFile("foo");
    struct stat sb;
  }

  PCHECK(mkdir(path.c_str(), S_IXUSR) == 0);
  PCHECK(rmdir(path.c_str()) == 0);
}

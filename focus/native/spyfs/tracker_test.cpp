#include "tracker.h"

#include <glog/logging.h>
#include <gtest/gtest.h>

#include <string>

#include "moniker.h"
#include "test_util.h"

using namespace eulefs;
using namespace test_util;

class TrackerTest : public ::testing::Test {
  void SetUp() override { InitLogging(); }
};

TEST_F(TrackerTest, Smoke) {
  struct stat sb;

  auto dir = TempDir{"TrackerTest", true};
  auto foo = dir.CreateSubdir("foo");
  auto foo_bar = foo.CreateSubdir("bar");
  auto foo_1 = foo.CreateFile("1");
  auto foo_bar_2 = foo_bar.CreateFile("2");

  PCHECK(stat(dir.Path().c_str(), &sb) == 0);
  auto table = MonikerTable{sb.st_ino};

  ASSERT_EQ(AddFilesystemContentToMonikerTable(dir.Path(), table, true), 4);
  {
    PCHECK(stat(foo_1.Path().c_str(), &sb) == 0);
    auto r = table.Get(sb.st_ino, 1);
    ASSERT_TRUE(r);
    ASSERT_EQ(*r, "foo/1");
  }
  {
    PCHECK(stat(foo_bar_2.Path().c_str(), &sb) == 0);
    auto r = table.Get(sb.st_ino, 1);
    ASSERT_TRUE(r);
    ASSERT_EQ(*r, "foo/bar/2");
  }

  table.Clear();
  ASSERT_EQ(AddFilesystemContentToMonikerTable(dir.Path(), table, false),
            2);  // Directories only
  {
    PCHECK(stat(foo_1.Path().c_str(), &sb) == 0);
    auto r = table.Get(sb.st_ino, 1);
    ASSERT_FALSE(r);
  }
}

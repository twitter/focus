#include "moniker.h"

#include <glog/logging.h>
#include <gtest/gtest.h>

#include <mutex>

#include "test_util.h"

using namespace eulefs;
using namespace test_util;

class MonikerTest : public ::testing::Test {
  void SetUp() override { InitLogging(); }
};

TEST_F(MonikerTest, TokenTable) {
  TokenTable tab;
  {
    // Original insert
    auto r = tab.GetOrInsert("foo");
    ASSERT_EQ(r.second, true);
    ASSERT_EQ(r.first, 0);
  }

  {
    // Duplicate inseret
    auto r = tab.GetOrInsert("foo");
    ASSERT_EQ(r.second, false);
    ASSERT_EQ(r.first, 0);
  }

  {
    // Original insert
    auto r = tab.GetOrInsert("bar");
    ASSERT_EQ(r.second, true);
    ASSERT_EQ(r.first, 1);
  }

  {
    // Duplicate insert
    auto r = tab.GetOrInsert("bar");
    ASSERT_EQ(r.second, false);
    ASSERT_EQ(r.first, 1);
  }

  {
    // Original insert
    auto r = tab.GetOrInsert("baz");
    ASSERT_EQ(r.second, true);
    ASSERT_EQ(r.first, 2);
  }

  {
    // Reverse lookup of an existing value.
    auto r = tab.ReverseLookup(1);
    ASSERT_TRUE(r);
    ASSERT_EQ(*r, "bar");
  }
  {
    // Reverse lookup of an existing value.
    auto r = tab.ReverseLookup(2);
    ASSERT_TRUE(r);
    ASSERT_EQ(*r, "baz");
  }

  {
    // Reverse lookup of an absent value.
    auto r = tab.ReverseLookup(99);
    ASSERT_FALSE(r);
  }
}

TEST_F(MonikerTest, MonikerTable) {
  MonikerTable m{0};

  ASSERT_TRUE(m.Insert(1, "a"));
  ASSERT_TRUE(m.Insert(2, "a/b0"));
  ASSERT_TRUE(m.Insert(3, "a/b1"));
  ASSERT_TRUE(m.Insert(4, "a/b1/c0"));

  {
    // Fetch a
    auto r = m.Get(1, 0);
    ASSERT_TRUE(r);
    ASSERT_EQ(*r, "a");
  }
  {
    // fetch b0
    auto r = m.Get(2, 0);
    ASSERT_TRUE(r);
    ASSERT_EQ(*r, "a/b0");
  }
  {
    // Fetch b1
    auto r = m.Get(3, 0);
    ASSERT_TRUE(r);
    ASSERT_EQ(*r, "a/b1");
  }
  {
    // Fetch c0
    auto r = m.Get(4, 0);
    ASSERT_TRUE(r);
    ASSERT_EQ(*r, "a/b1/c0");
  }
  {
    auto r = m.Get(99, 0);
    ASSERT_FALSE(r);
  }
}

TEST_F(MonikerTest, MonikerNode) {
  auto root = std::make_unique<MonikerNode>(0, nullptr);
  auto node = root->Get(8)->Get(6)->Get(7)->Get(5)->Get(3)->Get(0)->Get(9);
  auto stack = node->Path();
  ASSERT_EQ(stack.top(), 8);
  stack.pop();
  ASSERT_EQ(stack.top(), 6);
  stack.pop();
  ASSERT_EQ(stack.top(), 7);
  stack.pop();
  ASSERT_EQ(stack.top(), 5);
  stack.pop();
  ASSERT_EQ(stack.top(), 3);
  stack.pop();
  ASSERT_EQ(stack.top(), 0);
  stack.pop();
  ASSERT_EQ(stack.top(), 9);
  stack.pop();
  ASSERT_EQ(stack.empty(), true);
}

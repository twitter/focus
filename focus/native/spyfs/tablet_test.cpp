#include "tablet.h"

#include <glog/logging.h>
#include <gtest/gtest.h>

#include <chrono>
#include <random>
#include <thread>

#include "test_util.h"

using namespace eulefs;
using namespace test_util;

class TabletTest : public ::testing::Test {
  void SetUp() override { InitLogging(); }
};

TEST_F(TabletTest, Smoke) {
  size_t thread_id = GetLogicalThreadId();
  // ASSERT_EQ(thread_id, 0);
  Tablets tablets;
  std::shared_ptr<Tablet> t = tablets.At(thread_id);
  ASSERT_EQ(t->Size(), 0);
  t->Insert(10);
  ASSERT_EQ(t->Size(), 1);

  std::shared_ptr<Tablet> thread_tablet = tablets.GetTabletForThisThread();
  {
    auto lock = absl::ReaderMutexLock{t->Mutex()};
    ASSERT_TRUE(t->Eq(*thread_tablet));
  }
}

TEST_F(TabletTest, Swap) {
  Tablets tablets;
  std::shared_ptr<Tablet> t0 = tablets.At(4);
  std::shared_ptr<Tablet> t1 = tablets.At(5);
  {
    t0->Insert(4);
    t1->Insert(5);
  }
  ASSERT_EQ(t0->Size(), 1);
  ASSERT_EQ(t1->Size(), 1);
}

TEST_F(TabletTest, Sweep) {
  Tablets tablets;

  std::function<void(Tablets&, size_t, size_t)> fn =
      [](Tablets& tablets, size_t begin, size_t count){
        std::shared_ptr<Tablet> t = tablets.GetTabletForThisThread();
        for (size_t i = begin; i < (begin + count); ++i) {
          t->Insert(i);
        }
      };

  std::thread t0(fn, std::ref(tablets), 0, 500);
  std::thread t1(fn, std::ref(tablets), 500, 500);

  t0.join();
  t1.join();

  Tablet aggregated;
  tablets.Sweep(aggregated);
  {
    ASSERT_EQ(aggregated.Size(), 1000);
    auto lock = absl::WriterMutexLock{aggregated.Mutex()};
    auto data = aggregated.Data();
    for (size_t i = 0; i < 1000; ++i) {
      ASSERT_TRUE(data->contains(i)) << "for item " << i; 
    }

  }
}
TEST_F(TabletTest, StressTest) {
  Tablets tablets;

  std::atomic<size_t> remaining;
  auto workers = std::vector<std::thread>();
  std::function<void(Tablets&, size_t, size_t)> worker_fn =
      [&remaining](Tablets& tablets, size_t begin, size_t count){
        std::random_device rd;
        std::mt19937 gen(rd());
        std::poisson_distribution<> distrib(20);
        std::this_thread::sleep_for(std::chrono::microseconds(distrib(gen)));

        std::shared_ptr<Tablet> t = tablets.GetTabletForThisThread();
        for (size_t i = begin; i < (begin + count); ++i) {
          t->Insert(i);
          std::this_thread::sleep_for(std::chrono::microseconds(distrib(gen)));
        }
        remaining.fetch_sub(1, std::memory_order_relaxed);
      };

  size_t n_threads = std::thread::hardware_concurrency() * 8;
  remaining = n_threads;
  const size_t mul = 500;

  std::vector<std::thread> threads;
  for (size_t i = 0; i < n_threads; ++i) {
    threads.push_back(std::thread(std::bind(worker_fn, std::ref(tablets), i * mul, mul)));
  }

  std::random_device rd;  //Will be used to obtain a seed for the random number engine
  std::mt19937 gen(rd()); //Standard mersenne_twister_engine seeded with rd()
  std::uniform_int_distribution<> distrib(0, 2);
  Tablet aggregated;

  size_t aggregations = 0;
  while (remaining.load(std::memory_order_relaxed) > 0) {
    tablets.Sweep(aggregated);
    ++aggregations;
    std::this_thread::sleep_for(std::chrono::microseconds(distrib(gen)));
  }

  for (auto& thread : threads) {
    thread.join(); // We must join because otherwise the thread dtor will flip out
  }

  {
    ASSERT_EQ(aggregated.Size(), n_threads * mul);
    auto lock = absl::WriterMutexLock{aggregated.Mutex()};
    auto data = aggregated.Data();
    for (size_t i = 0; i < n_threads * 100; ++i) {
      ASSERT_TRUE(data->contains(i)) << "for item " << i;
    }

  }
}

// TODO: Some benchmarks would be nice

#include <glog/logging.h>

#include <atomic>
#include <vector>

#include <cstdint>
#include <pthread.h>

#include "absl/container/flat_hash_set.h"
#include "absl/synchronization/mutex.h"

#include "annotations.h"

// TODO: Working through eliminating some shared pointers would be nice, at the risk of the mutexes living for too long.

// TODO: It's probably possible to eliminate some locks.

namespace eulefs {

size_t GetLogicalThreadId();

class Tablets;

class Tablet {
public:
  using StorageType = absl::flat_hash_set<uint64_t>;
  using StoragePointerType = std::shared_ptr<StorageType>;

  Tablet();
  Tablet(Tablet&& other);
  DISALLOW_COPY_AND_ASSIGN(Tablet);

  bool Insert(const uint64_t datum) LOCKS_EXCLUDED(mu_);
  void Swap(Tablet& other) EXCLUSIVE_LOCKS_REQUIRED(mu_);
  size_t Size() LOCKS_EXCLUDED(mu_);
  bool Eq(Tablet& other) SHARED_LOCKS_REQUIRED(mu_);
  StoragePointerType Data() EXCLUSIVE_LOCKS_REQUIRED(mu_);

  inline absl::Mutex* Mutex() {
    return &mu_;
  }

protected:
  friend class Tablets;

 private:
  StoragePointerType data_ GUARDED_BY(mu_);
  absl::Mutex mu_;
};

// TODO: Consider generalizing this
class Tablets {
  using StorageType = std::shared_ptr<Tablet>;
  using ReturnType = std::shared_ptr<Tablet>;

  static_assert(std::is_default_constructible<StorageType>::value);
public:
  Tablets() = default;
  DISALLOW_COPY_AND_ASSIGN(Tablets);
  DISALLOW_MOVE(Tablets);

  Tablets::ReturnType At(const size_t index) LOCKS_EXCLUDED(mu_);
  Tablets::ReturnType GetTabletForThisThread() LOCKS_EXCLUDED(mu_);
  void Sweep(Tablet& into) LOCKS_EXCLUDED(sweep_mu_);
  absl::Mutex* Mutex();

private:
  std::vector<StorageType> tablets_;
  absl::Mutex mu_;
  absl::Mutex sweep_mu_;
};

} // namespace eulefs

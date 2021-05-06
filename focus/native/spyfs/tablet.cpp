#include "tablet.h"

namespace eulefs {

size_t GetLogicalThreadId() {
  static std::atomic<size_t> global_thread_count = 0;
  thread_local uint64_t thread_id = SIZE_MAX;
  if (thread_id == SIZE_MAX) {
    thread_id = global_thread_count.fetch_add(1, std::memory_order_relaxed);
    CHECK(thread_id < SIZE_MAX);
  }
  return thread_id;
}

Tablet::Tablet() : data_(std::make_shared<StorageType>()) {}

Tablet::Tablet(Tablet&& other) : data_(std::move(other.data_)) {}

bool Tablet::Insert(const uint64_t datum) {
  auto writer = absl::WriterMutexLock{&mu_ /* TODO */};
  const auto result = data_->emplace(datum);
  const bool inserted = std::get<1>(result);
  return inserted;
}

void Tablet::Swap(Tablet& other) {
  Mutex()->AssertHeld();
  other.Mutex()->AssertHeld();
  return data_.swap(other.data_);
}

size_t Tablet::Size() {
  auto mutex = Mutex();
  auto reader = absl::ReaderMutexLock{mutex /* TODO */};
  return data_->size();
}

bool Tablet::Eq(Tablet& other) {
  Mutex()->AssertReaderHeld();
  other.Mutex()->AssertReaderHeld();
  return data_ == other.data_;
}


Tablet::StoragePointerType Tablet::Data() {
  return data_;
}

Tablets::ReturnType
Tablets::At(const size_t index) {
  // Return an allocated slot
  {
    auto reader = absl::ReaderMutexLock{Mutex()};
    if (tablets_.size() > index + 1) {
      return tablets_.at(index);
    }
  }

  // Possibly expand the number of slots and return an item
  {
    auto writer = absl::WriterMutexLock{Mutex()};
    for (size_t i = index + 1 - tablets_.size(); i > 0; --i) {
      tablets_.emplace_back(std::make_unique<Tablet>());
    }
    return tablets_.at(index);
  }
}

Tablets::ReturnType
Tablets::GetTabletForThisThread() {
  return At(GetLogicalThreadId());
}

inline absl::Mutex* Tablets::Mutex() {
  return &mu_;
}

void Tablets::Sweep(Tablet& into) {
  auto lock = absl::MutexLock{&sweep_mu_}; // Prevent concurrent sweep attempts

  // Try to avoid allocating while holding a lock by reading how many tablets there are and resizing
  size_t count;
  std::vector<Tablet> swap_tablets;
  {
    auto reader = absl::ReaderMutexLock{Mutex()};
    count = tablets_.size();
  }
  swap_tablets.resize(count);

  // Swap our freshly allocated tablets into the live tablets
  {
    auto reader = absl::ReaderMutexLock{Mutex()}; // Ensure no new tablets are created
    swap_tablets.resize(tablets_.size()); // Right-size out tablets in case the size changed. We did our best.
    for (size_t i = 0; i < tablets_.size(); ++i) {
      auto& tab = tablets_.at(i);
      auto swap_mutex = swap_tablets[i].Mutex();
      auto tab_mutex = tab->Mutex();
      auto swap_writer = absl::WriterMutexLock{swap_mutex /* TODO */};
      auto tab_writer = absl::WriterMutexLock{tab_mutex /* TODO */};
      tab->Swap(swap_tablets[i]);
    }
  }

  // Merge swapped tablets
  for (auto& swap_tablet : swap_tablets) {
    into.data_->merge(*swap_tablet.data_); // We don't need mutexes.
    swap_tablet.data_.reset(); // Deallocate during merge
  }
}

} // namespace eulefs

#pragma once

#include "absl/synchronization/mutex.h"
#include "annotations.h"

namespace eulefs {

class ReaderMutexLock {
 public:
  ReaderMutexLock(absl::Mutex* mu);
  ~ReaderMutexLock();
  DISALLOW_COPY_AND_ASSIGN(ReaderMutexLock);
  DISALLOW_MOVE(ReaderMutexLock);

 private:
  absl::Mutex* mu_;
};

}  // namespace eulefs

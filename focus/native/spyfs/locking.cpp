#include "locking.h"

namespace eulefs {

ReaderMutexLock::ReaderMutexLock(absl::Mutex* mu) : mu_(mu) {
  mu_->ReaderLock();
}

ReaderMutexLock::~ReaderMutexLock() { mu_->ReaderUnlock(); }

}  // namespace eulefs
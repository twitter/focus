#pragma once

#pragma once

#include <fts.h>

#include "absl/base/thread_annotations.h"
#include "absl/strings/string_view.h"
#include "moniker.h"

namespace eulefs {

size_t AddFilesystemContentToMonikerTable(const std::string& path,
                                          MonikerTable& table,
                                          const bool include_files);

}  // namespace eulefs

#pragma once

#include <glog/logging.h>

#include <optional>
#include <stack>
#include <unordered_map>

#include "absl/base/thread_annotations.h"
#include "absl/container/flat_hash_map.h"
#include "absl/container/node_hash_map.h"
#include "absl/container/node_hash_set.h"
#include "absl/strings/str_format.h"
#include "absl/synchronization/mutex.h"
#include "annotations.h"

namespace eulefs {

constexpr uint64_t kFuseRootNodeId =
    1;  // TODO: Either use fuse_common and use that or make a compile time
        // assertion somewhere. Seems like a waste to take a dependency just for
        // this simple defintion.

class TokenTable;

// The token is a string maintaining its position in an external
//
class Token {
 public:
  Token(const std::string& value, const uint64_t position);

  std::string Value() const;
  uint64_t Position() const;
  bool Eq(const Token& other) const;
  std::string Display() const;

  template <typename H>
  friend H AbslHashValue(H h, const Token& t) {
    return H::combine(std::move(h), t.Value());
  }

 private:
  const std::string value_;
  const uint64_t position_;
};

inline bool operator==(const Token& lhs, const Token& rhs) {
  return lhs.Eq(rhs);
}

class TokenTable {
 public:
  TokenTable();
  DISALLOW_COPY_AND_ASSIGN(TokenTable);
  DISALLOW_MOVE(TokenTable);

  std::pair<uint64_t, bool> GetOrInsert(const std::string& token);
  std::optional<std::string> ReverseLookup(uint64_t id);
  std::string Display() const;

 protected:
  friend class Token;
  uint64_t NextId();
  uint64_t InsertName(const std::string& token);

 private:
  absl::Mutex mu_;
  uint64_t id_;
  absl::node_hash_set<Token>
      forward_;  // N.B. we use node set for pointer stability
  std::vector<const Token*> reverse_;
};

class MonikerNode {
 public:
  MonikerNode() = delete;
  MonikerNode(const uint64_t name, MonikerNode* const parent);
  DISALLOW_COPY_AND_ASSIGN(MonikerNode);
  DISALLOW_MOVE(MonikerNode);

  std::stack<uint64_t> Path();
  uint64_t Name() const;
  MonikerNode* Get(const uint64_t word);
  void Clear();

 private:
  absl::Mutex mu_;
  uint64_t name_;
  absl::flat_hash_map<uint64_t, std::unique_ptr<MonikerNode>> children_;
  MonikerNode* const parent_;
};

class MonikerTable {
  // using MapType = absl::flat_hash_map<uint64_t, std::vector<uint64_t>>;

 public:
  MonikerTable(const uint64_t root_node_id);
  DISALLOW_COPY_AND_ASSIGN(MonikerTable);
  DISALLOW_MOVE(MonikerTable);

  // bool Insert(const uint64_t parent_id, const uint64_t id,
  //             const std::string& name);
  bool Insert(const uint64_t id, const std::string& path);

  bool Remove(const uint64_t id);

  std::optional<std::string> Get(const uint64_t id, const size_t offset,
                                 const bool fully_qualified = true);
  size_t Size();
  void Clear();

 protected:
  std::vector<uint64_t> Tokenize(const std::string& path);
  bool Parents(uint64_t id, std::stack<uint64_t>& to);

 private:
  // Root of the trie of path names
  MonikerNode root_;

  // Index from inode to the last component of the path
  absl::flat_hash_map<uint64_t, MonikerNode*> id_to_terminal_ GUARDED_BY(mu_);
  absl::Mutex mu_;

  TokenTable tokens_;
  uint64_t root_node_id_;
};

}  // namespace eulefs

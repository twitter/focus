#include "moniker.h"

#include <glog/logging.h>

#include <limits>
#include <stack>

#include "absl/strings/str_format.h"
#include "locking.h"

namespace eulefs {

Token::Token(const std::string& value, const uint64_t position)
    : value_(value), position_(position) {}

std::string Token::Value() const { return value_; }

uint64_t Token::Position() const { return position_; }

std::string Token::Display() const {
  return absl::StrFormat("Token(position=%d, value=%s)", position_, value_);
}

bool Token::Eq(const Token& other) const { return value_ == other.value_; }

TokenTable::TokenTable() : id_(0) {}

std::pair<uint64_t, bool> TokenTable::GetOrInsert(const std::string& token) {
  auto lock = absl::MutexLock{&mu_};
  uint64_t id = id_++;

  auto result = forward_.insert(Token{token, id});
  const bool inserted = result.second;
  if (inserted) {
    // Add to the reverse index
    reverse_.emplace_back(&*result.first);
  } else {
    id = result.first->Position();
    // Reclaim the ID since it was not used
    --id_;
  }

  return std::make_pair(id, inserted);
}

std::optional<std::string> TokenTable::ReverseLookup(uint64_t id) {
  auto lock = ReaderMutexLock{&mu_};
  if (reverse_.size() < id) {
    return std::nullopt;
  }

  return reverse_.at(id)->Value();
}

MonikerTable::MonikerTable(const uint64_t root_node_id)
    : root_(0, nullptr), root_node_id_(root_node_id) {
  Insert(root_node_id, "<root>");
}

const size_t kHexOutputWidth = sizeof(uint64_t) / 2;
const char kPathSeparatorChar = '/';

std::vector<uint64_t> MonikerTable::Tokenize(const std::string& path) {
  mu_.AssertReaderHeld();

  std::vector<uint64_t> result;
  std::string component;
  std::istringstream stream(path);
  while (std::getline(stream, component, kPathSeparatorChar)) {
    uint64_t token_id;
    if (!component.empty()) {
      std::tie(token_id, std::ignore) = tokens_.GetOrInsert(component);
      result.push_back(token_id);
    }
  }

  return result;
}

MonikerNode::MonikerNode(const uint64_t name, MonikerNode* const parent)
    : name_(name), parent_(parent) {}

MonikerNode* MonikerNode::Get(const uint64_t word) {
  {
    auto reader = ReaderMutexLock{&mu_};
    auto result = children_.find(word);

    if (result != children_.end()) {
      return (*result).second.get();
    }
  }

  {
    auto writer = absl::MutexLock{&mu_};
    auto result =
        children_.try_emplace(word, std::make_unique<MonikerNode>(word, this));
    return result.first->second.get();
  }
}

uint64_t MonikerNode::Name() const { return name_; }

std::stack<uint64_t> MonikerNode::Path() {
  std::stack<uint64_t> result;
  MonikerNode* node = this;
  for (;;) {
    auto reader = ReaderMutexLock{&node->mu_};
    if (!node->parent_) {
      // Skip the root node, which has a nonsense name datum.
      break;
    }
    result.push(node->Name());
    node = node->parent_;
  }
  return result;
}

void MonikerNode::Clear() { children_.clear(); }

// TODO: Handle hard links
bool MonikerTable::Insert(const uint64_t id, const std::string& path) {
  bool terminal_inserted;
  auto lock = absl::MutexLock(&mu_);

  auto tokens = Tokenize(path);
  MonikerNode* node = &root_;
  for (const auto token : tokens) {
    node = node->Get(token);
    CHECK(node != nullptr);
  }
  std::tie(std::ignore, terminal_inserted) = id_to_terminal_.emplace(id, node);

  return terminal_inserted;
}

bool MonikerTable::Remove(const uint64_t id) { return false; }

std::optional<std::string> MonikerTable::Get(const uint64_t id,
                                             const size_t offset,
                                             const bool fully_qualified) {
  auto lock = ReaderMutexLock(&mu_);
  std::string result;

  auto p = id_to_terminal_.find(id);
  if (p == id_to_terminal_.end()) {
    return std::nullopt;
  }

  MonikerNode* node = p->second;
  auto stack = node->Path();
  while (!stack.empty()) {
    const auto component = stack.top();
    auto token = tokens_.ReverseLookup(component);
    if (!token) {
      return std::nullopt;
    }
    result += *token;
    stack.pop();
    if (!stack.empty()) {
      result += "/";
    }
  }

  VLOG(3) << "Get " << std::hex << id << " -> '" << result << "'";
  return result;
}

bool MonikerTable::Parents(const uint64_t id, std::stack<uint64_t>& to) {
  // Not implemented.
  return false;
}

size_t MonikerTable::Size() {
  mu_.AssertReaderHeld();
  return id_to_terminal_.size();
}

void MonikerTable::Clear() {
  auto lock = absl::MutexLock(&mu_);
  id_to_terminal_.clear();
  root_.Clear();
}

}  // namespace eulefs

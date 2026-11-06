// Calls the app is answered for later by its own id: baa_profile_clear_data and
// baa_profile_load_unpacked. Each is answered exactly once, even when the profile closes first.

#include <map>
#include <string>
#include <utility>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/task/single_thread_task_runner.h"

namespace baa {
namespace {

using Key = std::pair<RequestKind, uint64_t>;

// The profile each unanswered request runs in.
std::map<Key, Profile*>& Pending() {
  static base::NoDestructor<std::map<Key, Profile*>> pending;
  return *pending;
}

void Deliver(RequestKind kind, uint64_t id, bool ok, const std::string& message) {
  if (kind == RequestKind::kClear) {
    Callbacks().clear_done(CallbackContext(), id);
  } else {
    Callbacks().load_unpacked_done(CallbackContext(), id, ok, message.c_str());
  }
}

}  // namespace

void BeginRequest(RequestKind kind, uint64_t id, Profile* profile) {
  Pending()[{kind, id}] = profile;
}

void FinishRequest(RequestKind kind, uint64_t id, bool ok, const std::string& message) {
  if (Pending().erase({kind, id})) {
    Deliver(kind, id, ok, message);
  }
}

void FinishRequestLater(RequestKind kind, uint64_t id, bool ok, const std::string& message) {
  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
      FROM_HERE, base::BindOnce(&FinishRequest, kind, id, ok, message));
}

void AnswerPendingOf(Profile* profile) {
  for (auto it = Pending().begin(); it != Pending().end();) {
    if (it->second != profile) {
      ++it;
      continue;
    }
    // Not inside baa_profile_close: from a task, after the entry is gone so the late real
    // completion finds nothing.
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&Deliver, it->first.first, it->first.second, false,
                                  std::string("The profile closed")));
    it = Pending().erase(it);
  }
}

}  // namespace baa

// The ids the app answers later (Request: asks and menus, by baa_answer and baa_context_menu_done)
// and the ones it is answered for later (baa_profile_clear_data, baa_profile_load_unpacked; exactly
// once, even when the profile closes first).

#include <map>
#include <string>
#include <utility>
#include <vector>

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

std::map<uint64_t, Request*>& Registry() {
  static base::NoDestructor<std::map<uint64_t, Request*>> registry;
  return *registry;
}

}  // namespace

Request::Request(uint64_t tab, Role role) : id_(NextId()), tab_(tab), role_(role) {
  Registry()[id_] = this;
}

Request::~Request() {
  Unregister();
}

void Request::Unregister() {
  Registry().erase(id_);
}

Request* FindRequest(uint64_t id) {
  auto it = Registry().find(id);
  return it == Registry().end() ? nullptr : it->second;
}

void WithdrawRequest(uint64_t id) {
  Request* request = FindRequest(id);
  if (!request) {
    return;
  }
  uint64_t tab = request->tab();
  request->Unregister();
  if (TabAnnounced(TabFromId(tab))) {
    Callbacks().request_closed(CallbackContext(), tab, id);
  }
}

void DropRequestsOf(uint64_t tab, bool tell, bool menus_only) {
  std::vector<uint64_t> ids;
  for (auto& [id, request] : Registry()) {
    if (request->tab() == tab && (!menus_only || request->role() == Request::Role::kMenu)) {
      ids.push_back(id);
    }
  }
  for (uint64_t id : ids) {
    Request* request = FindRequest(id);
    if (!request) {
      continue;
    }
    bool ask = request->role() != Request::Role::kMenu;
    request->Unregister();
    // Before the app hears of it: nothing the callback re-enters can reach the request then.
    request->Drop();
    if (tell && ask) {
      Callbacks().request_closed(CallbackContext(), tab, id);
    }
  }
}

std::vector<uint64_t> DialogsOf(uint64_t tab) {
  std::vector<uint64_t> ids;
  for (auto& [id, request] : Registry()) {
    if (request->tab() == tab && request->role() == Request::Role::kDialog) {
      ids.push_back(id);
    }
  }
  return ids;
}

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

void baa_answer(uint64_t id, const BaaAnswer* answer) {
  baa::Request* request = baa::FindRequest(id);
  if (answer && request && request->role() != baa::Request::Role::kMenu) {
    request->Answer(*answer);
  }
}

void baa_context_menu_done(uint64_t id, int command) {
  baa::Request* request = baa::FindRequest(id);
  if (request && request->role() == baa::Request::Role::kMenu) {
    request->Command(command);
  }
}

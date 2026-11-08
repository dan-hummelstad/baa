// The tab's DevTools protocol, in process (baa_tab_cdp). PLAN-m4-features.md §0.
// A DevToolsAgentHostClient on the tab's own agent host: no
// --remote-debugging-port, no socket, nothing another process can reach.
// Chrome keeps several clients per host, so the user's DevTools work too.
// Which commands are sent is decided in Rust (crates/baa-web/src/core_host/cdp.rs);
// this side refuses what no command of ours needs: the browser target,
// Chrome's own pages, local files, unsafe operations.

#include <map>
#include <memory>
#include <optional>
#include <set>
#include <string>
#include <string_view>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/containers/span.h"
#include "base/json/json_reader.h"
#include "base/memory/scoped_refptr.h"
#include "base/no_destructor.h"
#include "base/strings/string_number_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "base/values.h"
#include "content/public/browser/devtools_agent_host.h"
#include "content/public/browser/devtools_agent_host_client.h"
#include "content/public/browser/web_contents.h"
#include "url/gurl.h"

namespace baa {
namespace {

void Send(uint64_t tab, uint64_t id, const std::string& json) {
  Callbacks().cdp(CallbackContext(), tab, id, json.c_str());
}

void SendError(uint64_t tab, uint64_t id, std::string_view message) {
  Send(tab, id,
       "{\"id\":" + base::NumberToString(id) +
           ",\"error\":{\"code\":-32000,\"message\":\"" + std::string(message) + "\"}}");
}

class Session;

// tab id -> its session.
std::map<uint64_t, std::unique_ptr<Session>>& Sessions() {
  static base::NoDestructor<std::map<uint64_t, std::unique_ptr<Session>>> sessions;
  return *sessions;
}

class Session : public content::DevToolsAgentHostClient {
 public:
  Session(uint64_t tab, scoped_refptr<content::DevToolsAgentHost> host)
      : tab_(tab), host_(std::move(host)) {}
  ~Session() override = default;

  bool Attach() { return host_->AttachClient(this); }

  void Dispatch(uint64_t id, std::string_view json) {
    waiting_.insert(id);
    host_->DispatchProtocolMessage(this, base::as_byte_span(json));
  }

  // Leaves the host; DetachClient does not call AgentHostClosed.
  void Detach(bool tell) {
    host_->DetachClient(this);
    if (tell) {
      Ended();
    }
  }

  // content::DevToolsAgentHostClient:
  void DispatchProtocolMessage(content::DevToolsAgentHost*,
                               base::span<const uint8_t> message) override {
    std::string json(base::as_string_view(message));
    uint64_t id = 0;
    if (std::optional<base::Value> value = base::JSONReader::Read(json)) {
      if (const base::Value::Dict* dict = value->GetIfDict()) {
        // FindDouble also reads ints; ids may outgrow int.
        if (std::optional<double> n = dict->FindDouble("id")) {
          id = static_cast<uint64_t>(*n);
        }
      }
    }
    if (id != 0 && !waiting_.erase(id)) {
      return;  // not a command of ours
    }
    Send(tab_, id, json);
  }

  // Chrome ended the session: the page went somewhere MayAttachToURL refuses,
  // or the contents are going away.
  void AgentHostClosed(content::DevToolsAgentHost*) override {
    Ended();
    auto it = Sessions().find(tab_);
    if (it != Sessions().end() && it->second.get() == this) {
      // Chrome is inside the host's code: the session goes on a later turn.
      base::SingleThreadTaskRunner::GetCurrentDefault()->DeleteSoon(FROM_HERE,
                                                                    std::move(it->second));
      Sessions().erase(it);
    }
  }

  // An http(s) page, or about:blank (whose site URL may be empty). Checked
  // again by Chrome on every navigation.
  bool MayAttachToURL(const GURL& url, bool is_webui) override {
    return !is_webui && (url.is_empty() || url.SchemeIsHTTPOrHTTPS() || url.IsAboutBlank());
  }
  bool MayAttachToBrowser() override { return false; }
  bool MayReadLocalFiles() override { return false; }
  bool MayWriteLocalFiles() override { return false; }
  bool AllowUnsafeOperations() override { return false; }

 private:
  // What still waits gets an error reply, then the app hears its enabled
  // domains are gone.
  void Ended() {
    std::set<uint64_t> waiting = std::move(waiting_);
    waiting_.clear();
    for (uint64_t id : waiting) {
      SendError(tab_, id, "session ended");
    }
    Send(tab_, 0, R"({"method":"Baa.detached","params":{}})");
  }

  const uint64_t tab_;
  const scoped_refptr<content::DevToolsAgentHost> host_;
  std::set<uint64_t> waiting_;
};

}  // namespace

void DetachCdp(uint64_t tab, bool tell) {
  auto it = Sessions().find(tab);
  if (it == Sessions().end()) {
    return;
  }
  std::unique_ptr<Session> session = std::move(it->second);
  Sessions().erase(it);
  session->Detach(tell);
}

}  // namespace baa

void baa_tab_cdp(BaaTabId tab_id, uint64_t id, const char* json) {
  content::WebContents* wc = baa::ContentsOf(baa::TabFromId(tab_id));
  if (!wc) {
    return;
  }
  auto& sessions = baa::Sessions();
  auto it = sessions.find(tab_id);
  if (it == sessions.end()) {
    auto session = std::make_unique<baa::Session>(
        tab_id, content::DevToolsAgentHost::GetOrCreateFor(wc));
    if (!session->Attach()) {
      baa::SendError(tab_id, id, "page does not allow a session");
      return;
    }
    it = sessions.emplace(tab_id, std::move(session)).first;
  }
  it->second->Dispatch(id, json ? json : "");
}

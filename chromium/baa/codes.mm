// One-time-code fields reported as `code_field`, baa_tab_fill_code, and baa_site_of
// (PLAN-mail.md §7). Helium leaves address and card autofill and the password manager off, so
// BrowserAutofillManager::ShouldParseForms() is false, forms are never classified and Chrome's
// ONE_TIME_CODE type never appears. A field is found from the focus change instead, by its
// autocomplete attribute, read in an isolated world: the page sees no script, no global and no
// event until the fill.

#include <algorithm>
#include <map>
#include <memory>
#include <string>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/memory/weak_ptr.h"
#include "base/no_destructor.h"
#include "base/strings/string_util.h"
#include "base/strings/utf_string_conversions.h"
#include "base/values.h"
#include "chrome/common/chrome_isolated_world_ids.h"
#include "content/public/browser/focused_node_details.h"
#include "content/public/browser/global_routing_id.h"
#include "content/public/browser/navigation_handle.h"
#include "content/public/browser/page.h"
#include "content/public/browser/render_frame_host.h"
#include "content/public/browser/render_widget_host_view.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_observer.h"
#include "net/base/registry_controlled_domains/registry_controlled_domain.h"
#include "ui/gfx/geometry/rect.h"
#include "url/gurl.h"
#include "url/origin.h"

namespace baa {
namespace {

constexpr int32_t kWorld = ISOLATED_WORLD_ID_CHROME_INTERNAL;

// Keeps the focused element in this world's global when it is a code field, and says whether it is.
constexpr char16_t kProbe[] =
    u"(()=>{const e=document.activeElement;"
    u"const ok=e instanceof HTMLInputElement&&!e.disabled&&!e.readOnly&&"
    u"e.autocomplete.split(' ').includes('one-time-code');"
    u"window.__baaCodeField=ok?e:null;return ok})()";

// A new document has a new global, so a frame that navigated finds no field. Text goes in as
// typing does (input events, undo, maxlength); the fallback is for fields that refuse execCommand.
std::u16string FillScript(const std::string& code) {
  return base::UTF8ToUTF16(
      "(()=>{const e=window.__baaCodeField;if(!e||document.activeElement!==e)return;e.select();"
      "if(!document.execCommand('insertText',false,'" + code + "')){e.value='" + code +
      "';e.dispatchEvent(new Event('input',{bubbles:true}));"
      "e.dispatchEvent(new Event('change',{bubbles:true}))}})()");
}

// What goes into a script string: no quote, backslash or markup can.
bool IsCode(const std::string& code) {
  if (code.empty() || code.size() > 32) {
    return false;
  }
  for (char c : code) {
    if (!(base::IsAsciiAlphaNumeric(c) || c == '-')) {
      return false;
    }
  }
  return true;
}

class CodeWatch : public content::WebContentsObserver {
 public:
  CodeWatch(uint64_t tab, content::WebContents* wc) : content::WebContentsObserver(wc), tab_(tab) {}

  void Fill(uint64_t id, const std::string& code) {
    if (id == 0 || id != id_ || !IsCode(code)) {
      return;
    }
    content::RenderFrameHost* frame = content::RenderFrameHost::FromID(frame_);
    if (!frame || !frame->IsActive() || frame->GetLastCommittedOrigin() != origin_ ||
        web_contents()->GetFocusedFrame() != frame) {
      return;
    }
    frame->ExecuteJavaScriptInIsolatedWorld(FillScript(code), base::NullCallback(), kWorld);
  }

  // content::WebContentsObserver:
  void OnFocusChangedInPage(const content::FocusedNodeDetails& details) override {
    Withdraw();
    ++focus_;
    content::RenderFrameHost* frame = details.global_dom_node_id.document.AsRenderFrameHostIfValid();
    if (!frame || details.editable_level == content::EditableLevel::kNotEditable ||
        !frame->IsActive() || !frame->GetPage().IsPrimary()) {
      return;
    }
    // WebContentsImpl made the screen rectangle by adding the view's origin to the rectangle in
    // the view.
    gfx::Rect bounds = details.node_bounds_in_screen;
    if (content::RenderWidgetHostView* view = web_contents()->GetRenderWidgetHostView()) {
      bounds.Offset(-view->GetViewBounds().OffsetFromOrigin());
    }
    frame->ExecuteJavaScriptInIsolatedWorld(
        kProbe,
        base::BindOnce(&CodeWatch::Probed, weak_factory_.GetWeakPtr(), focus_, frame->GetGlobalId(),
                       bounds),
        kWorld);
  }
  void DidFinishNavigation(content::NavigationHandle* handle) override {
    if (handle->HasCommitted() && !handle->IsSameDocument() &&
        (handle->IsInPrimaryMainFrame() || handle->GetFrameTreeNodeId() == node_)) {
      Withdraw();
    }
  }
  void RenderFrameDeleted(content::RenderFrameHost* frame) override {
    if (frame->GetGlobalId() == frame_) {
      Withdraw();
    }
  }

 private:
  void Probed(int focus, content::GlobalRenderFrameHostId frame_id, gfx::Rect bounds, base::Value is_field) {
    content::RenderFrameHost* frame = content::RenderFrameHost::FromID(frame_id);
    // A later focus change won: this answer is about an element that no longer has focus.
    if (focus != focus_ || !is_field.is_bool() || !is_field.GetBool() || !frame || !frame->IsActive()) {
      return;
    }
    id_ = NextId();
    frame_ = frame_id;
    node_ = frame->GetFrameTreeNodeId();
    origin_ = frame->GetLastCommittedOrigin();
    origin_s_ = origin_.Serialize();
    top_s_ = web_contents()->GetPrimaryMainFrame()->GetLastCommittedOrigin().Serialize();
    bounds_ = bounds;
    Send(true);
  }

  void Withdraw() {
    if (id_ == 0) {
      return;
    }
    Send(false);
    id_ = 0;
    frame_ = {};
    node_ = {};
  }

  void Send(bool shown) {
    if (!TabAnnounced(TabFromId(tab_))) {
      return;
    }
    Callbacks().code_field(CallbackContext(), tab_, id_, origin_s_.c_str(), top_s_.c_str(), bounds_.x(),
                           bounds_.y(), bounds_.width(), bounds_.height(), shown);
  }

  const uint64_t tab_;
  int focus_ = 0;
  // The field shown now; 0 for none.
  uint64_t id_ = 0;
  content::GlobalRenderFrameHostId frame_;
  content::FrameTreeNodeId node_;
  url::Origin origin_;
  std::string origin_s_, top_s_;
  gfx::Rect bounds_;
  base::WeakPtrFactory<CodeWatch> weak_factory_{this};
};

std::map<uint64_t, std::unique_ptr<CodeWatch>>& Watches() {
  static base::NoDestructor<std::map<uint64_t, std::unique_ptr<CodeWatch>>> watches;
  return *watches;
}

}  // namespace

void WatchCodes(uint64_t tab, content::WebContents* wc) {
  Watches()[tab] = std::make_unique<CodeWatch>(tab, wc);
}

void UnwatchCodes(uint64_t tab) {
  Watches().erase(tab);
}

}  // namespace baa

void baa_tab_fill_code(BaaTabId tab, uint64_t id, const char* code) {
  auto it = baa::Watches().find(tab);
  if (it != baa::Watches().end() && code) {
    it->second->Fill(id, code);
  }
}

size_t baa_site_of(const char* host, char* out, size_t len) {
  std::string input = host ? host : "";
  GURL url("https://" + input);
  std::string site = url.is_valid() ? net::registry_controlled_domains::GetDomainAndRegistry(
                                          url, net::registry_controlled_domains::INCLUDE_PRIVATE_REGISTRIES)
                                    : "";
  if (site.empty()) {
    site = url.is_valid() ? url.host() : input;
  }
  if (out && len > 0) {
    size_t n = std::min(site.size(), len - 1);
    site.copy(out, n);
    out[n] = '\0';
  }
  return site.size();
}

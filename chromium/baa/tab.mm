// BaaTab: the registry, its lifecycle and every baa_tab_* call.
// PLAN-chromium-phase3.md "baa::Tab".

#import <Cocoa/Cocoa.h>

#include <algorithm>
#include <map>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/logging.h"
#include "base/no_destructor.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "chrome/browser/favicon/favicon_utils.h"
#include "chrome/browser/printing/print_view_manager_common.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ssl/chrome_security_state_util.h"
#include "chrome/browser/ui/browser_tabstrip.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/tab_list/tab_removed_reason.h"
#include "chrome/browser/ui/tabs/tab_muted_utils.h"
#include "components/favicon/content/content_favicon_driver.h"
#include "components/favicon/core/favicon_driver_observer.h"
#include "components/find_in_page/find_tab_helper.h"
#include "components/find_in_page/find_types.h"
#include "components/permissions/permission_prompt.h"
#include "components/permissions/permission_request_manager.h"
#include "components/security_state/core/security_state.h"
#include "components/tabs/public/tab_interface.h"
#include "components/zoom/zoom_controller.h"
#include "content/public/browser/navigation_controller.h"
#include "content/public/browser/navigation_entry.h"
#include "content/public/browser/navigation_handle.h"
#include "content/public/browser/render_frame_host.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_observer.h"
#include "third_party/blink/public/common/page/page_zoom.h"
#include "third_party/blink/public/common/web_preferences/web_preferences.h"
#include "third_party/blink/public/mojom/devtools/console_message.mojom.h"
#include "third_party/blink/public/mojom/webpreferences/web_preferences.mojom.h"
#include "third_party/blink/public/mojom/frame/user_activation_notification_type.mojom.h"
#include "ui/base/page_transition_types.h"
#include "ui/gfx/codec/png_codec.h"
#include "ui/gfx/image/image.h"
#include "ui/gfx/image/image_skia.h"
#include "ui/gfx/image/image_skia_rep.h"
#include "url/gurl.h"

struct BaaTab;

namespace {

// ---- Tabs registry -----------------------------------------------------------

// Every page the app holds a handle to. A page outside a tab strip (an
// extension popup) has no TabInterface: `!tab`.
std::vector<std::unique_ptr<BaaTab>>& Registry() {
  static base::NoDestructor<std::vector<std::unique_ptr<BaaTab>>> registry;
  return *registry;
}

// The insertion in progress, set around code that makes a tab inside Chrome.
baa::ScopedInsertion* g_insertion = nullptr;

uint64_t g_next_id = 1;

}  // namespace

// ---- BaaTab ------------------------------------------------------------------

struct BaaTab : content::WebContentsObserver,
                favicon::FaviconDriverObserver {
  explicit BaaTab(tabs::TabInterface* tab) : tab(tab) {}
  ~BaaTab() override = default;

  tabs::TabInterface* const tab;  // null for a page outside a tab strip
  base::CallbackListSubscription will_detach;
  // A page outside a tab strip: runs when the handle ends, to destroy what
  // holds the contents.
  base::OnceClosure close_bare;
  // baa_tab_set_init_script / baa_tab_set_color_scheme.
  std::u16string init_script;
  int color_scheme = 0;
  // Chrome-made tabs are announced (tab_created) once their URL is known;
  // until then they are silent.
  bool reported = true;
  bool background = false;
  bool user_gesture = false;
  base::WeakPtr<BaaTab> opener;
  std::string icon_url;
  // Storage behind get_state's strings.
  std::string url_s, title_s, icon_s;
  base::WeakPtrFactory<BaaTab> weak_factory{this};

  content::WebContents* contents() const { return web_contents(); }

  void Attach(content::WebContents* wc);
  void ApplyColorScheme();
  void Detach();

  BaaTabState Snapshot(std::string& url, std::string& title, std::string& icon) {
    BaaTabState state = {};
    content::WebContents* wc = web_contents();
    url = wc->GetVisibleURL().spec();
    title = base::UTF16ToUTF8(wc->GetTitle());
    icon = icon_url;
    state.url = url.c_str();
    state.title = title.c_str();
    state.favicon_url = icon.c_str();
    state.view = (__bridge void*)wc->GetNativeView().GetNativeNSView();
    state.loading = wc->IsLoading();
    state.can_go_back = wc->GetController().CanGoBack();
    state.can_go_forward = wc->GetController().CanGoForward();
    state.audible = wc->IsCurrentlyAudible();
    state.secure = chrome_security_state::GetSecurityLevel(wc) == security_state::SECURE;
    return state;
  }

  void SendState() {
    if (!reported || !web_contents()) {
      return;
    }
    std::string url, title, icon;
    BaaTabState state = Snapshot(url, title, icon);
    baa::Callbacks().tab_state(baa::CallbackContext(), this, &state);
  }

  void TryReport(const GURL& url, bool force);

  // content::WebContentsObserver:
  void DidStartLoading() override { SendState(); }
  void DidStopLoading() override { SendState(); }
  void TitleWasSet(content::NavigationEntry*) override { SendState(); }
  void NavigationEntryCommitted(
      const content::LoadCommittedDetails&) override {
    SendState();
  }
  void NavigationListPruned(const content::PrunedDetails&) override {
    SendState();
  }
  void NavigationEntryChanged(const content::EntryChangedDetails&) override {
    SendState();
  }
  void OnAudioStateChanged(bool) override { SendState(); }
  void DidChangeVisibleSecurityState() override { SendState(); }
  void DidStartNavigation(content::NavigationHandle* handle) override {
    if (!reported && handle->IsInPrimaryMainFrame()) {
      TryReport(handle->GetURL(), false);
    }
  }
  void DidFinishNavigation(content::NavigationHandle* handle) override {
    if (!handle->IsInPrimaryMainFrame()) {
      return;
    }
    if (handle->HasCommitted() && !handle->IsSameDocument()) {
      icon_url.clear();
    }
    SendState();
  }
  void PrimaryMainDocumentElementAvailable() override {
    ApplyColorScheme();
    if (!init_script.empty()) {
      web_contents()->GetPrimaryMainFrame()->ExecuteJavaScript(init_script, base::NullCallback());
    }
  }
  // Pages tell the app things through console.debug("baa:..."), e.g. the
  // DevTools dock picker.
  void OnDidAddMessageToConsole(content::RenderFrameHost*,
                                blink::mojom::ConsoleMessageLevel,
                                const std::u16string& message,
                                int32_t,
                                const std::u16string&,
                                const std::optional<std::u16string>&) override {
    constexpr std::u16string_view kPrefix = u"baa:";
    if (message.starts_with(kPrefix) && reported) {
      std::string rest = base::UTF16ToUTF8(message.substr(kPrefix.size()));
      baa::Callbacks().page_message(baa::CallbackContext(), this, rest.c_str());
    }
  }
  void WebContentsDestroyed() override { Detach(); }

  // favicon::FaviconDriverObserver:
  void OnFaviconUpdated(favicon::FaviconDriver*,
                        NotificationIconType,
                        const GURL& icon_url_in,
                        bool,
                        const gfx::Image& image) override {
    if (image.IsEmpty()) {
      return;
    }
    icon_url = icon_url_in.spec();
    if (!reported) {
      return;
    }
    SendState();
    // 2x when the site has it.
    SkBitmap bitmap = image.AsImageSkia().GetRepresentation(2.0f).GetBitmap();
    std::vector<uint8_t> png;
    if (!bitmap.empty()) {
      if (auto encoded = gfx::PNGCodec::EncodeBGRASkBitmap(
              bitmap, /*discard_transparency=*/false)) {
        png = std::move(*encoded);
      }
    }
    if (png.empty()) {
      if (auto bytes = image.As1xPNGBytes()) {
        png.assign(bytes->begin(), bytes->end());
      }
    }
    if (!png.empty()) {
      baa::Callbacks().tab_favicon(baa::CallbackContext(), this,
                                   icon_url.c_str(), png.data(), png.size());
    }
  }
};

namespace baa {

namespace {

BaaTab* Find(tabs::TabInterface* ti) {
  for (auto& value : Registry()) {
    if (ti && value->tab == ti) {
      return value.get();
    }
  }
  return nullptr;
}

// The one way a handle ends. Everything that hangs off the tab goes with it;
// the app hears tab_closed (`tell`) only for closes Chrome made, and only if it
// knew the tab. Unregistered first, so whatever this or the destruction that
// follows re-enters finds nothing.
void Forget(BaaTab* t, bool tell) {
  auto it = std::find_if(Registry().begin(), Registry().end(),
                         [&](auto& value) { return value.get() == t; });
  if (it == Registry().end()) {
    return;
  }
  std::unique_ptr<BaaTab> owned = std::move(*it);
  Registry().erase(it);
  tell = tell && t->reported;
  DropPermissionsOf(t, tell);
  DropMenusOf(t);
  ForgetDevtoolsOf(t);
  DropDialogsOf(t, tell);
  if (tell) {
    Callbacks().tab_closed(CallbackContext(), t);
  }
  t->Detach();
  base::OnceClosure close = std::move(t->close_bare);
  owned.reset();
  if (close) {
    std::move(close).Run();
  }
}

}  // namespace

ScopedInsertion::ScopedInsertion(InsertionKind kind, BaaTab** out)
    : kind(kind), out(out), prev(g_insertion) {
  *out = nullptr;
  g_insertion = this;
}

ScopedInsertion::~ScopedInsertion() {
  g_insertion = prev;
}

bool DevtoolsDocking() {
  return g_insertion && g_insertion->kind == InsertionKind::kDevtoolsFront;
}

BaaTab* TabFor(content::WebContents* contents) {
  if (!contents) {
    return nullptr;
  }
  auto* ti = tabs::TabInterface::MaybeGetFromContents(contents);
  for (auto& value : Registry()) {
    if ((ti && value->tab == ti) || value->web_contents() == contents) {
      return value.get();
    }
  }
  return nullptr;
}

bool TabAlive(BaaTab* tab) {
  for (auto& value : Registry()) {
    if (value.get() == tab) {
      return true;
    }
  }
  return false;
}

}  // namespace baa

void BaaTab::Attach(content::WebContents* wc) {
  Observe(wc);
  if (auto* driver = favicon::ContentFaviconDriver::FromWebContents(wc)) {
    driver->AddObserver(this);
  }
  if (!tab) {
    return;  // a popup has no tab helpers: its dialogs are app-modal
  }
  baa::AttachDialogs(wc);
  baa::AttachPermissions(wc);
}

void BaaTab::ApplyColorScheme() {
  content::WebContents* wc = web_contents();
  if (!wc || color_scheme == 0) {
    return;
  }
  blink::web_pref::WebPreferences prefs = wc->GetOrCreateWebPreferences();
  prefs.preferred_color_scheme = color_scheme == 2 ? blink::mojom::PreferredColorScheme::kDark
                                                   : blink::mojom::PreferredColorScheme::kLight;
  wc->SetWebPreferences(prefs);
}

void BaaTab::Detach() {
  content::WebContents* wc = web_contents();
  if (!wc) {
    return;
  }
  if (auto* driver = favicon::ContentFaviconDriver::FromWebContents(wc)) {
    driver->RemoveObserver(this);
  }
  Observe(nullptr);
}

void BaaTab::TryReport(const GURL& url, bool force) {
  if (reported) {
    return;
  }
  if (!force && (url.is_empty() || url.spec() == "about:blank")) {
    return;
  }
  reported = true;
  BaaTab* opener_tab = opener.get();
  BaaProfile* profile = baa::ProfileFor(Profile::FromBrowserContext(web_contents()->GetBrowserContext()));
  std::string spec = url.spec();
  baa::Callbacks().tab_created(baa::CallbackContext(), this, opener_tab,
                               profile, spec.c_str(), background, user_gesture);
  // A dialog the page raised before the app knew the tab.
  baa::ShowPendingDialogsOf(this);
}

// ---- window.mm entry points ----------------------------------------------------

namespace baa {

void OnWillDetach(tabs::TabInterface* tab, tabs::TabInterface::DetachReason reason);

void OnTabInserted(TabStripModel* model,
                   tabs::TabInterface* tab,
                   int index,
                   bool background) {
  if (Find(tab)) {
    return;  // moved between windows, or ours already
  }
  auto owned = std::make_unique<BaaTab>(tab);
  BaaTab* t = owned.get();
  Registry().push_back(std::move(owned));
  content::WebContents* wc = tab->GetContents();
  t->will_detach = tab->RegisterWillDetach(base::BindRepeating(&OnWillDetach));
  t->Attach(wc);
  if (g_insertion) {
    *g_insertion->out = t;  // the app's own insertion: no tab_created
    return;
  }
  t->reported = false;
  t->background = background;
  // window.opener first; else the strip's opener (the active tab at insertion,
  // which is the page a link was clicked in when baa keeps the shown tab active).
  tabs::TabInterface* opener_tab = nullptr;
  content::RenderFrameHost* opener_frame = wc->GetOpener();
  if (opener_frame) {
    opener_tab = tabs::TabInterface::MaybeGetFromContents(
        content::WebContents::FromRenderFrameHost(opener_frame));
  }
  if (!opener_tab) {
    opener_tab = model->GetOpenerOfTabAt(index);
  }
  if (BaaTab* from_tab = Find(opener_tab)) {
    t->opener = WeakTab(from_tab);
    content::WebContents* from = from_tab->web_contents();
    t->user_gesture = from && (from->HasRecentInteraction() ||
                               from->GetPrimaryMainFrame()->HasTransientUserActivation());
  }
  // window.open() inserts the tab blank and navigates it right after, so the
  // URL is awaited (DidStartNavigation), but not for long: a tab that never
  // navigates is still announced.
  base::SingleThreadTaskRunner::GetCurrentDefault()->PostDelayedTask(
      FROM_HERE,
      base::BindOnce(
          [](base::WeakPtr<BaaTab> weak) {
            if (weak && weak->web_contents()) {
              weak->TryReport(weak->web_contents()->GetVisibleURL(), true);
            }
          },
          t->weak_factory.GetWeakPtr()),
      base::Milliseconds(300));
  t->TryReport(wc->GetVisibleURL(), false);
}

// Chrome deleting a tab, from any window (a tab reparented into a Chrome-made
// Browser still dies through here): before its WebContents and NSView go.
void OnWillDetach(tabs::TabInterface* tab, tabs::TabInterface::DetachReason reason) {
  if (reason == tabs::TabInterface::DetachReason::kDelete) {
    Forget(Find(tab), /*tell=*/true);
  }
}

void OnTabRemoved(tabs::TabInterface* tab, TabRemovedReason reason) {
  BaaTab* t = Find(tab);
  if (!t || TabRemoveReasonUtils::WillDeleteTab(reason)) {
    return;  // deletion is OnWillDetach's
  }
  // Moving to another strip. If it lands somewhere that is not ours (an app
  // Browser), the app no longer shows it: give Chrome its own dialogs and
  // prompts back, answer what is pending, and forget the handle.
  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
      FROM_HERE, base::BindOnce(
                     [](base::WeakPtr<BaaTab> weak) {
                       if (!weak) {
                         return;
                       }
                       content::WebContents* wc = weak->web_contents();
                       BrowserWindowInterface* browser = wc ? weak->tab->GetBrowserWindowInterface() : nullptr;
                       if (browser && baa::IsOurBrowser(browser)) {
                         return;
                       }
                       if (wc) {
                         ReturnDialogsToChrome(weak.get(), wc);
                         RestoreChromePermissions(wc);
                       }
                       Forget(weak.get(), /*tell=*/true);
                     },
                     WeakTab(t)));
}

void OnTabReplaced(tabs::TabInterface* tab, content::WebContents* new_contents) {
  BaaTab* t = Find(tab);
  if (!t) {
    return;
  }
  t->Detach();
  DropMenusOf(t);
  t->icon_url.clear();
  t->Attach(new_contents);
  t->SendState();
}

}  // namespace baa

// Strip tabs first, then popups (their Browser is the one the strip tabs
// leave behind).
void baa::CloseTabsOf(Profile* chrome_profile) {
  std::vector<BaaTab*> doomed;
  for (bool bare : {false, true}) {
    for (auto& value : Registry()) {
      content::WebContents* wc = value->web_contents();
      if ((value->tab == nullptr) == bare && wc &&
          Profile::FromBrowserContext(wc->GetBrowserContext()) == chrome_profile) {
        doomed.push_back(value.get());
      }
    }
  }
  for (BaaTab* tab : doomed) {
    baa_tab_close(tab);
  }
}

// ---- C ABI: tabs ---------------------------------------------------------------

BaaTab* baa_tab_create(BaaProfile* profile, const char* url) {
  if (baa_is_quitting()) {
    return nullptr;
  }
  BrowserWindowInterface* browser = baa::BrowserOf(profile);
  if (!browser) {
    return nullptr;
  }
  BaaTab* unused;  // only the DevTools front needs the out-pointer
  content::WebContents* wc;
  {
    baa::ScopedInsertion insertion(baa::InsertionKind::kBaa, &unused);
    wc = chrome::AddAndReturnTabAt(browser, GURL(url), -1, /*foreground=*/false);
  }
  BaaTab* tab = baa::TabFor(wc);
  // Navigate sends a URL for an OTR profile to the parent's Browser; a tab
  // in another profile than asked for is not ours to hand out.
  if (wc && tab && Profile::FromBrowserContext(wc->GetBrowserContext()) != baa::ChromeProfileOf(profile)) {
    baa_tab_close(tab);
    return nullptr;
  }
  return tab;
}

void baa_tab_close(BaaTab* tab) {
  if (!baa::TabAlive(tab)) {
    return;
  }
  tabs::TabInterface* ti = tab->tab;
  // Unregistered before the contents die: the observers that destruction
  // wakes find nothing, and the app is not told of a close it asked for.
  baa::Forget(tab, /*tell=*/false);
  if (!ti) {
    return;  // a popup: Forget ran the holder's close
  }
  BrowserWindowInterface* browser = ti->GetBrowserWindowInterface();
  TabStripModel* model = browser ? browser->GetTabStripModel() : nullptr;
  int index = model ? model->GetIndexOfTab(ti) : -1;
  if (index >= 0) {
    model->DetachAndDeleteWebContentsAt(index);
  }
}

bool baa_tab_get_state(BaaTab* tab, BaaTabState* state) {
  if (!baa::ContentsOf(tab)) {
    return false;
  }
  *state = tab->Snapshot(tab->url_s, tab->title_s, tab->icon_s);
  return true;
}

void baa_tab_load_url(BaaTab* tab, const char* url) {
  if (content::WebContents* wc = baa::ContentsOf(tab)) {
    content::NavigationController::LoadURLParams params{GURL(url)};
    params.transition_type = ui::PAGE_TRANSITION_TYPED;
    wc->GetController().LoadURLWithParams(params);
  }
}

void baa_tab_reload(BaaTab* tab) {
  if (content::WebContents* wc = baa::ContentsOf(tab)) {
    wc->GetController().Reload(content::ReloadType::NORMAL, true);
  }
}

void baa_tab_stop(BaaTab* tab) {
  if (content::WebContents* wc = baa::ContentsOf(tab)) {
    wc->Stop();
  }
}

void baa_tab_go_to_offset(BaaTab* tab, int offset) {
  if (content::WebContents* wc = baa::ContentsOf(tab)) {
    if (wc->GetController().CanGoToOffset(offset)) {
      wc->GetController().GoToOffset(offset);
    }
  }
}

int baa_tab_history(BaaTab* tab, BaaHistoryVisitor visit, void* ctx) {
  content::WebContents* wc = baa::ContentsOf(tab);
  if (!wc) {
    return -1;
  }
  content::NavigationController& controller = wc->GetController();
  for (int i = 0; i < controller.GetEntryCount(); ++i) {
    content::NavigationEntry* entry = controller.GetEntryAtIndex(i);
    std::string url = entry->GetVirtualURL().spec();
    std::string title = base::UTF16ToUTF8(entry->GetTitle());
    visit(ctx, title.empty() ? url.c_str() : title.c_str(), url.c_str());
  }
  return controller.GetCurrentEntryIndex();
}

void baa_tab_eval(BaaTab* tab, const char* script, bool user_gesture) {
  content::WebContents* wc = baa::ContentsOf(tab);
  if (!wc) {
    return;
  }
  content::RenderFrameHost* frame = wc->GetPrimaryMainFrame();
  if (user_gesture) {
    frame->NotifyUserActivation(
        blink::mojom::UserActivationNotificationType::kInteraction);
  }
  frame->ExecuteJavaScript(base::UTF8ToUTF16(script), base::NullCallback());
}

void baa_tab_set_zoom(BaaTab* tab, double factor) {
  if (content::WebContents* wc = baa::ContentsOf(tab)) {
    if (auto* zoom = zoom::ZoomController::FromWebContents(wc)) {
      zoom->SetZoomLevel(blink::ZoomFactorToZoomLevel(factor));
    }
  }
}

void baa_tab_find(BaaTab* tab, const char* text, bool forward) {
  content::WebContents* wc = baa::ContentsOf(tab);
  if (!wc) {
    return;
  }
  auto* helper = find_in_page::FindTabHelper::FromWebContents(wc);
  if (!helper) {
    return;
  }
  std::u16string query = base::UTF8ToUTF16(text);
  if (query.empty()) {
    helper->StopFinding(find_in_page::SelectionAction::kClear);
    return;
  }
  helper->StartFinding(query, forward, /*case_sensitive=*/false,
                       /*find_match=*/true);
}

void baa_tab_print(BaaTab* tab) {
  if (content::WebContents* wc = baa::ContentsOf(tab)) {
    printing::StartBasicPrint(wc);
  }
}

void baa_tab_set_muted(BaaTab* tab, bool muted) {
  if (content::WebContents* wc = baa::ContentsOf(tab)) {
    SetTabAudioMuted(wc, muted, TabMutedReason::kAudioIndicator, "");
  }
}

void baa_tab_activate(BaaTab* tab) {
  if (!baa::ContentsOf(tab)) {
    return;
  }
  if (!tab->tab) {
    return;
  }
  BrowserWindowInterface* browser = tab->tab->GetBrowserWindowInterface();
  if (!browser) {
    return;
  }
  TabStripModel* model = browser->GetTabStripModel();
  int index = model->GetIndexOfTab(tab->tab);
  if (index >= 0) {
    model->ActivateTabAt(index);
  }
  baa::ActivateBrowser(browser);
}

void baa_tab_focus(BaaTab* tab) {
  if (content::WebContents* wc = baa::ContentsOf(tab)) {
    wc->Focus();
  }
}

namespace baa {

uint64_t NextId() {
  return g_next_id++;
}

bool TabAnnounced(BaaTab* tab) {
  return tab && tab->reported;
}

content::WebContents* ContentsOf(BaaTab* tab) {
  return TabAlive(tab) ? tab->web_contents() : nullptr;
}

BaaTab* CreateBareTab(content::WebContents* contents, base::OnceClosure close) {
  auto owned = std::make_unique<BaaTab>(nullptr);
  BaaTab* tab = owned.get();
  tab->close_bare = std::move(close);
  Registry().push_back(std::move(owned));
  tab->Attach(contents);
  return tab;
}

// The holder closed the page itself (a popup's window.close(), its extension
// unloaded): tab_closed once, then the handle is dead.
void BareTabClosedByChrome(BaaTab* tab) {
  if (!baa::TabAlive(tab)) {
    return;
  }
  tab->close_bare = base::OnceClosure();
  Forget(tab, /*tell=*/true);
}

// Quit: popups are not in any tab strip, so nothing else closes them, and a
// BrowserContext may not die with a WebContents alive.
void CloseBareTabsForQuit() {
  for (;;) {
    auto bare = std::find_if(Registry().begin(), Registry().end(),
                             [](auto& value) { return !value->tab; });
    if (bare == Registry().end()) {
      break;
    }
    Forget(bare->get(), /*tell=*/true);
  }
  // Popups that closed themselves and wait for their deletion task too.
  DestroyAllPopups();
}

// Sends the first state of a page nobody told the app about yet.
void SendTabState(BaaTab* tab) {
  if (baa::TabAlive(tab)) {
    tab->SendState();
  }
}

}  // namespace baa

void baa_tab_set_init_script(BaaTab* tab, const char* script) {
  if (baa::TabAlive(tab)) {
    tab->init_script = base::UTF8ToUTF16(script ? script : "");
  }
}

void baa_tab_set_color_scheme(BaaTab* tab, int scheme) {
  if (!baa::TabAlive(tab)) {
    return;
  }
  tab->color_scheme = scheme;
  if (content::WebContents* wc = tab->web_contents()) {
    if (scheme == 0) {
      wc->OnWebPreferencesChanged();  // back to what the system says
    } else {
      tab->ApplyColorScheme();
    }
  }
}

base::WeakPtr<BaaTab> baa::WeakTab(BaaTab* tab) {
  return tab ? tab->weak_factory.GetWeakPtr() : nullptr;
}

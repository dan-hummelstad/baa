// BaaProfile, BaaTab, JS dialogs and every baa_profile_* / baa_tab_* call.
// PLAN-chromium-phase3.md "baa::Profile", "baa::Tab", "baa::DialogDelegate".

#import <Cocoa/Cocoa.h>

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
#include "base/threading/thread_restrictions.h"
#include "base/timer/elapsed_timer.h"
#include "chrome/browser/browser_process.h"
#include "chrome/browser/favicon/favicon_utils.h"
#include "chrome/browser/printing/print_view_manager_common.h"
#include "chrome/browser/profiles/keep_alive/profile_keep_alive_types.h"
#include "chrome/browser/profiles/keep_alive/scoped_profile_keep_alive.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/profiles/profile_destroyer.h"
#include "chrome/browser/profiles/profile_manager.h"
#include "chrome/browser/profiles/profile_observer.h"
#include "chrome/browser/ssl/chrome_security_state_util.h"
#include "chrome/browser/ui/browser_tabstrip.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/browser_window/public/create_browser_window.h"
#include "chrome/browser/ui/javascript_dialogs/javascript_tab_modal_dialog_manager_delegate_desktop.h"
#include "chrome/browser/tab_list/tab_removed_reason.h"
#include "chrome/browser/ui/tabs/tab_muted_utils.h"
#include "chrome/common/chrome_constants.h"
#include "chrome/common/pref_names.h"
#include "components/favicon/content/content_favicon_driver.h"
#include "components/favicon/core/favicon_driver_observer.h"
#include "components/find_in_page/find_tab_helper.h"
#include "components/find_in_page/find_types.h"
#include "components/javascript_dialogs/tab_modal_dialog_manager.h"
#include "components/javascript_dialogs/tab_modal_dialog_view.h"
#include "components/permissions/permission_prompt.h"
#include "components/permissions/permission_request_manager.h"
#include "components/prefs/pref_service.h"
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

// ---- Profiles ----------------------------------------------------------------

struct BaaProfile : ProfileObserver {
  BaaProfile(Profile* profile, bool ephemeral)
      : profile(profile), ephemeral(ephemeral) {
    observation.Observe(profile);
    if (!ephemeral) {
      keep_alive = std::make_unique<ScopedProfileKeepAlive>(
          profile, ProfileKeepAliveOrigin::kBrowserWindow);
    }
    // Printing goes through the system panel (printing::StartBasicPrint);
    // the in-page preview needs a views dialog host.
    profile->GetPrefs()->SetBoolean(prefs::kPrintPreviewDisabled, true);
    // Autofill and password bubbles need a BrowserView (the window has no
    // autofill bubble handler).
    for (const char* name : {"autofill.profile_enabled", "autofill.credit_card_enabled",
                             "autofill.payment_methods_enabled", "credentials_enable_service",
                             "credentials_enable_autosignin"}) {
      baa::SetPrefBool(profile, name, false);
    }
  }

  void OnProfileWillBeDestroyed(Profile*) override {
    observation.Reset();
    profile = nullptr;
    browser = nullptr;
    browser_closed = {};
  }

  // The Browser holding this profile's tabs, made on first need and again
  // after it closed. Null when the profile is gone or Chrome refuses.
  BrowserWindowInterface* GetBrowser() {
    // A Browser whose last tab just closed is on its way out (Window::Close
    // is posted); a tab put in it now would be closed with it.
    if (browser && baa::BrowserIsClosing(browser)) {
      browser = nullptr;
      browser_closed = {};
    }
    if (browser) {
      return browser;
    }
    if (!profile || baa_is_quitting() ||
        GetBrowserWindowCreationStatusForProfile(*profile) !=
            BrowserWindowInterface::CreationStatus::kOk) {
      return nullptr;
    }
    BrowserWindowCreateParams params(BrowserWindowInterface::TYPE_NORMAL,
                                     *profile, /*from_user_gesture=*/false);
    // baa-model is the session, not Chrome's.
    params.should_trigger_session_restore = false;
    params.omit_from_session_restore = true;
    browser = CreateBrowserWindow(std::move(params));
    browser_closed = browser->RegisterBrowserDidClose(base::BindRepeating(
        [](BaaProfile* self, BrowserWindowInterface*) {
          self->browser = nullptr;
          self->browser_closed = {};
        },
        base::Unretained(this)));
    return browser;
  }

  raw_ptr<Profile> profile;
  const bool ephemeral;
  std::string name;
  std::unique_ptr<ScopedProfileKeepAlive> keep_alive;
  raw_ptr<BrowserWindowInterface> browser;
  base::CallbackListSubscription browser_closed;
  base::ScopedObservation<Profile, ProfileObserver> observation{this};
};

namespace {

std::vector<std::unique_ptr<BaaProfile>>& Profiles() {
  static base::NoDestructor<std::vector<std::unique_ptr<BaaProfile>>> profiles;
  return *profiles;
}

BaaProfile* AliveProfile(BaaProfile* profile) {
  for (auto& p : Profiles()) {
    if (p.get() == profile) {
      return profile;
    }
  }
  return nullptr;
}

BaaProfile* ProfileOf(Profile* profile) {
  for (auto& p : Profiles()) {
    if (p->profile == profile) {
      return p.get();
    }
  }
  return nullptr;
}

// ---- Tabs registry -----------------------------------------------------------

std::map<tabs::TabInterface*, std::unique_ptr<BaaTab>>& Tabs() {
  static base::NoDestructor<std::map<tabs::TabInterface*, std::unique_ptr<BaaTab>>>
      tabs;
  return *tabs;
}

// Set around baa_tab_create so the insertion it causes is not reported as a
// tab Chrome made.
bool g_creating = false;

}  // namespace

// ---- JS dialogs --------------------------------------------------------------

namespace {

class DialogView;
std::map<uint64_t, std::unique_ptr<DialogView>>& Dialogs() {
  static base::NoDestructor<std::map<uint64_t, std::unique_ptr<DialogView>>> d;
  return *d;
}
uint64_t g_next_id = 1;

BaaTab* TabOf(content::WebContents* contents);
bool AliveTab(BaaTab* tab);

// The "view" Chrome's dialog manager holds: nothing is drawn here, the app
// shows the dialog and answers through baa_dialog_answer.
class DialogView : public javascript_dialogs::TabModalDialogView {
 public:
  DialogView(uint64_t id,
             BaaTab* tab,
             std::u16string default_text,
             content::JavaScriptDialogManager::DialogClosedCallback callback)
      : id_(id),
        tab_(tab),
        default_text_(std::move(default_text)),
        callback_(std::move(callback)) {}

  // The manager makes the dialog go away (navigation, tab hidden, a newer
  // dialog). Not called back for answers, which forget the id first.
  void CloseDialogWithoutCallback() override;
  std::u16string GetUserInput() override { return default_text_; }

  base::WeakPtr<DialogView> GetWeakPtr() { return weak_factory_.GetWeakPtr(); }
  content::JavaScriptDialogManager::DialogClosedCallback TakeCallback() {
    return std::move(callback_);
  }
  void MarkAnswered() { answered_ = true; }
  BaaTab* tab() const { return tab_; }
  uint64_t id() const { return id_; }
  void Remember(int type, std::string message, std::string default_text) {
    type_ = type;
    message_ = std::move(message);
    default_s_ = std::move(default_text);
  }
  int type_ = 0;
  std::string message_, default_s_;

 private:
  const uint64_t id_;
  raw_ptr<BaaTab> tab_;
  std::u16string default_text_;
  content::JavaScriptDialogManager::DialogClosedCallback callback_;
  bool answered_ = false;
  base::WeakPtrFactory<DialogView> weak_factory_{this};
};

}  // namespace

// ---- BaaTab ------------------------------------------------------------------

struct BaaTab : content::WebContentsObserver,
                favicon::FaviconDriverObserver {
  explicit BaaTab(tabs::TabInterface* tab) : tab(tab) {}
  ~BaaTab() override = default;

  tabs::TabInterface* const tab;
  bool closing_by_baa = false;
  // tab_closed already sent (a tab deleted from a window that is not ours).
  bool closed_sent = false;
  base::CallbackListSubscription will_detach;
  // A page that is not in a Browser's tab strip (an extension popup): no
  // TabInterface; baa_tab_close runs this to destroy what holds the contents.
  bool bare = false;
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
  std::u16string find_text;
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
    if (!reported || !web_contents() || !baa::Callbacks().tab_state) {
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
    if (message.starts_with(kPrefix) && reported && baa::Callbacks().page_message) {
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
    if (!baa::Callbacks().tab_favicon) {
      return;
    }
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

namespace {

// Pages outside a tab strip.
std::vector<std::unique_ptr<BaaTab>>& BareTabs() {
  static base::NoDestructor<std::vector<std::unique_ptr<BaaTab>>> bare;
  return *bare;
}

BaaTab* TabOf(content::WebContents* contents) {
  auto* tab = tabs::TabInterface::MaybeGetFromContents(contents);
  auto it = Tabs().find(tab);
  if (it != Tabs().end()) {
    return it->second.get();
  }
  for (auto& value : BareTabs()) {
    if (value->web_contents() == contents) {
      return value.get();
    }
  }
  return nullptr;
}

bool AliveTab(BaaTab* tab) {
  for (auto& [key, value] : Tabs()) {
    if (value.get() == tab) {
      return true;
    }
  }
  for (auto& value : BareTabs()) {
    if (value.get() == tab) {
      return true;
    }
  }
  return false;
}

int KindOf(int type) {
  return type == content::JAVASCRIPT_DIALOG_TYPE_CONFIRM  ? BAA_DIALOG_CONFIRM
         : type == content::JAVASCRIPT_DIALOG_TYPE_PROMPT ? BAA_DIALOG_PROMPT
                                                          : BAA_DIALOG_ALERT;
}

class DialogDelegate : public JavaScriptTabModalDialogManagerDelegateDesktop {
 public:
  explicit DialogDelegate(content::WebContents* wc)
      : JavaScriptTabModalDialogManagerDelegateDesktop(wc), wc_(wc) {}

  base::WeakPtr<javascript_dialogs::TabModalDialogView> CreateNewDialog(
      content::WebContents*,
      const std::u16string&,
      content::JavaScriptDialogType type,
      const std::u16string& message,
      const std::u16string& default_text,
      content::JavaScriptDialogManager::DialogClosedCallback callback,
      base::OnceClosure) override {
    BaaTab* tab = TabOf(wc_);
    uint64_t id = baa::NextId();
    auto view = std::make_unique<DialogView>(id, tab, default_text,
                                             std::move(callback));
    auto weak = view->GetWeakPtr();
    view->Remember(static_cast<int>(type), base::UTF16ToUTF8(message), base::UTF16ToUTF8(default_text));
    Dialogs()[id] = std::move(view);
    if (!tab) {
      // A page nobody shows (its tab handle is gone): answer "no" so it does not hang.
      base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
          FROM_HERE, base::BindOnce([](uint64_t id) { baa_dialog_answer(id, false, ""); }, id));
    } else if (tab->reported && baa::Callbacks().dialog) {
      int kind = KindOf(type);
      baa::Callbacks().dialog(baa::CallbackContext(), tab, id, kind,
                              base::UTF16ToUTF8(message).c_str(),
                              base::UTF16ToUTF8(default_text).c_str());
    }
    return weak;
  }

 private:
  raw_ptr<content::WebContents> wc_;
};

}  // namespace

void DialogView::CloseDialogWithoutCallback() {
  if (answered_) {
    return;
  }
  auto it = Dialogs().find(id_);
  if (it == Dialogs().end()) {
    return;
  }
  // Destroys this; nothing after.
  std::unique_ptr<DialogView> self = std::move(it->second);
  Dialogs().erase(it);
  if (AliveTab(tab_) && baa::Callbacks().dialog_closed) {
    baa::Callbacks().dialog_closed(baa::CallbackContext(), tab_, id_);
  }
}

void BaaTab::Attach(content::WebContents* wc) {
  Observe(wc);
  if (auto* driver = favicon::ContentFaviconDriver::FromWebContents(wc)) {
    driver->AddObserver(this);
  }
  if (bare) {
    return;  // a popup has no tab helpers: its dialogs are app-modal
  }
  // Chrome's views dialog CHECK-crashes with our window; the app draws them.
  wc->RemoveUserData(javascript_dialogs::TabModalDialogManager::UserDataKey());
  javascript_dialogs::TabModalDialogManager::CreateForWebContents(
      wc, std::make_unique<DialogDelegate>(wc));
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
  if (!baa::Callbacks().tab_created) {
    return;
  }
  BaaTab* opener_tab = opener.get();
  BaaProfile* profile = ProfileOf(Profile::FromBrowserContext(web_contents()->GetBrowserContext()));
  std::string spec = url.spec();
  baa::Callbacks().tab_created(baa::CallbackContext(), this, opener_tab,
                               profile, spec.c_str(), background, user_gesture);
  // A dialog the page raised before the app knew the tab.
  if (baa::Callbacks().dialog) {
    for (auto& [id, view] : Dialogs()) {
      if (view->tab() == this) {
        baa::Callbacks().dialog(baa::CallbackContext(), this, id, KindOf(view->type_),
                                view->message_.c_str(), view->default_s_.c_str());
      }
    }
  }
}

// ---- window.mm entry points ----------------------------------------------------

namespace baa {

void OnWillDetach(tabs::TabInterface* tab, tabs::TabInterface::DetachReason reason);

void OnTabInserted(TabStripModel* model,
                   tabs::TabInterface* tab,
                   int index,
                   bool background) {
  if (Tabs().count(tab)) {
    return;  // moved between windows, or ours already
  }
  auto owned = std::make_unique<BaaTab>(tab);
  BaaTab* t = owned.get();
  Tabs()[tab] = std::move(owned);
  content::WebContents* wc = tab->GetContents();
  t->will_detach = tab->RegisterWillDetach(base::BindRepeating(&OnWillDetach));
  t->Attach(wc);
  if (g_creating || DevtoolsDocking()) {
    if (DevtoolsDocking()) {
      NoteDevtoolsFront(t);
    }
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
  auto opener_it = Tabs().find(opener_tab);
  if (opener_it != Tabs().end()) {
    t->opener = opener_it->second->weak_factory.GetWeakPtr();
    content::WebContents* from = opener_it->second->web_contents();
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

// The tab is going away for good, from any window (a tab reparented into a
// Chrome-made Browser still dies through here): tab_closed once, before its
// WebContents and NSView are destroyed.
void ReportClosed(BaaTab* t) {
  if (t->closed_sent || t->closing_by_baa) {
    return;
  }
  t->closed_sent = true;
  DropPermissionsOf(t);
  DropMenusOf(t);
  ForgetDevtoolsOf(t);
  for (auto d = Dialogs().begin(); d != Dialogs().end();) {
    if (d->second->tab() == t) {
      uint64_t id = d->first;
      d = Dialogs().erase(d);
      if (t->reported && Callbacks().dialog_closed) {
        Callbacks().dialog_closed(CallbackContext(), t, id);
      }
    } else {
      ++d;
    }
  }
  if (t->reported && Callbacks().tab_closed) {
    Callbacks().tab_closed(CallbackContext(), t);
  }
}

void OnWillDetach(tabs::TabInterface* tab, tabs::TabInterface::DetachReason reason) {
  auto it = Tabs().find(tab);
  if (it == Tabs().end() || reason != tabs::TabInterface::DetachReason::kDelete) {
    return;
  }
  ReportClosed(it->second.get());
}

void OnTabRemoved(tabs::TabInterface* tab, TabRemovedReason reason) {
  auto it = Tabs().find(tab);
  if (it == Tabs().end()) {
    return;
  }
  BaaTab* t = it->second.get();
  if (TabRemoveReasonUtils::WillDeleteTab(reason)) {
    ReportClosed(t);
    t->Detach();
    Tabs().erase(it);
    return;
  }
  // Moving to another strip. If it lands somewhere that is not ours (an app
  // Browser), nothing would report it any more: check once it has settled.
  base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
      FROM_HERE, base::BindOnce(
                     [](base::WeakPtr<BaaTab> weak) {
                       if (!weak || !weak->web_contents()) {
                         return;
                       }
                       tabs::TabInterface* ti = weak->tab;
                       BrowserWindowInterface* browser = ti->GetBrowserWindowInterface();
                       if (browser && baa::IsOurBrowser(browser)) {
                         return;
                       }
                       ReportClosed(weak.get());
                       weak->Detach();
                       Tabs().erase(ti);
                     },
                     t->weak_factory.GetWeakPtr()));
}

void OnTabReplaced(tabs::TabInterface* tab, content::WebContents* new_contents) {
  auto it = Tabs().find(tab);
  if (it == Tabs().end()) {
    return;
  }
  BaaTab* t = it->second.get();
  t->Detach();
  DropMenusOf(t);
  t->icon_url.clear();
  t->Attach(new_contents);
  t->SendState();
}

}  // namespace baa

// ---- C ABI: profiles -----------------------------------------------------------

BaaProfile* baa_profile_open(const char* dir_name) {
  baa::WatchTermination();
  std::string name = dir_name ? dir_name : "";
  if (!g_browser_process || !g_browser_process->profile_manager() ||
      baa_is_quitting() || name.empty() || name.find('/') != std::string::npos ||
      name.find("..") != std::string::npos) {
    return nullptr;
  }
  for (auto& p : Profiles()) {
    if (!p->ephemeral && p->name == name && p->profile) {
      return p.get();
    }
  }
  // Loading a profile reads its directory; Chrome's UI thread forbids that
  // by default (DisallowUnresponsiveTasks) because it never loads one here.
  // ponytail: the ForTesting scope is the only public one; the real fix is
  // ProfileManager::CreateProfileAsync (an ABI change; risk 5).
  base::ScopedAllowBlockingForTesting allow_blocking;
  base::ElapsedTimer timer;
  ProfileManager* manager = g_browser_process->profile_manager();
  Profile* profile =
      manager->GetProfile(manager->user_data_dir().AppendASCII(name));
  if (!profile) {
    return nullptr;
  }
  LOG(INFO) << "baa: profile_open " << name << " took "
            << timer.Elapsed().InMilliseconds() << " ms";
  baa::InstallAppModalFactory();
  baa::WatchDownloads(profile);
  auto owned = std::make_unique<BaaProfile>(profile, /*ephemeral=*/false);
  owned->name = name;
  Profiles().push_back(std::move(owned));
  return Profiles().back().get();
}

BaaProfile* baa_profile_open_ephemeral(void) {
  baa::WatchTermination();
  if (!g_browser_process || !g_browser_process->profile_manager() ||
      baa_is_quitting()) {
    return nullptr;
  }
  base::ScopedAllowBlockingForTesting allow_blocking;
  // The Default profile, never GetLastUsedProfile(): Chrome moves that to
  // whatever Space was active.
  ProfileManager* manager = g_browser_process->profile_manager();
  Profile* base =
      manager->GetProfile(manager->user_data_dir().AppendASCII(chrome::kInitialProfile));
  if (!base) {
    return nullptr;
  }
  Profile* profile = base->GetOffTheRecordProfile(
      Profile::OTRProfileID::CreateUnique("baa::space"),
      /*create_if_needed=*/true);
  if (!profile) {
    return nullptr;
  }
  baa::InstallAppModalFactory();
  baa::WatchDownloads(profile);
  Profiles().push_back(std::make_unique<BaaProfile>(profile, /*ephemeral=*/true));
  return Profiles().back().get();
}

void baa_profile_close(BaaProfile* profile) {
  if (!AliveProfile(profile)) {
    return;
  }
  Profile* chrome_profile = profile->profile;
  std::vector<tabs::TabInterface*> doomed;
  for (auto& [key, value] : Tabs()) {
    content::WebContents* wc = value->web_contents();
    if (wc && Profile::FromBrowserContext(wc->GetBrowserContext()) == chrome_profile) {
      value->closing_by_baa = true;
      doomed.push_back(key);
    }
  }
  for (tabs::TabInterface* tab : doomed) {
    if (!Tabs().count(tab)) {
      continue;
    }
    BrowserWindowInterface* browser = tab->GetBrowserWindowInterface();
    int index = browser ? browser->GetTabStripModel()->GetIndexOfTab(tab) : -1;
    if (index >= 0) {
      browser->GetTabStripModel()->DetachAndDeleteWebContentsAt(index);
    }
  }
  for (size_t i = 0; i < BareTabs().size();) {
    BaaTab* bare = BareTabs()[i].get();
    content::WebContents* wc = bare->web_contents();
    if (wc && Profile::FromBrowserContext(wc->GetBrowserContext()) == chrome_profile) {
      baa_tab_close(bare);
    } else {
      ++i;
    }
  }
  bool ephemeral = profile->ephemeral;
  std::erase_if(Profiles(), [&](auto& p) { return p.get() == profile; });
  if (chrome_profile) {
    // The tabs are gone but their Browsers close by a posted task, and
    // destroying an OTR profile while one lives CHECKs. Ours go now.
    baa::DestroyBrowsersOf(chrome_profile);
    if (ephemeral) {
      ProfileDestroyer::DestroyOTRProfileWhenAppropriate(chrome_profile);
    }
  }
}

// ---- C ABI: tabs ---------------------------------------------------------------

BaaTab* baa_tab_create(BaaProfile* profile, const char* url) {
  if (!AliveProfile(profile) || baa_is_quitting()) {
    return nullptr;
  }
  BrowserWindowInterface* browser = profile->GetBrowser();
  if (!browser) {
    return nullptr;
  }
  g_creating = true;
  content::WebContents* wc =
      chrome::AddAndReturnTabAt(browser, GURL(url), -1, /*foreground=*/false);
  g_creating = false;
  BaaTab* tab = wc ? TabOf(wc) : nullptr;
  // Navigate sends a URL for an OTR profile to the parent's Browser; a tab
  // in another profile than asked for is not ours to hand out.
  if (tab && Profile::FromBrowserContext(wc->GetBrowserContext()) != profile->profile) {
    baa_tab_close(tab);
    return nullptr;
  }
  return tab;
}

namespace {
// Null unless the handle is live and has contents.
content::WebContents* Contents(BaaTab* tab) {
  return AliveTab(tab) ? tab->web_contents() : nullptr;
}
}  // namespace

void baa_tab_close(BaaTab* tab) {
  if (!AliveTab(tab)) {
    return;
  }
  tab->closing_by_baa = true;
  if (!tab->tab) {
    // A popup: destroying what holds the contents closes it.
    base::OnceClosure close = std::move(tab->close_bare);
    baa::DropPermissionsOf(tab);
    baa::DropMenusOf(tab);
    tab->Detach();
    std::erase_if(BareTabs(), [&](auto& t) { return t.get() == tab; });
    if (close) {
      std::move(close).Run();
    }
    return;
  }
  BrowserWindowInterface* browser = tab->tab->GetBrowserWindowInterface();
  if (!browser) {
    return;
  }
  TabStripModel* model = browser->GetTabStripModel();
  int index = model->GetIndexOfTab(tab->tab);
  if (index >= 0) {
    model->DetachAndDeleteWebContentsAt(index);
  }
}

bool baa_tab_get_state(BaaTab* tab, BaaTabState* state) {
  if (!Contents(tab)) {
    return false;
  }
  *state = tab->Snapshot(tab->url_s, tab->title_s, tab->icon_s);
  return true;
}

void baa_tab_load_url(BaaTab* tab, const char* url) {
  if (content::WebContents* wc = Contents(tab)) {
    content::NavigationController::LoadURLParams params{GURL(url)};
    params.transition_type = ui::PAGE_TRANSITION_TYPED;
    wc->GetController().LoadURLWithParams(params);
  }
}

void baa_tab_reload(BaaTab* tab) {
  if (content::WebContents* wc = Contents(tab)) {
    wc->GetController().Reload(content::ReloadType::NORMAL, true);
  }
}

void baa_tab_stop(BaaTab* tab) {
  if (content::WebContents* wc = Contents(tab)) {
    wc->Stop();
  }
}

void baa_tab_go_to_offset(BaaTab* tab, int offset) {
  if (content::WebContents* wc = Contents(tab)) {
    if (wc->GetController().CanGoToOffset(offset)) {
      wc->GetController().GoToOffset(offset);
    }
  }
}

int baa_tab_history(BaaTab* tab, BaaHistoryVisitor visit, void* ctx) {
  content::WebContents* wc = Contents(tab);
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
  content::WebContents* wc = Contents(tab);
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
  if (content::WebContents* wc = Contents(tab)) {
    if (auto* zoom = zoom::ZoomController::FromWebContents(wc)) {
      zoom->SetZoomLevel(blink::ZoomFactorToZoomLevel(factor));
    }
  }
}

void baa_tab_find(BaaTab* tab, const char* text, bool forward) {
  content::WebContents* wc = Contents(tab);
  if (!wc) {
    return;
  }
  auto* helper = find_in_page::FindTabHelper::FromWebContents(wc);
  if (!helper) {
    return;
  }
  std::u16string query = base::UTF8ToUTF16(text);
  if (query.empty()) {
    tab->find_text.clear();
    helper->StopFinding(find_in_page::SelectionAction::kClear);
    return;
  }
  tab->find_text = query;
  helper->StartFinding(query, forward, /*case_sensitive=*/false,
                       /*find_match=*/true);
}

void baa_tab_print(BaaTab* tab) {
  if (content::WebContents* wc = Contents(tab)) {
    printing::StartBasicPrint(wc);
  }
}

void baa_tab_set_muted(BaaTab* tab, bool muted) {
  if (content::WebContents* wc = Contents(tab)) {
    SetTabAudioMuted(wc, muted, TabMutedReason::kAudioIndicator, "");
  }
}

void baa_tab_activate(BaaTab* tab) {
  if (!Contents(tab)) {
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
  if (content::WebContents* wc = Contents(tab)) {
    wc->Focus();
  }
}

void baa_dialog_answer(uint64_t id, bool ok, const char* text) {
  auto it = Dialogs().find(id);
  if (it == Dialogs().end()) {
    baa::AnswerAppModal(id, ok, base::UTF8ToUTF16(text ? text : ""));
    return;
  }
  // Forgotten before the callback runs: the manager answers by closing the
  // dialog, which must not read as the page going away.
  std::unique_ptr<DialogView> view = std::move(it->second);
  Dialogs().erase(it);
  view->MarkAnswered();
  auto callback = view->TakeCallback();
  std::move(callback).Run(ok, base::UTF8ToUTF16(text ? text : ""));
}

namespace baa {

bool SetPrefBool(Profile* profile, const char* name, bool value) {
  PrefService* prefs = profile ? profile->GetPrefs() : nullptr;
  const PrefService::Preference* pref = prefs ? prefs->FindPreference(name) : nullptr;
  if (!pref || !pref->IsUserModifiable() || !pref->GetValue()->is_bool()) {
    return false;
  }
  prefs->SetBoolean(name, value);
  return true;
}

}  // namespace baa

void baa_profile_set_pref_bool(BaaProfile* profile, const char* name, bool value) {
  if (AliveProfile(profile)) {
    LOG(INFO) << "baa: set_pref_bool " << name << " = " << value << " -> "
              << baa::SetPrefBool(profile->profile, name, value);
  }
}

namespace baa {

uint64_t NextId() {
  return g_next_id++;
}

BaaTab* TabFor(content::WebContents* contents) {
  return contents ? TabOf(contents) : nullptr;
}

bool TabAnnounced(BaaTab* tab) {
  return tab && tab->reported;
}

bool TabAlive(BaaTab* tab) {
  return AliveTab(tab);
}

content::WebContents* ContentsOf(BaaTab* tab) {
  return AliveTab(tab) ? tab->web_contents() : nullptr;
}

Profile* ChromeProfileOf(BaaProfile* profile) {
  return AliveProfile(profile) ? profile->profile.get() : nullptr;
}

BaaProfile* ProfileFor(Profile* profile) {
  return ProfileOf(profile);
}

BrowserWindowInterface* ExistingBrowserOf(BaaProfile* profile) {
  return AliveProfile(profile) && profile->browser && !BrowserIsClosing(profile->browser)
             ? profile->browser.get()
             : nullptr;
}

BaaTab* CreateBareTab(content::WebContents* contents, base::OnceClosure close) {
  auto owned = std::make_unique<BaaTab>(nullptr);
  BaaTab* tab = owned.get();
  tab->bare = true;
  tab->close_bare = std::move(close);
  BareTabs().push_back(std::move(owned));
  tab->Attach(contents);
  return tab;
}

// The holder closed the page itself (a popup's window.close(), its extension
// unloaded): tab_closed once, then the handle is dead.
void BareTabClosedByChrome(BaaTab* tab) {
  if (!AliveTab(tab)) {
    return;
  }
  tab->close_bare = base::OnceClosure();
  ReportClosed(tab);
  tab->Detach();
  std::erase_if(BareTabs(), [&](auto& t) { return t.get() == tab; });
}

// Quit: popups are not in any tab strip, so nothing else closes them, and a
// BrowserContext may not die with a WebContents alive.
void CloseBareTabsForQuit() {
  while (!BareTabs().empty()) {
    BaaTab* tab = BareTabs().front().get();
    base::OnceClosure close = std::move(tab->close_bare);
    ReportClosed(tab);
    tab->Detach();
    BareTabs().erase(BareTabs().begin());
    if (close) {
      std::move(close).Run();
    }
  }
}

// Sends the first state of a page nobody told the app about yet.
void SendTabState(BaaTab* tab) {
  if (AliveTab(tab)) {
    tab->SendState();
  }
}

}  // namespace baa

void baa_tab_set_init_script(BaaTab* tab, const char* script) {
  if (AliveTab(tab)) {
    tab->init_script = base::UTF8ToUTF16(script ? script : "");
  }
}

void baa_tab_set_color_scheme(BaaTab* tab, int scheme) {
  if (!AliveTab(tab)) {
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

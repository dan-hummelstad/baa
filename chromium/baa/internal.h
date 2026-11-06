// Shared declarations for //baa. Spec: PLAN-chromium-phase3.md "C++ layout".
#ifndef BAA_INTERNAL_H_
#define BAA_INTERNAL_H_

#include <memory>
#include <string>
#include <vector>

#include "baa/baa.h"
#include "base/functional/callback.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "chrome/browser/tab_list/tab_removed_reason.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "content/public/browser/download_manager.h"
#include "content/public/browser/web_contents.h"

class ExtensionsContainer;
class Profile;
struct BaaTab;
struct BaaProfile;
namespace input {
struct NativeWebKeyboardEvent;
}

namespace baa {

// The callbacks from baa_install_app; every one is set (checked there).
const BaaCallbacks& Callbacks();
void* CallbackContext();

// app_mac.mm: the one-time installs (termination watch, window hook, context
// menu hook, app-modal factory). Idempotent; called once Chrome is up.
void EnsureStarted();
void InstallContextMenuHook();  // menu.mm
void InstallAppModalFactory();  // dialogs: after Chrome installed its own

// ---- tab.mm and profile.mm: handles and lookups the other files share. A BaaTab is any page
// the app shows: a tab in a Browser, a popup, a DevTools front-end.
// Tab ids, asks, downloads and tab-group keys share one counter; ids are never reused.
uint64_t NextId();
uint64_t IdOf(BaaTab* tab);  // 0 for null
BaaTab* TabFromId(uint64_t id);  // null if closed
BaaTab* TabFor(content::WebContents* contents);  // null if not a baa page
bool TabAnnounced(BaaTab* tab);  // the app knows the handle
bool TabAlive(BaaTab* tab);
// What long-lived holders keep instead of a raw handle: null once the tab is gone.
base::WeakPtr<BaaTab> WeakTab(BaaTab* tab);
content::WebContents* ContentsOf(BaaTab* tab);  // null if gone
tabs::TabInterface* TabInterfaceOf(BaaTab* tab);  // null for a popup or a gone tab
Profile* ChromeProfileOf(BaaProfile* profile);  // null if gone
BaaProfile* ProfileFor(Profile* profile);       // null if baa never opened it
BrowserWindowInterface* BrowserOf(BaaProfile* profile);  // made on first need
BrowserWindowInterface* ExistingBrowserOf(BaaProfile* profile);
// What a tab inserted while one of these lives is: the app made it itself, so
// it is not reported as a tab Chrome made, and `*out` is the BaaTab it got
// (null if none). The DevTools kind also makes the DevTools Browser's window a
// hidden baa one.
enum class InsertionKind { kBaa, kDevtoolsFront };
struct ScopedInsertion {
  ScopedInsertion(InsertionKind kind, BaaTab** out);
  ~ScopedInsertion();
  ScopedInsertion(const ScopedInsertion&) = delete;
  ScopedInsertion& operator=(const ScopedInsertion&) = delete;

  const InsertionKind kind;
  BaaTab** const out;
  ScopedInsertion* const prev;
};
bool DevtoolsDocking();  // a kDevtoolsFront insertion is in progress

// A page that is not in a tab strip. `close` destroys what holds the contents.
BaaTab* CreateBareTab(content::WebContents* contents, base::OnceClosure close);
void ReapplyColorSchemes(Profile* profile);  // tab.mm: after the profile's choice changed
void CloseTabsOf(Profile* profile);  // baa_profile_close: every page of it, now
void BareTabClosedByChrome(BaaTab* tab);
void CloseBareTabsForQuit();
void DestroyAllPopups();  // extensions.mm: now, with their WebContents
void SendTabState(BaaTab* tab);

// ---- dialogs.mm
void AttachDialogs(content::WebContents* wc);
// A tab that left our windows: pending dialogs answer "no", Chrome's come back.
void ReturnDialogsToChrome(BaaTab* tab, content::WebContents* wc);
void ShowPendingDialogsOf(BaaTab* tab);  // asked before the app knew the tab

// ---- asks.mm: Chrome's own dialogs drawn by the app
void InstallDialogHooks();  // the g_baa_* hooks of patches/baa/dialog-hooks.patch
void AttachTabDialogs(content::WebContents* wc);  // the hung-page prompt
void RestoreTabDialogs(content::WebContents* wc);

// ---- permissions.mm / menu.mm / devtools.mm
void AttachPermissions(content::WebContents* wc);
// Back to Chrome's own prompt (a tab that left our windows).
void RestoreChromePermissions(content::WebContents* wc);
void ForgetDevtoolsOf(BaaTab* tab);
void SetDockSide(BaaTab* front, const char* side);  // tab.mm: for the front's dock script

// downloads.mm: observes a profile's DownloadManager for as long as it lives
// (BaaProfile owns one).
class DownloadWatch : public content::DownloadManager::Observer {
 public:
  explicit DownloadWatch(Profile* profile);
  ~DownloadWatch() override;
  void OnDownloadCreated(content::DownloadManager*, download::DownloadItem* item) override;
  void ManagerGoingDown(content::DownloadManager*) override;

 private:
  raw_ptr<content::DownloadManager> manager_;
};

// window.mm -> tab.mm: what the window's tab strip observer found.
void OnTabInserted(TabStripModel* model,
                   tabs::TabInterface* tab,
                   int index,
                   bool background);
void OnTabRemoved(tabs::TabInterface* tab, TabRemovedReason reason);
void OnTabReplaced(tabs::TabInterface* tab, content::WebContents* new_contents);
// Reports a key to the app. Synchronous.
bool OfferKey(const input::NativeWebKeyboardEvent& event, bool after_page);

// window.mm: Browser queries and teardown.
bool IsOurBrowser(BrowserWindowInterface* browser);
// A Browser whose window was asked to close (it is on its way out).
bool BrowserIsClosing(BrowserWindowInterface* browser);
// Destroys, now, every Browser of `profile` that has a baa window (their tabs
// are already closed).
void DestroyBrowsersOf(Profile* profile);
std::unique_ptr<ExtensionsContainer> MakeExtensionsContainer(BrowserWindowInterface* browser);  // extensions.mm
// ---- requests.mm: what the app answers later by id. Asks (dialogs, permissions) and context
// menus register themselves here; baa_answer and baa_context_menu_done look the id up, and
// request_closed leaves from WithdrawRequest and DropRequestsOf only. Not owning: each subclass
// has its own lifetime and is unregistered silently by ~Request.
class Request {
 public:
  enum class Role { kDialog, kPermission, kMenu };
  Request(uint64_t tab, Role role);  // `tab` is 0 for a page the app has no handle for
  virtual ~Request();
  Request(const Request&) = delete;
  Request& operator=(const Request&) = delete;
  uint64_t id() const { return id_; }
  uint64_t tab() const { return tab_; }
  Role role() const { return role_; }
  // Idempotent. Call before running what could close the ask, so that closing reads as answered.
  void Unregister();
  virtual void Answer(const BaaAnswer&) {}  // asks
  virtual void Command(int command) {}      // menus
  // The tab is gone or leaves our windows (already unregistered): let go of it, never touch the
  // app. Delete this if nothing else owns it.
  virtual void Drop() = 0;

 private:
  const uint64_t id_;
  const uint64_t tab_;
  const Role role_;
};
Request* FindRequest(uint64_t id);
// Chrome dropped the ask: unregisters it and sends request_closed if the app knows the tab.
void WithdrawRequest(uint64_t id);
// Forget(): every request of the tab is unregistered and dropped; asks (not menus) are reported
// closed when `tell`. `menus_only` is the tab keeping its contents (a replace).
void DropRequestsOf(uint64_t tab, bool tell, bool menus_only = false);
std::vector<uint64_t> DialogsOf(uint64_t tab);

// Calls answered later by id (clear_done, load_unpacked_done), each exactly once,
// also when the profile closes first.
enum class RequestKind { kClear, kLoadUnpacked };
void BeginRequest(RequestKind kind, uint64_t id, Profile* profile);
// Answers the app unless the request already was (a closed profile answered it); call from a task.
void FinishRequest(RequestKind kind, uint64_t id, bool ok, const std::string& message);
void FinishRequestLater(RequestKind kind, uint64_t id, bool ok, const std::string& message);
void AnswerPendingOf(Profile* profile);  // baa_profile_close

// An off-the-record profile has no defaults of its own (content settings, search engines,
// zoom): they live on its parent.
Profile* DefaultsProfile(Profile* profile);
// profile.mm: installs the colour scheme hook of patches/baa/color-scheme-hook.patch.
void InstallColorSchemeHook();
// 0 follows the system, 1 light, 2 dark (anything else is the system's).
enum class ColorScheme { kSystem, kLight, kDark };
inline ColorScheme ColorSchemeOf(int value) {
  return value == 1 ? ColorScheme::kLight : value == 2 ? ColorScheme::kDark : ColorScheme::kSystem;
}

// autocomplete.mm: a profile's omnibox controller (BaaProfile owns one, made on first use).
class Autocomplete {
 public:
  explicit Autocomplete(Profile* profile);
  ~Autocomplete();
  void Start(uint64_t id, const std::string& text);  // "" only stops

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

// profile.mm: sets a bool pref if the profile registers it and the user may change it.
bool SetPrefBool(Profile* profile, const char* name, bool value);

// Makes `browser`'s window the one Chrome treats as active.
void ActivateBrowser(BrowserWindowInterface* browser);

}  // namespace baa

#endif  // BAA_INTERNAL_H_

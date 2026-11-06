// Shared declarations for //baa. Spec: PLAN-chromium-phase3.md "C++ layout".
#ifndef BAA_INTERNAL_H_
#define BAA_INTERNAL_H_

#include <memory>
#include <string>

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
uint64_t NextId();  // dialogs, permission asks and menus share one counter
BaaTab* TabFor(content::WebContents* contents);  // null if not a baa page
bool TabAnnounced(BaaTab* tab);  // the app knows the handle
bool TabAlive(BaaTab* tab);
// What long-lived holders keep instead of a raw handle: null once the tab is gone.
base::WeakPtr<BaaTab> WeakTab(BaaTab* tab);
content::WebContents* ContentsOf(BaaTab* tab);  // null if gone
Profile* ChromeProfileOf(BaaProfile* profile);  // null if gone
BaaProfile* ProfileFor(Profile* profile);       // null if baa never opened it
BrowserWindowInterface* BrowserOf(BaaProfile* profile);  // made on first need
BrowserWindowInterface* ExistingBrowserOf(BaaProfile* profile);
// A page that is not in a tab strip. `close` destroys what holds the contents.
BaaTab* CreateBareTab(content::WebContents* contents, base::OnceClosure close);
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
void DropDialogsOf(BaaTab* tab, bool tell);

// ---- permissions.mm / menu.mm / devtools.mm
void AttachPermissions(content::WebContents* wc);
void DropPermissionsOf(BaaTab* tab, bool tell);
// Back to Chrome's own prompt (a tab that left our windows).
void RestoreChromePermissions(content::WebContents* wc);
void DropMenusOf(BaaTab* tab);
bool DevtoolsDocking();
void NoteDevtoolsFront(BaaTab* front);
void ForgetDevtoolsOf(BaaTab* tab);

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
// profile.mm: sets a bool pref if the profile registers it and the user may change it.
bool SetPrefBool(Profile* profile, const char* name, bool value);

// Makes `browser`'s window the one Chrome treats as active.
void ActivateBrowser(BrowserWindowInterface* browser);

}  // namespace baa

#endif  // BAA_INTERNAL_H_

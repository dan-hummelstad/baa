// NSApp, start-up and shutdown. See PLAN-chromium-phase2.md (run ownership)
// and PLAN-chromium-phase3.md.

#import <Cocoa/Cocoa.h>

#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "baa/window.h"
#include "base/command_line.h"
#include "base/functional/bind.h"
#include "base/logging.h"
#include "base/no_destructor.h"
#include "chrome/browser/browser_process.h"
#include "chrome/browser/lifetime/browser_shutdown.h"
#include "chrome/common/chrome_constants.h"
#include "chrome/common/pref_names.h"
#include "chrome/browser/ui/extensions/extension_install_ui.h"
#include "components/prefs/pref_change_registrar.h"
#include "components/prefs/pref_service.h"
#include "chrome/browser/lifetime/termination_notification.h"
#import "chrome/browser/chrome_browser_application_mac.h"
#include "chrome/browser/ui/views/frame/baa_browser_window_hook.h"
#include "content/public/browser/render_frame_host.h"

extern "C" int ChromeMain(int argc, const char** argv);

namespace {
std::vector<const char*>* g_argv = nullptr;
int g_exit_code = 0;
BaaCallbacks g_callbacks = {};
void* g_ctx = nullptr;
bool g_terminating = false;
PrefChangeRegistrar& PinRegistrar() {
  static base::NoDestructor<PrefChangeRegistrar> pin;
  return *pin;
}
}  // namespace

namespace baa {
const BaaCallbacks& Callbacks() {
  return g_callbacks;
}
void* CallbackContext() {
  return g_ctx;
}

void EnsureStarted() {
  static bool done = false;
  if (done) {
    return;
  }
  done = true;
  // Once Chrome starts terminating, no new Browsers or tabs are made: Browsers
  // close through BrowserCloseManager (our windows' Close), and later calls from
  // the UI on a gone handle do nothing.
  static base::NoDestructor<base::CallbackListSubscription> on_terminating(
      browser_shutdown::AddAppTerminatingCallback(
          base::BindOnce([] {
        g_terminating = true;
        // The registrar must be gone before local state is.
        PinRegistrar().RemoveAll();
        CloseBareTabsForQuit();
      })));
  // The post-install dialog dereferences a null BrowserView when it closes
  // (extension_install_ui_desktop.cc); the install prompt itself still shows.
  static base::NoDestructor<base::AutoReset<bool>> no_install_ui(
      ExtensionInstallUI::disable_ui_for_tests(true));
  // Keeps Chrome's "last used profile" on Default: it follows whichever Space
  // was active, and a deleted Space's directory would be recreated by the next
  // start-up.
  PrefService* local_state = g_browser_process->local_state();
  auto reset = [](PrefService* local_state) {
    if (local_state->GetString(prefs::kProfileLastUsed) != chrome::kInitialProfile) {
      local_state->SetString(prefs::kProfileLastUsed, chrome::kInitialProfile);
    }
  };
  PinRegistrar().Init(local_state);
  PinRegistrar().Add(prefs::kProfileLastUsed, base::BindRepeating(reset, local_state));
  reset(local_state);
  InstallContextMenuHook();
  InstallAppModalFactory();
}
}  // namespace baa

// On macOS Chromium's message pump runs tasks only while RunLoop::Run is on
// the stack, so Chromium owns [NSApp run]: the UI's call becomes ChromeMain,
// whose RunLoop calls [NSApp run] again and gets the real loop.
//
// gpui keeps its platform pointer in an NSApp ivar named `platform`
// (gpui_macos's GPUIApplication) and fetches NSApp through +sharedApplication,
// which hands back this instance.
@interface BaaApplication : BrowserCrApplication {
  void* platform;
}
@end

@implementation BaaApplication {
  BOOL _chromiumStarted;
}

- (void)run {
  if (_chromiumStarted) {
    [super run];
    return;
  }
  _chromiumStarted = YES;
  // Process-wide, and meant for embedders (WebView, Cast): without it
  // RenderFrameHost::ExecuteJavaScript CHECKs outside WebUI. baa_tab_eval needs it.
  content::RenderFrameHost::AllowInjectingJavaScript();
  // Before ChromeMain: every Browser Chrome makes gets a baa::Window.
  g_baa_create_browser_window = &baa::Window::Create;
  g_exit_code = ChromeMain(static_cast<int>(g_argv->size()), g_argv->data());
}

@end

int baa_abi_version(void) {
  return BAA_ABI_VERSION;
}

void baa_install_app(int argc,
                     const char** argv,
                     const BaaCallbacks* callbacks,
                     void* ctx) {
  // Every callback is called unguarded.
#define REQUIRE(f) CHECK(callbacks->f) << "BaaCallbacks." #f " is NULL"
  REQUIRE(key);
  REQUIRE(tab_created);
  REQUIRE(tab_state);
  REQUIRE(tab_favicon);
  REQUIRE(dialog);
  REQUIRE(dialog_closed);
  REQUIRE(tab_closed);
  REQUIRE(permission);
  REQUIRE(permission_closed);
  REQUIRE(context_menu);
  REQUIRE(download);
  REQUIRE(resized);
  REQUIRE(page_message);
  REQUIRE(fetch_done);
  REQUIRE(clear_done);
  REQUIRE(load_unpacked_done);
#undef REQUIRE
  g_argv = new std::vector<const char*>(argv, argv + argc);
  g_callbacks = *callbacks;
  g_ctx = ctx;
  // +[BrowserCrApplication initialize] reads the command line; content's own
  // Init later is a no-op.
  base::CommandLine::Init(argc, argv);
  [BaaApplication sharedApplication];
}

bool baa_is_quitting(void) {
  return g_terminating || browser_shutdown::IsTryingToQuit() ||
         browser_shutdown::HasShutdownStarted();
}

int baa_exit_code(void) {
  return g_exit_code;
}

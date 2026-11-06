// NSApp, start-up and shutdown. See PLAN-chromium-phase2.md (run ownership)
// and PLAN-chromium-phase3.md.

#import <Cocoa/Cocoa.h>

#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/command_line.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
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
}  // namespace

namespace baa {
const BaaCallbacks& Callbacks() {
  return g_callbacks;
}
void* CallbackContext() {
  return g_ctx;
}
bool Terminating() {
  return g_terminating;
}

// Pages and Browsers go when Chrome starts terminating, before profiles are
// torn down (a page outliving its profile trips shutdown checks); later calls
// from the UI on a gone handle do nothing. Called once Chrome is up.
void WatchTermination() {
  static base::NoDestructor<base::CallbackListSubscription> on_terminating(
      browser_shutdown::AddAppTerminatingCallback(
          base::BindOnce([] { g_terminating = true; })));
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
  g_argv = new std::vector<const char*>(argv, argv + argc);
  g_callbacks = *callbacks;
  g_ctx = ctx;
  // +[BrowserCrApplication initialize] reads the command line; content's own
  // Init later is a no-op.
  base::CommandLine::Init(argc, argv);
  [BaaApplication sharedApplication];
}

int baa_exit_code(void) {
  return g_exit_code;
}

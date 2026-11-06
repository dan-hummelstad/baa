// Phase 2 spike: NSApp, start-up and one page. See PLAN-chromium-phase2.md.

#import <Cocoa/Cocoa.h>

#include <memory>
#include <vector>

#include "baa/baa.h"
#import "chrome/browser/chrome_browser_application_mac.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/profiles/profile_manager.h"
#include "components/input/native_web_keyboard_event.h"
#include "content/public/browser/keyboard_event_processing_result.h"
#include "content/public/browser/navigation_controller.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_delegate.h"
#include "third_party/blink/public/common/input/web_input_event.h"
#include "ui/base/page_transition_types.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "url/gurl.h"

extern "C" int ChromeMain(int argc, const char** argv);

namespace {
std::vector<const char*>* g_argv = nullptr;
int g_exit_code = 0;
BaaKeyFn g_key = nullptr;
void* g_key_ctx = nullptr;
}  // namespace

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
  g_exit_code = ChromeMain(static_cast<int>(g_argv->size()), g_argv->data());
}

@end

void baa_install_app(int argc,
                     const char** argv,
                     BaaKeyFn key,
                     void* key_ctx) {
  g_argv = new std::vector<const char*>(argv, argv + argc);
  g_key = key;
  g_key_ctx = key_ctx;
  [BaaApplication sharedApplication];
}

int baa_exit_code(void) {
  return g_exit_code;
}

// ponytail: a bare WebContents with its own delegate; phase 3 puts pages in a
// Browser's TabStripModel and the key hooks move to baa::ui::Window.
struct BaaWebContents : content::WebContentsDelegate {
  std::unique_ptr<content::WebContents> contents;

  content::KeyboardEventProcessingResult PreHandleKeyboardEvent(
      content::WebContents*,
      const input::NativeWebKeyboardEvent& event) override {
    return Offer(event, false)
               ? content::KeyboardEventProcessingResult::HANDLED
               : content::KeyboardEventProcessingResult::NOT_HANDLED;
  }

  bool HandleKeyboardEvent(
      content::WebContents*,
      const input::NativeWebKeyboardEvent& event) override {
    return Offer(event, true);
  }

  // The app's shortcuts get the first look, as with CEF's on_pre_key_event;
  // Escape too, so an overlay over the page can close.
  bool Offer(const input::NativeWebKeyboardEvent& event, bool after_page) {
    if (!g_key || event.GetType() != blink::WebInputEvent::Type::kRawKeyDown) {
      return false;
    }
    const int shortcut = blink::WebInputEvent::kMetaKey |
                         blink::WebInputEvent::kControlKey;
    if (!(event.GetModifiers() & shortcut) &&
        event.windows_key_code != ui::VKEY_ESCAPE) {
      return false;
    }
    NSEvent* ns_event = event.os_event.Get();
    if (!ns_event) {
      return false;
    }
    return g_key(g_key_ctx, this, (__bridge void*)ns_event, after_page);
  }
};

BaaWebContents* baa_web_contents_create(const char* url) {
  auto* page = new BaaWebContents;
  page->contents = content::WebContents::Create(
      content::WebContents::CreateParams(ProfileManager::GetLastUsedProfile()));
  page->contents->SetDelegate(page);
  content::NavigationController::LoadURLParams params{GURL(url)};
  params.transition_type = ui::PAGE_TRANSITION_TYPED;
  page->contents->GetController().LoadURLWithParams(params);
  return page;
}

void* baa_web_contents_view(BaaWebContents* page) {
  return (__bridge void*)page->contents->GetNativeView().GetNativeNSView();
}

void baa_web_contents_focus(BaaWebContents* page) {
  page->contents->Focus();
}

void baa_web_contents_edit(BaaWebContents* page, int command) {
  content::WebContents* c = page->contents.get();
  switch (command) {
    case BAA_EDIT_COPY:
      c->Copy();
      break;
    case BAA_EDIT_CUT:
      c->Cut();
      break;
    case BAA_EDIT_PASTE:
      c->Paste();
      break;
    case BAA_EDIT_SELECT_ALL:
      c->SelectAll();
      break;
    case BAA_EDIT_UNDO:
      c->Undo();
      break;
    case BAA_EDIT_REDO:
      c->Redo();
      break;
  }
}

void baa_web_contents_destroy(BaaWebContents* page) {
  delete page;
}

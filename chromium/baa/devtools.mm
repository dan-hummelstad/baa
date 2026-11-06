// DevTools docked in the app (a hidden TYPE_DEVTOOLS Browser holds the
// front-end) and in Chrome's own window. PLAN-chromium-phase4.md step 5.

#import <Cocoa/Cocoa.h>

#include <map>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/no_destructor.h"
#include "chrome/browser/devtools/devtools_window.h"
#include "content/public/browser/render_frame_host.h"
#include "content/public/browser/web_contents.h"

namespace baa {
namespace {

// inspected page -> its docked front-end, for closing it before reopening.
std::map<BaaTab*, BaaTab*>& Fronts() {
  static base::NoDestructor<std::map<BaaTab*, BaaTab*>> fronts;
  return *fronts;
}

}  // namespace

void ForgetDevtoolsOf(BaaTab* tab) {
  Fronts().erase(tab);
  for (auto it = Fronts().begin(); it != Fronts().end();) {
    it = it->second == tab ? Fronts().erase(it) : std::next(it);
  }
}

}  // namespace baa

BaaTab* baa_tab_open_devtools(BaaTab* tab, bool inspect, int x, int y) {
  content::WebContents* wc = baa::ContentsOf(tab);
  if (!wc || baa_is_quitting()) {
    return nullptr;
  }
  if (auto it = baa::Fronts().find(tab); it != baa::Fronts().end()) {
    BaaTab* old = it->second;
    baa::Fronts().erase(it);
    if (baa::TabAlive(old)) {
      baa_tab_close(old);
    }
  }
  BaaTab* front;
  {
    baa::ScopedInsertion docking(baa::InsertionKind::kDevtoolsFront, &front);
    if (inspect) {
      DevToolsWindow::InspectElement(wc->GetPrimaryMainFrame(), x, y);
    } else {
      DevToolsWindow::OpenDevToolsWindow(wc, DevToolsToggleAction::Show(),
                                         DevToolsOpenedByAction::kUnknown);
    }
  }
  if (!front && inspect) {
    // The window may be made without the inspect action having run: open it,
    // then inspect inside the open window.
    {
      baa::ScopedInsertion docking(baa::InsertionKind::kDevtoolsFront, &front);
      DevToolsWindow::OpenDevToolsWindow(wc, DevToolsToggleAction::Show(),
                                         DevToolsOpenedByAction::kUnknown);
    }
    DevToolsWindow::InspectElement(wc->GetPrimaryMainFrame(), x, y);
  }
  if (front) {
    baa::Fronts()[tab] = front;
  }
  return front;
}

void baa_tab_show_devtools_window(BaaTab* tab, bool inspect, int x, int y) {
  content::WebContents* wc = baa::ContentsOf(tab);
  if (!wc || baa_is_quitting()) {
    return;
  }
  if (inspect) {
    DevToolsWindow::InspectElement(wc->GetPrimaryMainFrame(), x, y);
  } else {
    DevToolsWindow::OpenDevToolsWindow(wc, DevToolsToggleAction::Show(),
                                       DevToolsOpenedByAction::kUnknown);
  }
}

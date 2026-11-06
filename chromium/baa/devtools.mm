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

BaaTabId baa_tab_open_devtools(BaaTabId tab_id, bool inspect, int x, int y, int side) {
  BaaTab* tab = baa::TabFromId(tab_id);
  content::WebContents* wc = baa::ContentsOf(tab);
  if (!wc || baa_is_quitting()) {
    return 0;
  }
  if (auto it = baa::Fronts().find(tab); it != baa::Fronts().end()) {
    BaaTab* old = it->second;
    baa::Fronts().erase(it);
    if (baa::TabAlive(old)) {
      baa_tab_close(baa::IdOf(old));
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
    baa::SetDockSide(front, side == BAA_DOCK_BOTTOM ? "bottom" : side == BAA_DOCK_LEFT ? "left" : "right");
  }
  return baa::IdOf(front);
}

void baa_tab_show_devtools_window(BaaTabId tab_id, bool inspect, int x, int y) {
  BaaTab* tab = baa::TabFromId(tab_id);
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

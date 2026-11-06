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

bool g_docking = false;
BaaTab* g_front = nullptr;

// inspected page -> its docked front-end, for closing it before reopening.
std::map<BaaTab*, BaaTab*>& Fronts() {
  static base::NoDestructor<std::map<BaaTab*, BaaTab*>> fronts;
  return *fronts;
}

struct ScopedDocking {
  ScopedDocking() {
    g_docking = true;
    g_front = nullptr;
  }
  ~ScopedDocking() { g_docking = false; }
};

}  // namespace

// While set, the window hook gives a TYPE_DEVTOOLS Browser a hidden baa
// window instead of Chrome's views window, and its tab is not announced.
bool DevtoolsDocking() {
  return g_docking;
}

void NoteDevtoolsFront(BaaTab* front) {
  g_front = front;
}

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
  {
    baa::ScopedDocking docking;
    if (inspect) {
      DevToolsWindow::InspectElement(wc->GetPrimaryMainFrame(), x, y);
    } else {
      DevToolsWindow::OpenDevToolsWindow(wc, DevToolsToggleAction::Show(),
                                         DevToolsOpenedByAction::kUnknown);
    }
  }
  if (!baa::g_front && inspect) {
    // The window may be made without the inspect action having run: open it,
    // then inspect inside the open window.
    {
      baa::ScopedDocking docking;
      DevToolsWindow::OpenDevToolsWindow(wc, DevToolsToggleAction::Show(),
                                         DevToolsOpenedByAction::kUnknown);
    }
    DevToolsWindow::InspectElement(wc->GetPrimaryMainFrame(), x, y);
  }
  BaaTab* front = baa::g_front;
  baa::g_front = nullptr;
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

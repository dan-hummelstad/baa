// Context menus (through context-menu-hook.patch). PLAN-chromium-phase4.md step 2.

#import <Cocoa/Cocoa.h>

#include <map>
#include <memory>
#include <string>
#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/no_destructor.h"
#include "base/strings/sys_string_conversions.h"
#include "chrome/app/chrome_command_ids.h"
#include "chrome/browser/ui/views/tab_contents/baa_context_menu_hook.h"
#include "components/renderer_context_menu/render_view_context_menu_base.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/context_menu_params.h"
#include "ui/base/l10n/l10n_util_mac.h"
#include "ui/base/models/menu_model.h"
#include "ui/menus/simple_menu_model.h"

namespace baa {
namespace {

// ---- Context menus ----------------------------------------------------------

// Deleted by the answer or by Drop (the menu is just destroyed then, not closed).
class PendingMenu : public Request {
 public:
  PendingMenu(uint64_t tab, std::unique_ptr<RenderViewContextMenuBase> menu)
      : Request(tab, Role::kMenu), menu_(std::move(menu)) {}

  void Command(int command) override {
    Unregister();
    if (command >= 0 && menu_->IsCommandIdEnabled(command)) {
      // Before closing: a page's own items need the menu's client.
      menu_->ExecuteCommand(command, 0);
    }
    menu_->MenuClosed(const_cast<ui::SimpleMenuModel*>(&menu_->menu_model()));
    delete this;
  }
  void Drop() override { delete this; }

 private:
  std::unique_ptr<RenderViewContextMenuBase> menu_;
};

struct Row {
  std::string label;
  int command;
  uint8_t depth;
  uint8_t type;
  bool enabled;
  bool checked;
};

void Flatten(ui::MenuModel* model, int depth, std::vector<Row>& rows) {
  for (size_t i = 0; i < model->GetItemCount(); ++i) {
    if (!model->IsVisibleAt(i)) {
      continue;
    }
    ui::MenuModel::ItemType type = model->GetTypeAt(i);
    int command = model->GetCommandIdAt(i);
    // The emoji panel needs a window of ours that does not exist.
    if (command == IDC_CONTENT_CONTEXT_EMOJI) {
      continue;
    }
    if (type == ui::MenuModel::TYPE_SEPARATOR) {
      // No leading or doubled separators.
      if (!rows.empty() && rows.back().type != BAA_MENU_SEPARATOR) {
        rows.push_back({"", -1, static_cast<uint8_t>(depth), static_cast<uint8_t>(BAA_MENU_SEPARATOR), true, false});
      }
      continue;
    }
    std::string label = base::SysNSStringToUTF8(l10n_util::FixUpWindowsStyleLabel(model->GetLabelAt(i)));
    bool enabled = model->IsEnabledAt(i);
    if (type == ui::MenuModel::TYPE_SUBMENU || type == ui::MenuModel::TYPE_ACTIONABLE_SUBMENU) {
      ui::MenuModel* submenu = model->GetSubmenuModelAt(i);
      if (!submenu) {
        continue;
      }
      rows.push_back({label, -1, static_cast<uint8_t>(depth), static_cast<uint8_t>(BAA_MENU_SUBMENU), enabled, false});
      Flatten(submenu, depth + 1, rows);
      continue;
    }
    bool checked = type == ui::MenuModel::TYPE_CHECK || type == ui::MenuModel::TYPE_RADIO
                       ? model->IsItemCheckedAt(i)
                       : false;
    rows.push_back({label, command, static_cast<uint8_t>(depth),
                    static_cast<uint8_t>(command == IDC_CONTENT_CONTEXT_INSPECTELEMENT ? BAA_MENU_INSPECT : BAA_MENU_COMMAND),
                    enabled, checked});
  }
  while (!rows.empty() && rows.back().type == BAA_MENU_SEPARATOR) {
    rows.pop_back();
  }
}

// The hook of context-menu-hook.patch: true when the app takes the menu.
bool ShowContextMenu(std::unique_ptr<RenderViewContextMenuBase>& menu) {
  content::WebContents* wc = menu->GetWebContents();
  BaaTab* tab = TabFor(wc);
  if (!tab || !TabAnnounced(tab)) {
    return false;
  }
  // A second "show" before the first menu closed (the renderer repeats it).
  if (wc->IsShowingContextMenu()) {
    menu.reset();
    return true;
  }
  RenderViewContextMenuBase* raw = menu.get();
  raw->OnMenuWillShow(const_cast<ui::SimpleMenuModel*>(&raw->menu_model()));
  std::vector<Row> rows;
  Flatten(const_cast<ui::SimpleMenuModel*>(&raw->menu_model()), 0, rows);
  float x = raw->params().x, y = raw->params().y;
  uint64_t id = (new PendingMenu(IdOf(tab), std::move(menu)))->id();

  std::vector<BaaMenuItem> items;
  items.reserve(rows.size());
  for (const Row& r : rows) {
    items.push_back({r.label.c_str(), r.command, r.depth, r.type, r.enabled, r.checked});
  }
  Callbacks().context_menu(CallbackContext(), IdOf(tab), id, x, y, items.data(), items.size());
  return true;
}

}  // namespace

void InstallContextMenuHook() {
  g_baa_show_context_menu = &ShowContextMenu;
}

}  // namespace baa

// Context menus (through context-menu-hook.patch) and app-modal page dialogs
// ("leave site?"). PLAN-chromium-phase4.md steps 2 and 3.

#import <Cocoa/Cocoa.h>

#include <map>
#include <memory>
#include <string>
#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/apple/foundation_util.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/strings/sys_string_conversions.h"
#include "base/strings/utf_string_conversions.h"
#include "chrome/app/chrome_command_ids.h"
#import "chrome/browser/chrome_browser_application_mac.h"
#include "chrome/browser/ui/views/tab_contents/baa_context_menu_hook.h"
#include "components/javascript_dialogs/app_modal_dialog_controller.h"
#include "components/javascript_dialogs/app_modal_dialog_manager.h"
#include "components/javascript_dialogs/app_modal_dialog_view.h"
#include "components/renderer_context_menu/render_view_context_menu_base.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/context_menu_params.h"
#include "ui/base/l10n/l10n_util_mac.h"
#include "ui/base/models/menu_model.h"
#include "ui/menus/simple_menu_model.h"

namespace baa {
namespace {

// ---- Context menus ----------------------------------------------------------

struct PendingMenu {
  std::unique_ptr<RenderViewContextMenuBase> menu;
  BaaTab* tab;
};

std::map<uint64_t, PendingMenu>& Menus() {
  static base::NoDestructor<std::map<uint64_t, PendingMenu>> menus;
  return *menus;
}

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
  uint64_t id = NextId();
  RenderViewContextMenuBase* raw = menu.get();
  raw->OnMenuWillShow(const_cast<ui::SimpleMenuModel*>(&raw->menu_model()));
  std::vector<Row> rows;
  Flatten(const_cast<ui::SimpleMenuModel*>(&raw->menu_model()), 0, rows);
  float x = raw->params().x, y = raw->params().y;
  Menus()[id] = {std::move(menu), tab};

  std::vector<BaaMenuItem> items;
  items.reserve(rows.size());
  for (const Row& r : rows) {
    items.push_back({r.label.c_str(), r.command, r.depth, r.type, r.enabled, r.checked});
  }
  Callbacks().context_menu(CallbackContext(), tab, id, x, y, items.data(), items.size());
  return true;
}

// ---- App-modal dialogs ---------------------------------------------------------

class AppModal;
std::map<uint64_t, AppModal*>& AppModals() {
  static base::NoDestructor<std::map<uint64_t, AppModal*>> modals;
  return *modals;
}

class AppModal : public javascript_dialogs::AppModalDialogView {
 public:
  AppModal(std::unique_ptr<javascript_dialogs::AppModalDialogController> controller, BaaTab* tab)
      : controller_(std::move(controller)), tab_(WeakTab(tab)), id_(NextId()) {}

  void ShowAppModalDialog() override {
    AppModals()[id_] = this;
    int kind = controller_->is_before_unload_dialog()
                   ? (controller_->is_reload() ? BAA_DIALOG_BEFOREUNLOAD_RELOAD
                                               : BAA_DIALOG_BEFOREUNLOAD)
               : controller_->javascript_dialog_type() == content::JAVASCRIPT_DIALOG_TYPE_CONFIRM
                   ? BAA_DIALOG_CONFIRM
               : controller_->javascript_dialog_type() == content::JAVASCRIPT_DIALOG_TYPE_PROMPT
                   ? BAA_DIALOG_PROMPT
                   : BAA_DIALOG_ALERT;
    Callbacks().dialog(CallbackContext(), tab_.get(), id_,
                       kind, base::UTF16ToUTF8(controller_->message_text()).c_str(),
                       base::UTF16ToUTF8(controller_->default_prompt_text()).c_str());
  }
  void ActivateAppModalDialog() override {}
  // Chrome closes it (the page navigated or went away); OnClose before deleting.
  void CloseAppModalDialog() override {
    Forget(/*tell_app=*/true);
    controller_->OnClose();
    delete this;
  }
  // CDP's Page.handleJavaScriptDialog.
  void AcceptAppModalDialog() override { Answer(true, controller_->default_prompt_text(), true); }
  void CancelAppModalDialog() override { Answer(false, std::u16string(), true); }
  bool IsShowing() const override { return true; }

  void Answer(bool ok, const std::u16string& text, bool tell_app) {
    Forget(tell_app);
    if (ok) {
      controller_->OnAccept(text, false);
    } else {
      // Staying on the page stops a quit that was waiting on it.
      if (controller_->is_before_unload_dialog()) {
        chrome_browser_application_mac::CancelTerminate();
      }
      controller_->OnCancel(false);
    }
    delete this;
  }

 private:
  void Forget(bool tell_app) {
    BaaTab* tab = tab_.get();
    if (AppModals().erase(id_) && tell_app && tab && TabAnnounced(tab)) {
      Callbacks().dialog_closed(CallbackContext(), tab, id_);
    }
  }

  std::unique_ptr<javascript_dialogs::AppModalDialogController> controller_;
  base::WeakPtr<BaaTab> tab_;
  const uint64_t id_;
};

}  // namespace

// After Chrome installed its own Cocoa factory (so this runs from the first
// baa_profile_open): pages of the app get the app's dialog, anything else
// (a Chrome-made window) keeps Chrome's.
void InstallAppModalFactory() {
  auto* manager = javascript_dialogs::AppModalDialogManager::GetInstance();
  auto chrome_factory = *manager->view_factory();
  manager->SetNativeDialogFactory(base::BindRepeating(
      [](javascript_dialogs::AppModalDialogManager::AppModalViewFactory chrome_factory,
         std::unique_ptr<javascript_dialogs::AppModalDialogController> controller)
          -> javascript_dialogs::AppModalDialogView* {
        BaaTab* tab = TabFor(controller->web_contents());
        if (!tab || !TabAnnounced(tab)) {
          return chrome_factory.Run(std::move(controller));
        }
        return new AppModal(std::move(controller), tab);
      },
      chrome_factory));
}

void InstallContextMenuHook() {
  g_baa_show_context_menu = &ShowContextMenu;
}

bool AnswerAppModal(uint64_t id, bool ok, const std::u16string& text) {
  auto it = AppModals().find(id);
  if (it == AppModals().end()) {
    return false;
  }
  // Answering deletes it.
  it->second->Answer(ok, text, /*tell_app=*/false);
  return true;
}

// The tab is going away: menus not yet answered are dropped, not run.
void DropMenusOf(BaaTab* tab) {
  for (auto it = Menus().begin(); it != Menus().end();) {
    if (it->second.tab == tab) {
      it = Menus().erase(it);
    } else {
      ++it;
    }
  }
}

}  // namespace baa

void baa_context_menu_done(uint64_t id, int command) {
  auto it = baa::Menus().find(id);
  if (it == baa::Menus().end()) {
    return;
  }
  std::unique_ptr<RenderViewContextMenuBase> menu = std::move(it->second.menu);
  baa::Menus().erase(it);
  if (command >= 0 && menu->IsCommandIdEnabled(command)) {
    // Before closing: a page's own items need the menu's client.
    menu->ExecuteCommand(command, 0);
  }
  menu->MenuClosed(const_cast<ui::SimpleMenuModel*>(&menu->menu_model()));
}

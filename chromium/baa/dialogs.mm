// Page dialogs the app draws: alert/confirm/prompt of a tab, and the app-modal
// ones ("leave site?", popups). PLAN-chromium-phase3.md "baa::DialogDelegate",
// PLAN-chromium-phase4.md step 3.

#import <Cocoa/Cocoa.h>

#include <map>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/single_thread_task_runner.h"
#import "chrome/browser/chrome_browser_application_mac.h"
#include "chrome/browser/ui/javascript_dialogs/javascript_tab_modal_dialog_manager_delegate_desktop.h"
#include "components/javascript_dialogs/app_modal_dialog_controller.h"
#include "components/javascript_dialogs/app_modal_dialog_manager.h"
#include "components/javascript_dialogs/app_modal_dialog_view.h"
#include "components/javascript_dialogs/tab_modal_dialog_manager.h"
#include "components/javascript_dialogs/tab_modal_dialog_view.h"
#include "content/public/browser/javascript_dialog_manager.h"
#include "content/public/browser/web_contents.h"

namespace baa {
namespace {

// JS dialogs: the app words title and labels.
void SendDialog(uint64_t tab, uint64_t id, int kind, const std::string& message, const std::string& default_text) {
  BaaDialog dialog = {kind, message.c_str(), default_text.c_str(), "", "", "", ""};
  Callbacks().dialog(CallbackContext(), tab, id, &dialog);
}

bool AnswerDialog(uint64_t id, bool ok, const std::u16string& answer);

int DialogKind(content::JavaScriptDialogType type,
               bool before_unload = false,
               bool reload = false) {
  if (before_unload) {
    return reload ? BAA_DIALOG_BEFOREUNLOAD_RELOAD : BAA_DIALOG_BEFOREUNLOAD;
  }
  return type == content::JAVASCRIPT_DIALOG_TYPE_CONFIRM  ? BAA_DIALOG_CONFIRM
         : type == content::JAVASCRIPT_DIALOG_TYPE_PROMPT ? BAA_DIALOG_PROMPT
                                                          : BAA_DIALOG_ALERT;
}

// ---- Tab-modal dialogs ---------------------------------------------------------

// The "view" Chrome's dialog manager holds: nothing is drawn here, the app
// shows the dialog and answers through baa_dialog_answer.
class DialogView : public javascript_dialogs::TabModalDialogView {
 public:
  DialogView(uint64_t id,
             BaaTab* tab,
             int kind,
             std::string message,
             std::u16string default_text,
             content::JavaScriptDialogManager::DialogClosedCallback callback)
      : id_(id),
        tab_(WeakTab(tab)),
        kind_(kind),
        message_(std::move(message)),
        default_text_(std::move(default_text)),
        callback_(std::move(callback)) {}

  // The manager makes the dialog go away (navigation, tab hidden, a newer
  // dialog). Not called back for answers, which forget the id first.
  void CloseDialogWithoutCallback() override;
  std::u16string GetUserInput() override { return default_text_; }

  base::WeakPtr<DialogView> GetWeakPtr() { return weak_factory_.GetWeakPtr(); }
  content::JavaScriptDialogManager::DialogClosedCallback TakeCallback() {
    return std::move(callback_);
  }
  void MarkAnswered() { answered_ = true; }
  BaaTab* tab() const { return tab_.get(); }
  uint64_t id() const { return id_; }
  // Tells the app about it (again, once the tab is announced).
  void Show(BaaTab* tab) const { SendDialog(IdOf(tab), id_, kind_, message_, base::UTF16ToUTF8(default_text_)); }

 private:
  const uint64_t id_;
  base::WeakPtr<BaaTab> tab_;
  const int kind_;
  std::string message_;
  std::u16string default_text_;
  content::JavaScriptDialogManager::DialogClosedCallback callback_;
  bool answered_ = false;
  base::WeakPtrFactory<DialogView> weak_factory_{this};
};

std::map<uint64_t, std::unique_ptr<DialogView>>& Dialogs() {
  static base::NoDestructor<std::map<uint64_t, std::unique_ptr<DialogView>>> d;
  return *d;
}

void DialogView::CloseDialogWithoutCallback() {
  if (answered_) {
    return;
  }
  auto it = Dialogs().find(id_);
  if (it == Dialogs().end()) {
    return;
  }
  // Destroys this; nothing after.
  std::unique_ptr<DialogView> self = std::move(it->second);
  Dialogs().erase(it);
  BaaTab* tab = tab_.get();
  if (TabAnnounced(tab)) {
    Callbacks().request_closed(CallbackContext(), IdOf(tab), id_);
  }
}

class DialogDelegate : public JavaScriptTabModalDialogManagerDelegateDesktop {
 public:
  explicit DialogDelegate(content::WebContents* wc)
      : JavaScriptTabModalDialogManagerDelegateDesktop(wc), wc_(wc) {}

  base::WeakPtr<javascript_dialogs::TabModalDialogView> CreateNewDialog(
      content::WebContents*,
      const std::u16string&,
      content::JavaScriptDialogType type,
      const std::u16string& message,
      const std::u16string& default_text,
      content::JavaScriptDialogManager::DialogClosedCallback callback,
      base::OnceClosure) override {
    BaaTab* tab = TabFor(wc_);
    uint64_t id = NextId();
    auto view = std::make_unique<DialogView>(id, tab, DialogKind(type),
                                             base::UTF16ToUTF8(message),
                                             default_text, std::move(callback));
    auto weak = view->GetWeakPtr();
    DialogView* raw = view.get();
    Dialogs()[id] = std::move(view);
    if (!tab) {
      // A page nobody shows (its tab handle is gone): answer "no" so it does not hang.
      base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
          FROM_HERE, base::BindOnce([](uint64_t id) { AnswerDialog(id, false, std::u16string()); }, id));
    } else if (TabAnnounced(tab)) {
      raw->Show(tab);
    }
    return weak;
  }

 private:
  raw_ptr<content::WebContents> wc_;
};

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
    int kind = DialogKind(controller_->javascript_dialog_type(),
                          controller_->is_before_unload_dialog(), controller_->is_reload());
    SendDialog(IdOf(tab_.get()), id_, kind, base::UTF16ToUTF8(controller_->message_text()),
               base::UTF16ToUTF8(controller_->default_prompt_text()));
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
    if (AppModals().erase(id_) && tell_app && TabAnnounced(tab)) {
      Callbacks().request_closed(CallbackContext(), IdOf(tab), id_);
    }
  }

  std::unique_ptr<javascript_dialogs::AppModalDialogController> controller_;
  base::WeakPtr<BaaTab> tab_;
  const uint64_t id_;
};

// WP-C1b replaces these maps with the request registry.
bool AnswerDialog(uint64_t id, bool ok, const std::u16string& answer) {
  auto it = Dialogs().find(id);
  if (it == Dialogs().end()) {
    auto modal = AppModals().find(id);
    if (modal == AppModals().end()) {
      return false;
    }
    // Answering deletes it.
    modal->second->Answer(ok, answer, /*tell_app=*/false);
    return true;
  }
  // Forgotten before the callback runs: the manager answers by closing the
  // dialog, which must not read as the page going away.
  std::unique_ptr<DialogView> view = std::move(it->second);
  Dialogs().erase(it);
  view->MarkAnswered();
  view->TakeCallback().Run(ok, answer);
  return true;
}

}  // namespace

void AttachDialogs(content::WebContents* wc) {
  // Chrome's views dialog CHECK-crashes with our window; the app draws them.
  wc->RemoveUserData(javascript_dialogs::TabModalDialogManager::UserDataKey());
  javascript_dialogs::TabModalDialogManager::CreateForWebContents(
      wc, std::make_unique<DialogDelegate>(wc));
}

// A tab that left our windows: pending page dialogs say "no" so the page does
// not hang, and Chrome's own dialogs come back.
void ReturnDialogsToChrome(BaaTab* tab, content::WebContents* wc) {
  std::vector<uint64_t> pending;
  for (auto& [id, view] : Dialogs()) {
    if (view->tab() == tab) {
      pending.push_back(id);
    }
  }
  for (uint64_t id : pending) {
    AnswerDialog(id, false, std::u16string());
  }
  wc->RemoveUserData(javascript_dialogs::TabModalDialogManager::UserDataKey());
  javascript_dialogs::TabModalDialogManager::CreateForWebContents(
      wc, std::make_unique<JavaScriptTabModalDialogManagerDelegateDesktop>(wc));
}

void ShowPendingDialogsOf(BaaTab* tab) {
  for (auto& [id, view] : Dialogs()) {
    if (view->tab() == tab) {
      view->Show(tab);
    }
  }
}

void DropDialogsOf(BaaTab* tab, bool tell) {
  for (auto d = Dialogs().begin(); d != Dialogs().end();) {
    if (d->second->tab() == tab) {
      uint64_t id = d->first;
      d = Dialogs().erase(d);
      if (tell) {
        Callbacks().request_closed(CallbackContext(), IdOf(tab), id);
      }
    } else {
      ++d;
    }
  }
}

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
        if (!TabAnnounced(tab)) {
          return chrome_factory.Run(std::move(controller));
        }
        return new AppModal(std::move(controller), tab);
      },
      chrome_factory));
}

}  // namespace baa

void baa_answer(uint64_t id, const BaaAnswer* answer) {
  if (!answer) {
    return;
  }
  if (!baa::AnswerDialog(id, answer->ok, base::UTF8ToUTF16(answer->text ? answer->text : ""))) {
    baa::AnswerPermission(id, answer->ok);
  }
}

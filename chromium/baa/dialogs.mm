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

// JS dialogs: the app words the labels. The title is Chrome's ("x.com says"), the only place
// the asking frame's origin is named.
void SendDialog(uint64_t tab,
                uint64_t id,
                int kind,
                const std::string& message,
                const std::string& default_text,
                const std::string& title) {
  BaaDialog dialog = {kind, message.c_str(), default_text.c_str(), title.c_str(), "", "", ""};
  Callbacks().dialog(CallbackContext(), tab, id, &dialog);
}

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
// shows the dialog and answers through baa_answer. Owns itself.
class DialogView : public javascript_dialogs::TabModalDialogView, public Request {
 public:
  DialogView(BaaTab* tab,
             int kind,
             std::string title,
             std::string message,
             std::u16string default_text,
             content::JavaScriptDialogManager::DialogClosedCallback callback)
      : Request(IdOf(tab), Role::kDialog),
        kind_(kind),
        title_(std::move(title)),
        message_(std::move(message)),
        default_text_(std::move(default_text)),
        callback_(std::move(callback)) {}

  // The manager makes the dialog go away (navigation, tab hidden, a newer
  // dialog). Not called back for answers, which unregister first.
  void CloseDialogWithoutCallback() override {
    if (answered_) {
      return;
    }
    WithdrawRequest(id());
    delete this;
  }
  std::u16string GetUserInput() override { return default_text_; }

  // The tab is gone or left: Chrome's callback is never run.
  void Drop() override { delete this; }
  void Answer(const BaaAnswer& answer) override {
    if (answered_) {
      return;
    }
    // Unregistered before the callback runs: the manager answers by closing the
    // dialog, which must not read as the page going away.
    answered_ = true;
    Unregister();
    auto callback = std::move(callback_);
    std::move(callback).Run(answer.ok, base::UTF8ToUTF16(answer.text ? answer.text : ""));
    delete this;
  }

  base::WeakPtr<DialogView> GetWeakPtr() { return weak_factory_.GetWeakPtr(); }
  // Tells the app about it (again, once the tab is announced).
  void Show(BaaTab* tab) const {
    SendDialog(IdOf(tab), id(), kind_, message_, base::UTF16ToUTF8(default_text_), title_);
  }

 private:
  const int kind_;
  std::string title_;
  std::string message_;
  std::u16string default_text_;
  content::JavaScriptDialogManager::DialogClosedCallback callback_;
  bool answered_ = false;
  base::WeakPtrFactory<DialogView> weak_factory_{this};
};

class DialogDelegate : public JavaScriptTabModalDialogManagerDelegateDesktop {
 public:
  explicit DialogDelegate(content::WebContents* wc)
      : JavaScriptTabModalDialogManagerDelegateDesktop(wc), wc_(wc) {}

  base::WeakPtr<javascript_dialogs::TabModalDialogView> CreateNewDialog(
      content::WebContents*,
      const std::u16string& title,
      content::JavaScriptDialogType type,
      const std::u16string& message,
      const std::u16string& default_text,
      content::JavaScriptDialogManager::DialogClosedCallback callback,
      base::OnceClosure) override {
    BaaTab* tab = TabFor(wc_);
    auto* view = new DialogView(tab, DialogKind(type), base::UTF16ToUTF8(title), base::UTF16ToUTF8(message),
                                default_text,
                                std::move(callback));
    auto weak = view->GetWeakPtr();
    if (!tab) {
      // A page nobody shows (its tab handle is gone): answer "no" so it does not hang.
      base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
          FROM_HERE, base::BindOnce([](uint64_t id) {
            if (Request* request = FindRequest(id)) {
              request->Answer(BaaAnswer{});
            }
          }, view->id()));
    } else if (TabAnnounced(tab)) {
      view->Show(tab);
    }
    return weak;
  }

 private:
  raw_ptr<content::WebContents> wc_;
};

// ---- App-modal dialogs ---------------------------------------------------------

class AppModal : public javascript_dialogs::AppModalDialogView, public Request {
 public:
  AppModal(std::unique_ptr<javascript_dialogs::AppModalDialogController> controller, BaaTab* tab)
      : Request(IdOf(tab), Role::kDialog), controller_(std::move(controller)) {}

  void ShowAppModalDialog() override {
    int kind = DialogKind(controller_->javascript_dialog_type(),
                          controller_->is_before_unload_dialog(), controller_->is_reload());
    SendDialog(tab(), id(), kind, base::UTF16ToUTF8(controller_->message_text()),
               base::UTF16ToUTF8(controller_->default_prompt_text()),
               base::UTF16ToUTF8(controller_->title()));
  }
  void ActivateAppModalDialog() override {}
  // Chrome closes it (the page navigated or went away); OnClose before deleting.
  void CloseAppModalDialog() override {
    WithdrawRequest(id());
    controller_->OnClose();
    delete this;
  }
  // CDP's Page.handleJavaScriptDialog.
  void AcceptAppModalDialog() override {
    WithdrawRequest(id());
    Finish(true, controller_->default_prompt_text());
  }
  void CancelAppModalDialog() override {
    WithdrawRequest(id());
    Finish(false, std::u16string());
  }
  bool IsShowing() const override { return true; }

  void Answer(const BaaAnswer& answer) override {
    Unregister();
    Finish(answer.ok, base::UTF8ToUTF16(answer.text ? answer.text : ""));
  }
  // The tab is gone: Chrome still closes the dialog (CloseAppModalDialog), which deletes it.
  void Drop() override {}

 private:
  // Deletes this.
  void Finish(bool ok, const std::u16string& text) {
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

  std::unique_ptr<javascript_dialogs::AppModalDialogController> controller_;
};

}  // namespace

void AttachDialogs(content::WebContents* wc) {
  // Chrome's views dialog CHECK-crashes with our window; the app draws them.
  wc->RemoveUserData(javascript_dialogs::TabModalDialogManager::UserDataKey());
  javascript_dialogs::TabModalDialogManager::CreateForWebContents(
      wc, std::make_unique<DialogDelegate>(wc));
  AttachTabDialogs(wc);
}

// A tab that left our windows: pending page dialogs say "no" so the page does
// not hang, and Chrome's own dialogs come back.
void ReturnDialogsToChrome(BaaTab* tab, content::WebContents* wc) {
  for (uint64_t id : DialogsOf(IdOf(tab))) {
    if (Request* request = FindRequest(id)) {
      request->Answer(BaaAnswer{});
    }
  }
  wc->RemoveUserData(javascript_dialogs::TabModalDialogManager::UserDataKey());
  javascript_dialogs::TabModalDialogManager::CreateForWebContents(
      wc, std::make_unique<JavaScriptTabModalDialogManagerDelegateDesktop>(wc));
  RestoreTabDialogs(wc);
}

void ShowPendingDialogsOf(BaaTab* tab) {
  for (uint64_t id : DialogsOf(IdOf(tab))) {
    // Only a DialogView can be waiting for the app to know its tab: AppModal exists only for an
    // announced one. (Chrome builds without RTTI.)
    if (Request* request = FindRequest(id)) {  // an earlier Show's answer may have closed it
      static_cast<DialogView*>(request)->Show(tab);
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

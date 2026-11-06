// Chrome's own dialogs, drawn by the app: HTTP auth, external protocol, hung page, extension
// install. Each is a Request that sends `dialog` with Chrome's localized strings and is answered by
// baa_answer. PLAN-m3.md §2.2. Only pages the app already knows are taken; anything else keeps
// Chrome's views dialog.

#include <memory>
#include <optional>
#include <string>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/strings/utf_string_conversions.h"
#include "chrome/browser/baa_hooks.h"
#include "chrome/browser/external_protocol/external_protocol_handler.h"
#include "chrome/browser/extensions/extension_install_prompt.h"
#include "chrome/browser/extensions/extension_install_prompt_show_params.h"
#include "chrome/browser/hang_monitor/hang_crash_dump.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/login/login_handler.h"
#include "chrome/browser/ui/views/tab_dialogs_views.h"
#include "chrome/common/pref_names.h"
#include "chrome/grit/generated_resources.h"
#include "components/prefs/pref_service.h"
#include "components/strings/grit/components_strings.h"
#include "components/url_formatter/elide_url.h"
#include "content/public/browser/browser_task_traits.h"
#include "content/public/browser/browser_thread.h"
#include "content/public/browser/navigation_handle.h"
#include "content/public/browser/render_process_host.h"
#include "content/public/browser/render_widget_host.h"
#include "content/public/browser/web_contents.h"
#include "content/public/browser/web_contents_observer.h"
#include "content/public/common/result_codes.h"
#include "extensions/browser/install_prompt_data.h"
#include "net/base/auth.h"
#include "net/base/registry_controlled_domains/registry_controlled_domain.h"
#include "ui/base/l10n/l10n_util.h"
#include "ui/gfx/text_elider.h"

namespace baa {
namespace {

std::string Utf8(const std::u16string& s) {
  return base::UTF16ToUTF8(s);
}

// The tab the app shows for `wc`, or null: Chrome's dialog then keeps the request.
BaaTab* ShownTab(content::WebContents* wc) {
  BaaTab* tab = wc ? TabFor(wc) : nullptr;
  return TabAnnounced(tab) ? tab : nullptr;
}

void Send(uint64_t tab, uint64_t id, int kind, const std::u16string& title,
          const std::u16string& message, const std::u16string& ok, const std::u16string& cancel,
          const std::u16string& check = std::u16string()) {
  std::string t = Utf8(title), m = Utf8(message), o = Utf8(ok), c = Utf8(cancel), k = Utf8(check);
  BaaDialog dialog = {kind, m.c_str(), "", t.c_str(), o.c_str(), c.c_str(), k.c_str()};
  Callbacks().dialog(CallbackContext(), tab, id, &dialog);
}

std::u16string FromAnswer(const char* s) {
  return base::UTF8ToUTF16(s ? s : "");
}

// ---- HTTP auth -----------------------------------------------------------------

// Chrome owns it (the network request's login delegate); the app draws the fields. The password
// manager never sees the view (LoginModelData is dropped), so its HttpAuthManager makes no form
// manager: SetAuth's OnPasswordFormSubmitted saves nothing and no save bubble (a BrowserView
// feature) can follow. credentials_enable_service is off as well (profile.mm).
class LoginPrompt : public LoginHandler, public Request {
 public:
  LoginPrompt(const net::AuthChallengeInfo& info,
              content::WebContents* wc,
              content::LoginDelegate::LoginAuthRequiredCallback callback,
              BaaTab* tab)
      : LoginHandler(info, wc, std::move(callback)), Request(IdOf(tab), Role::kDialog) {}
  ~LoginPrompt() override { CloseDialog(); }

  bool BuildViewImpl(const std::u16string& authority,
                     const std::u16string& explanation,
                     LoginModelData*) override {
    Send(Request::tab(), id(), BAA_DIALOG_AUTH, l10n_util::GetStringUTF16(IDS_LOGIN_DIALOG_TITLE),
         explanation.empty() ? authority : authority + u"\n" + explanation,
         l10n_util::GetStringUTF16(IDS_LOGIN_DIALOG_OK_BUTTON_LABEL), std::u16string());
    return true;
  }
  // Also Chrome withdrawing it (another handler answered the same challenge, the page left).
  void CloseDialog() override { WithdrawRequest(id()); }

  void Answer(const BaaAnswer& answer) override {
    Unregister();
    if (answer.ok) {
      SetAuth(FromAnswer(answer.text), FromAnswer(answer.password));
    } else {
      CancelAuth(true);
    }
  }
  // Chrome's callback must still run, but not inside the tab's teardown.
  void Drop() override {
    content::GetUIThreadTaskRunner({})->PostTask(
        FROM_HERE, base::BindOnce(&LoginHandler::CancelAuth, GetWeakPtr(), false));
  }
};

std::unique_ptr<LoginHandler> TakeLogin(const net::AuthChallengeInfo& info,
                                        content::WebContents* wc,
                                        content::LoginDelegate::LoginAuthRequiredCallback& cb) {
  BaaTab* tab = ShownTab(wc);
  if (!tab) {
    return nullptr;
  }
  return std::make_unique<LoginPrompt>(info, wc, std::move(cb), tab);
}

// ---- External protocol ----------------------------------------------------------

// Withdrawn when the page goes to another site, as Chrome's web-modal dialog is.
class ExternalAsk : public Request, public content::WebContentsObserver {
 public:
  ExternalAsk(BaaTab* tab,
              const GURL& url,
              content::WebContents* wc,
              const std::optional<url::Origin>& origin,
              content::WeakDocumentPtr document)
      : Request(IdOf(tab), Role::kDialog),
        content::WebContentsObserver(wc),
        url_(url),
        origin_(origin),
        document_(std::move(document)) {}

  void Show(const std::u16string& program) {
    std::u16string elided;
    gfx::ElideString(program, 32, &elided);
    std::u16string message =
        !origin_ || origin_->opaque()
            ? l10n_util::GetStringUTF16(IDS_EXTERNAL_PROTOCOL_MESSAGE)
            : l10n_util::GetStringFUTF16(IDS_EXTERNAL_PROTOCOL_MESSAGE_WITH_INITIATING_ORIGIN,
                                         url_formatter::FormatOriginForSecurityDisplay(*origin_));
    Profile* profile = Profile::FromBrowserContext(web_contents()->GetBrowserContext());
    std::u16string check;
    if (profile->GetPrefs()->GetBoolean(prefs::kExternalProtocolDialogShowAlwaysOpenCheckbox) &&
        ExternalProtocolHandler::MayRememberAllowDecisionsForThisOrigin(
            origin_ ? &*origin_ : nullptr)) {
      check = l10n_util::GetStringFUTF16(
          IDS_EXTERNAL_PROTOCOL_CHECKBOX_PER_ORIGIN_TEXT,
          url_formatter::FormatOriginForSecurityDisplay(
              *origin_, url_formatter::SchemeDisplay::OMIT_CRYPTOGRAPHIC));
    }
    Send(tab(), id(), BAA_DIALOG_EXTERNAL_PROTOCOL,
         l10n_util::GetStringFUTF16(IDS_EXTERNAL_PROTOCOL_TITLE, elided), message,
         l10n_util::GetStringFUTF16(IDS_EXTERNAL_PROTOCOL_OK_BUTTON_TEXT, program),
         l10n_util::GetStringUTF16(IDS_EXTERNAL_PROTOCOL_CANCEL_BUTTON_TEXT), check);
  }

  void Answer(const BaaAnswer& answer) override {
    Unregister();
    bool remember = answer.ok && answer.checked && origin_;
    ExternalProtocolHandler::RecordHandleStateMetrics(
        remember, answer.ok ? ExternalProtocolHandler::DONT_BLOCK : ExternalProtocolHandler::BLOCK);
    if (answer.ok && web_contents()) {
      // The app only offers the checkbox when Chrome would have.
      if (remember) {
        ExternalProtocolHandler::SetBlockState(
            url_.GetScheme(), *origin_, ExternalProtocolHandler::DONT_BLOCK,
            Profile::FromBrowserContext(web_contents()->GetBrowserContext()));
      }
      ExternalProtocolHandler::LaunchUrlWithoutSecurityCheck(url_, web_contents(), document_);
    }
    delete this;
  }
  void Drop() override { delete this; }

  // WebContentsModalDialogManager::DidFinishNavigation's rule.
  void DidFinishNavigation(content::NavigationHandle* navigation) override {
    if (navigation->IsInPrimaryMainFrame() && navigation->HasCommitted() &&
        !net::registry_controlled_domains::SameDomainOrHost(
            navigation->GetPreviousPrimaryMainFrameURL(), navigation->GetURL(),
            net::registry_controlled_domains::INCLUDE_PRIVATE_REGISTRIES)) {
      WithdrawRequest(id());
      delete this;
    }
  }

 private:
  GURL url_;
  std::optional<url::Origin> origin_;
  content::WeakDocumentPtr document_;
};

bool TakeExternal(const GURL& url,
                  content::WebContents* wc,
                  const std::optional<url::Origin>& origin,
                  content::WeakDocumentPtr document,
                  const std::u16string& program) {
  BaaTab* tab = ShownTab(wc);
  if (!tab) {
    return false;
  }
  (new ExternalAsk(tab, url, wc, origin, std::move(document)))->Show(program);
  return true;
}

// ---- Hung page -------------------------------------------------------------------

// Held by the tab's TabDialogs, which also drops it with the tab.
class Hung : public Request {
 public:
  Hung(BaaTab* tab, content::RenderWidgetHost* host, base::RepeatingClosure restart)
      : Request(IdOf(tab), Role::kDialog),
        host_(host),
        process_(host->GetProcess()->GetDeprecatedID()),
        restart_(std::move(restart)) {}

  const void* host() const { return host_; }
  bool answered() const { return answered_; }

  void Show(const std::u16string& page) {
    Send(tab(), id(), BAA_DIALOG_UNRESPONSIVE,
         l10n_util::GetPluralStringFUTF16(IDS_BROWSER_HANGMONITOR_RENDERER_TITLE, 1),
         l10n_util::GetPluralStringFUTF16(IDS_BROWSER_HANGMONITOR_RENDERER, 1) + u"\n" + page,
         l10n_util::GetPluralStringFUTF16(IDS_BROWSER_HANGMONITOR_RENDERER_END, 1),
         l10n_util::GetStringUTF16(IDS_BROWSER_HANGMONITOR_RENDERER_WAIT));
  }

  void Answer(const BaaAnswer& answer) override {
    Unregister();
    answered_ = true;
    if (!answer.ok) {
      restart_.Run();
      return;
    }
    // As Chrome's "Exit pages" (hung_renderer_view.cc ForceCrashHungRenderer).
    if (auto* process = content::RenderProcessHost::FromID(process_)) {
      CrashDumpHungChildProcess(process->GetProcess().Handle());
      process->Shutdown(content::RESULT_CODE_HUNG);
    }
  }
  void Drop() override {}

 private:
  const void* const host_;  // only compared
  const int process_;
  base::RepeatingClosure restart_;
  bool answered_ = false;
};

class BaaTabDialogs : public TabDialogsViews, public content::WebContentsObserver {
 public:
  explicit BaaTabDialogs(content::WebContents* wc)
      : TabDialogsViews(wc), content::WebContentsObserver(wc), wc_(wc) {}

  // Chrome's own views dialogs again (the tab left our windows).
  static void Restore(content::WebContents* wc) {
    wc->SetUserData(TabDialogs::UserDataKey(), std::make_unique<TabDialogsViews>(wc));
  }
  static void Install(content::WebContents* wc) {
    wc->SetUserData(TabDialogs::UserDataKey(), std::make_unique<BaaTabDialogs>(wc));
  }

  void ShowHungRendererDialog(content::RenderWidgetHost* host,
                              base::RepeatingClosure restart) override {
    BaaTab* tab = ShownTab(wc_);
    if (!tab) {
      TabDialogsViews::ShowHungRendererDialog(host, std::move(restart));
      return;
    }
    Withdraw();
    hung_ = std::make_unique<Hung>(tab, host, std::move(restart));
    std::u16string page = wc_->GetTitle();
    hung_->Show(page.empty() ? base::UTF8ToUTF16(wc_->GetVisibleURL().spec()) : page);
  }
  void HideHungRendererDialog(content::RenderWidgetHost* host) override {
    if (!hung_) {
      TabDialogsViews::HideHungRendererDialog(host);
    } else if (hung_->host() == host) {
      Withdraw();
    }
  }
  bool IsShowingHungRendererDialog() override {
    return hung_ ? !hung_->answered() : TabDialogsViews::IsShowingHungRendererDialog();
  }

  // The renderer died: nothing to wait for or end any more.
  void PrimaryMainFrameRenderProcessGone(base::TerminationStatus) override { Withdraw(); }

 private:
  void Withdraw() {
    if (hung_) {
      WithdrawRequest(hung_->id());
      hung_.reset();
    }
  }

  raw_ptr<content::WebContents> wc_;
  std::unique_ptr<Hung> hung_;
};

// ---- Extension install, re-enable, permissions.request ---------------------------

class InstallAsk : public Request {
 public:
  InstallAsk(BaaTab* tab,
             ExtensionInstallPrompt::DoneCallback done,
             std::unique_ptr<extensions::InstallPromptData> prompt)
      : Request(IdOf(tab), Role::kDialog), done_(std::move(done)), prompt_(std::move(prompt)) {}

  void Show() {
    std::u16string message;
    if (size_t count = prompt_->GetPermissionCount()) {
      message = prompt_->GetPermissionsHeading();
      for (size_t i = 0; i < count; ++i) {
        message += u"\n" + prompt_->GetPermission(i);
      }
    }
    // Chrome offers the checkbox (default off) when permissions can be withheld on accept.
    // ExtensionInstallPrompt already told prompt_'s observers OnDialogOpened.
    withhold_ = !prompt_->has_webstore_data() && prompt_->ShouldWithheldPermissionsOnDialogAccept();
    Send(tab(), id(), BAA_DIALOG_EXTENSION_INSTALL, prompt_->GetDialogTitle(), message,
         prompt_->GetAcceptButtonLabel(), prompt_->GetAbortButtonLabel(),
         withhold_ ? l10n_util::GetStringUTF16(IDS_EXTENSION_PROMPT_GRANT_PERMISSIONS_CHECKBOX)
                   : std::u16string());
  }

  void Answer(const BaaAnswer& answer) override {
    Unregister();
    Finish(answer.ok, answer.checked);
  }
  // Chrome's callback must run or the install hangs.
  void Drop() override { Finish(false, false); }

 private:
  // Deletes this.
  void Finish(bool ok, bool checked) {
    using Result = ExtensionInstallPrompt::Result;
    Result result = Result::USER_CANCELED;
    if (ok) {
      prompt_->OnDialogAccepted();
      result = withhold_ && !checked ? Result::ACCEPTED_WITH_WITHHELD_PERMISSIONS : Result::ACCEPTED;
    } else {
      prompt_->OnDialogCanceled();
    }
    auto done = std::move(done_);
    delete this;
    std::move(done).Run(ExtensionInstallPrompt::DoneCallbackPayload(result));
  }

  ExtensionInstallPrompt::DoneCallback done_;
  std::unique_ptr<extensions::InstallPromptData> prompt_;
  bool withhold_ = false;
};

bool TakeInstall(std::unique_ptr<ExtensionInstallPromptShowParams>& params,
                 ExtensionInstallPrompt::DoneCallback& done,
                 std::unique_ptr<extensions::InstallPromptData>& prompt) {
  // Enterprise request flows need Chrome's justification field.
  if (prompt->type() == extensions::InstallPromptData::EXTENSION_REQUEST_PROMPT ||
      prompt->requires_parent_permission()) {
    return false;
  }
  BaaTab* tab = ShownTab(params->GetParentWebContents());
  if (!tab) {
    // Installed from a page nobody shows (chrome.management, a store tab elsewhere): the
    // profile's active tab takes the sheet.
    if (BaaProfile* profile = ProfileFor(params->profile())) {
      if (BrowserWindowInterface* browser = ExistingBrowserOf(profile)) {
        tab = ShownTab(browser->GetTabStripModel()->GetActiveWebContents());
      }
    }
  }
  if (!tab) {
    return false;
  }
  (new InstallAsk(tab, std::move(done), std::move(prompt)))->Show();
  return true;
}

}  // namespace

void InstallDialogHooks() {
  g_baa_login_handler = &TakeLogin;
  g_baa_external_protocol = &TakeExternal;
  g_baa_install_dialog = &TakeInstall;
}

void AttachTabDialogs(content::WebContents* wc) {
  BaaTabDialogs::Install(wc);
}

void RestoreTabDialogs(content::WebContents* wc) {
  BaaTabDialogs::Restore(wc);
}

}  // namespace baa

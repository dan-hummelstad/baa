// Permission prompts and per-site settings. PLAN-chromium-phase4.md step 1.

#import <Cocoa/Cocoa.h>

#include <map>
#include <string>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/task/single_thread_task_runner.h"
#include "chrome/browser/content_settings/host_content_settings_map_factory.h"
#include "chrome/browser/profiles/profile.h"
#include "components/content_settings/core/browser/host_content_settings_map.h"
#include "components/content_settings/core/common/content_settings.h"
#include "components/content_settings/core/common/content_settings_types.h"
#include "components/permissions/permission_prompt.h"
#include "components/permissions/permission_request.h"
#include "components/permissions/permission_request_manager.h"
#include "components/permissions/request_type.h"
#include "content/public/browser/web_contents.h"
#include "url/gurl.h"

namespace baa {
namespace {

class Prompt;
std::map<uint64_t, Prompt*>& Prompts() {
  static base::NoDestructor<std::map<uint64_t, Prompt*>> prompts;
  return *prompts;
}

uint32_t KindBit(permissions::RequestType type) {
  switch (type) {
    case permissions::RequestType::kCameraStream:
      return BAA_PERMISSION_CAMERA;
    case permissions::RequestType::kMicStream:
      return BAA_PERMISSION_MICROPHONE;
    case permissions::RequestType::kGeolocation:
      return BAA_PERMISSION_LOCATION;
    case permissions::RequestType::kNotifications:
      return BAA_PERMISSION_NOTIFICATIONS;
    case permissions::RequestType::kClipboard:
      return BAA_PERMISSION_CLIPBOARD;
    case permissions::RequestType::kMidiSysex:
      return BAA_PERMISSION_MIDI;
    default:
      return BAA_PERMISSION_OTHER;
  }
}

// What the manager holds while a request waits for the app. Accept/Deny delete
// it (FinalizeCurrentRequests), so answering looks it up by id, forgets it
// first and never touches it afterwards.
class Prompt : public permissions::PermissionPrompt {
 public:
  Prompt(BaaTab* tab, Delegate* delegate)
      : tab_(tab), delegate_(delegate->GetWeakPtr()), id_(NextId()) {
    uint32_t kinds = 0;
    for (const auto& request : delegate->Requests()) {
      kinds |= KindBit(request->request_type());
    }
    origin_ = delegate->GetRequestingOrigin().spec();
    Prompts()[id_] = this;
    kinds_ = kinds;
    if (Callbacks().permission) {
      Callbacks().permission(CallbackContext(), tab_, id_, origin_.c_str(), kinds_);
    }
  }

  ~Prompt() override {
    // Still waiting: Chrome dropped the request (navigation, tab closed).
    if (Prompts().erase(id_) && TabAlive(tab_) && Callbacks().permission_closed) {
      Callbacks().permission_closed(CallbackContext(), tab_, id_);
    }
  }

  base::WeakPtr<Delegate> delegate() const { return delegate_; }
  BaaTab* tab() const { return tab_; }

  bool UpdateAnchor() override { return true; }
  // The app keeps asks per tab.
  TabSwitchingBehavior GetTabSwitchingBehavior() override { return kKeepPromptAlive; }
  permissions::PermissionPromptDisposition GetPromptDisposition() const override {
    return permissions::PermissionPromptDisposition::ANCHORED_BUBBLE;
  }
  bool IsAskPrompt() const override { return true; }
  std::optional<gfx::Rect> GetViewBoundsInScreen() const override { return std::nullopt; }
  std::vector<permissions::ElementAnchoredBubbleVariant> GetPromptVariants() const override {
    return {};
  }
  std::optional<permissions::feature_params::PermissionElementPromptPosition>
  GetPromptPosition() const override {
    return std::nullopt;
  }

 private:
  raw_ptr<BaaTab> tab_;
  base::WeakPtr<Delegate> delegate_;
  const uint64_t id_;
  std::string origin_;
  uint32_t kinds_ = 0;
};

class SilentPrompt : public permissions::PermissionPrompt {
 public:
  bool UpdateAnchor() override { return true; }
  TabSwitchingBehavior GetTabSwitchingBehavior() override { return kDestroyPromptAndIgnoreRequest; }
  permissions::PermissionPromptDisposition GetPromptDisposition() const override {
    return permissions::PermissionPromptDisposition::NONE_VISIBLE;
  }
  bool IsAskPrompt() const override { return true; }
  std::optional<gfx::Rect> GetViewBoundsInScreen() const override { return std::nullopt; }
  std::vector<permissions::ElementAnchoredBubbleVariant> GetPromptVariants() const override {
    return {};
  }
  std::optional<permissions::feature_params::PermissionElementPromptPosition>
  GetPromptPosition() const override {
    return std::nullopt;
  }
};

std::unique_ptr<permissions::PermissionPrompt> MakePrompt(
    content::WebContents* wc,
    permissions::PermissionPrompt::Delegate* delegate) {
  BaaTab* tab = TabFor(wc);
  if (!tab || !TabAnnounced(tab)) {
    // Not in the app yet (or not a page it shows): dismiss, from a task (the
    // manager has not stored the view yet).
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(
                       [](base::WeakPtr<permissions::PermissionPrompt::Delegate> d) {
                         if (d) {
                           d->Ignore(std::monostate());
                         }
                       },
                       delegate->GetWeakPtr()));
    return std::make_unique<SilentPrompt>();
  }
  return std::make_unique<Prompt>(tab, delegate);
}

ContentSettingsType TypeOf(uint32_t kind) {
  switch (kind) {
    case BAA_PERMISSION_CAMERA:
      return ContentSettingsType::MEDIASTREAM_CAMERA;
    case BAA_PERMISSION_MICROPHONE:
      return ContentSettingsType::MEDIASTREAM_MIC;
    case BAA_PERMISSION_LOCATION:
      return ContentSettingsType::GEOLOCATION;
    case BAA_PERMISSION_NOTIFICATIONS:
      return ContentSettingsType::NOTIFICATIONS;
    case BAA_PERMISSION_CLIPBOARD:
      return ContentSettingsType::CLIPBOARD_READ_WRITE;
    case BAA_PERMISSION_MIDI:
      return ContentSettingsType::MIDI_SYSEX;
    default:
      return ContentSettingsType::DEFAULT;
  }
}

}  // namespace

// After the tab's helpers exist, and again after a WebContents swap.
void AttachPermissions(content::WebContents* wc) {
  if (auto* manager = permissions::PermissionRequestManager::FromWebContents(wc)) {
    // The quiet chip UI needs a views toolbar; every request goes to the app.
    manager->clear_permission_ui_selector_for_testing();
    manager->set_view_factory_for_testing(base::BindRepeating(&MakePrompt));
  }
}

void DropPermissionsOf(BaaTab* tab) {
  for (auto it = Prompts().begin(); it != Prompts().end();) {
    if (it->second->tab() == tab) {
      uint64_t id = it->first;
      it = Prompts().erase(it);
      if (Callbacks().permission_closed) {
        Callbacks().permission_closed(CallbackContext(), tab, id);
      }
    } else {
      ++it;
    }
  }
}

}  // namespace baa

void baa_permission_answer(uint64_t id, bool allow) {
  auto it = baa::Prompts().find(id);
  if (it == baa::Prompts().end()) {
    return;
  }
  auto delegate = it->second->delegate();
  baa::Prompts().erase(it);
  if (!delegate) {
    return;
  }
  if (allow) {
    delegate->Accept(std::monostate());
  } else {
    delegate->Deny(std::monostate());
  }
}

int baa_profile_site_permission(BaaProfile* profile, const char* url, uint32_t kind) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  ContentSettingsType type = baa::TypeOf(kind);
  GURL site(url);
  if (!chrome_profile || type == ContentSettingsType::DEFAULT || !site.is_valid()) {
    return BAA_SITE_ASK;
  }
  switch (HostContentSettingsMapFactory::GetForProfile(chrome_profile)
              ->GetContentSetting(site, site, type)) {
    case CONTENT_SETTING_ALLOW:
      return BAA_SITE_ALLOW;
    case CONTENT_SETTING_BLOCK:
      return BAA_SITE_BLOCK;
    default:
      return BAA_SITE_ASK;
  }
}

void baa_profile_set_site_permission(BaaProfile* profile,
                                     const char* url,
                                     uint32_t kind,
                                     int state) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  ContentSettingsType type = baa::TypeOf(kind);
  GURL site(url);
  if (!chrome_profile || type == ContentSettingsType::DEFAULT || !site.is_valid()) {
    return;
  }
  // "Ask" is going back to the default, not writing an Ask rule.
  ContentSetting setting = state == BAA_SITE_ALLOW   ? CONTENT_SETTING_ALLOW
                           : state == BAA_SITE_BLOCK ? CONTENT_SETTING_BLOCK
                                                     : CONTENT_SETTING_DEFAULT;
  HostContentSettingsMapFactory::GetForProfile(chrome_profile)
      ->SetContentSettingDefaultScope(site, site, type, setting);
}

// Permission prompts and per-site settings. PLAN-chromium-phase4.md step 1.

#include <map>
#include <string>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/task/single_thread_task_runner.h"
#include "chrome/browser/content_settings/host_content_settings_map_factory.h"
#include "chrome/browser/profiles/profile.h"
#include "components/content_settings/core/browser/content_settings_info.h"
#include "components/content_settings/core/browser/content_settings_registry.h"
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
      : tab_(WeakTab(tab)), delegate_(delegate->GetWeakPtr()), id_(NextId()) {
    uint32_t kinds = 0;
    for (const auto& request : delegate->Requests()) {
      kinds |= KindBit(request->request_type());
    }
    origin_ = delegate->GetRequestingOrigin().spec();
    Prompts()[id_] = this;
    kinds_ = kinds;
    Callbacks().permission(CallbackContext(), IdOf(tab_.get()), id_, origin_.c_str(), kinds_);
  }

  ~Prompt() override {
    // Still waiting: Chrome dropped the request (navigation, tab closed).
    BaaTab* tab = tab_.get();
    if (Prompts().erase(id_) && tab && TabAnnounced(tab)) {
      Callbacks().request_closed(CallbackContext(), IdOf(tab), id_);
    }
  }

  base::WeakPtr<Delegate> delegate() const { return delegate_; }
  BaaTab* tab() const { return tab_.get(); }

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
  base::WeakPtr<BaaTab> tab_;
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
    case BAA_PERMISSION_POPUPS:
      return ContentSettingsType::POPUPS;
    case BAA_PERMISSION_SOUND:
      return ContentSettingsType::SOUND;
    case BAA_PERMISSION_JAVASCRIPT:
      return ContentSettingsType::JAVASCRIPT;
    case BAA_PERMISSION_COOKIES:
      return ContentSettingsType::COOKIES;
    case BAA_PERMISSION_AUTOMATIC_DOWNLOADS:
      return ContentSettingsType::AUTOMATIC_DOWNLOADS;
    case BAA_PERMISSION_INSECURE_CONTENT:
      return ContentSettingsType::MIXEDSCRIPT;
    case BAA_PERMISSION_SENSORS:
      return ContentSettingsType::SENSORS;
    case BAA_PERMISSION_WINDOW_MANAGEMENT:
      return ContentSettingsType::WINDOW_MANAGEMENT;
    default:
      return ContentSettingsType::DEFAULT;
  }
}

// BAA_SITE_* <-> ContentSetting; UNSET (and anything unknown) is DEFAULT, which removes a rule.
int StateOf(ContentSetting setting) {
  switch (setting) {
    case CONTENT_SETTING_ALLOW:
      return BAA_SITE_ALLOW;
    case CONTENT_SETTING_BLOCK:
      return BAA_SITE_BLOCK;
    case CONTENT_SETTING_SESSION_ONLY:
      return BAA_SITE_SESSION_ONLY;
    case CONTENT_SETTING_ASK:
      return BAA_SITE_ASK;
    default:
      return BAA_SITE_UNSET;
  }
}

ContentSetting SettingOf(int state) {
  switch (state) {
    case BAA_SITE_ALLOW:
      return CONTENT_SETTING_ALLOW;
    case BAA_SITE_BLOCK:
      return CONTENT_SETTING_BLOCK;
    case BAA_SITE_ASK:
      return CONTENT_SETTING_ASK;
    case BAA_SITE_SESSION_ONLY:
      return CONTENT_SETTING_SESSION_ONLY;
    default:
      return CONTENT_SETTING_DEFAULT;
  }
}

// Chrome's own rules for what a type takes, as a default or as an exception.
bool Takes(ContentSettingsType type, ContentSetting setting, bool as_default) {
  if (setting == CONTENT_SETTING_DEFAULT) {
    return true;  // UNSET
  }
  const content_settings::ContentSettingsInfo* info =
      content_settings::ContentSettingsRegistry::GetInstance()->Get(type);
  return info && (as_default ? info->IsDefaultSettingValid(setting) : info->IsSettingValid(setting));
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

void RestoreChromePermissions(content::WebContents* wc) {
  if (auto* manager = permissions::PermissionRequestManager::FromWebContents(wc)) {
    manager->set_view_factory_for_testing(base::BindRepeating(&permissions::PermissionPrompt::Create));
  }
}

void DropPermissionsOf(BaaTab* tab, bool tell) {
  for (auto it = Prompts().begin(); it != Prompts().end();) {
    if (it->second->tab() == tab) {
      uint64_t id = it->first;
      it = Prompts().erase(it);
      if (tell) {
        Callbacks().request_closed(CallbackContext(), IdOf(tab), id);
      }
    } else {
      ++it;
    }
  }
}

}  // namespace baa

namespace baa {

bool AnswerPermission(uint64_t id, bool allow) {
  auto it = Prompts().find(id);
  if (it == Prompts().end()) {
    return false;
  }
  auto delegate = it->second->delegate();
  Prompts().erase(it);
  if (!delegate) {
    return true;
  }
  if (allow) {
    delegate->Accept(std::monostate());
  } else {
    delegate->Deny(std::monostate());
  }
  return true;
}

}  // namespace baa

int baa_profile_site_permission(BaaProfile* profile, const char* url, uint32_t kind) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  ContentSettingsType type = baa::TypeOf(kind);
  GURL site(url ? url : "");
  if (!chrome_profile || type == ContentSettingsType::DEFAULT || !site.is_valid()) {
    return BAA_SITE_ASK;
  }
  int state = baa::StateOf(
      HostContentSettingsMapFactory::GetForProfile(chrome_profile)->GetContentSetting(site, site, type));
  return state == BAA_SITE_UNSET ? BAA_SITE_ASK : state;
}

void baa_profile_set_site_permission(BaaProfile* profile,
                                     const char* url,
                                     uint32_t kind,
                                     int state) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  ContentSettingsType type = baa::TypeOf(kind);
  GURL site(url ? url : "");
  if (!chrome_profile || type == ContentSettingsType::DEFAULT || !site.is_valid()) {
    return;
  }
  // "Ask" is going back to the default, not writing an Ask rule.
  ContentSetting setting = state == BAA_SITE_ALLOW || state == BAA_SITE_BLOCK
                               ? baa::SettingOf(state)
                               : CONTENT_SETTING_DEFAULT;
  HostContentSettingsMapFactory::GetForProfile(chrome_profile)
      ->SetContentSettingDefaultScope(site, site, type, setting);
}

int baa_profile_site_rules(BaaProfile* profile, uint32_t kind, BaaSiteRuleVisitor visit, void* ctx) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  if (!chrome_profile || !visit || kind == BAA_PERMISSION_OTHER ||
      baa::TypeOf(kind) == ContentSettingsType::DEFAULT) {
    return -1;
  }
  ContentSettingsType type = baa::TypeOf(kind);
  HostContentSettingsMap* map = HostContentSettingsMapFactory::GetForProfile(chrome_profile);
  // An off-the-record map answers with the parent's default as it applies there.
  visit(ctx, "*", baa::StateOf(map->GetDefaultContentSetting(type)));
  int count = 1;
  for (const ContentSettingPatternSource& rule : map->GetSettingsForOneType(type)) {
    // Only what the user set in this profile: not policy, extensions or built-in rules, not
    // the parent's rules (an off-the-record map lists those too), the default, or
    // third-party (secondary pattern) cookie rules.
    int state = baa::StateOf(rule.GetContentSetting());
    if (rule.source != content_settings::ProviderType::kPrefProvider ||
        rule.incognito != chrome_profile->IsOffTheRecord() ||
        rule.primary_pattern == ContentSettingsPattern::Wildcard() ||
        rule.secondary_pattern != ContentSettingsPattern::Wildcard() || rule.IsExpired() ||
        state == BAA_SITE_UNSET) {
      continue;
    }
    visit(ctx, rule.primary_pattern.ToString().c_str(), state);
    ++count;
  }
  return count;
}

bool baa_profile_set_site_rule(BaaProfile* profile, uint32_t kind, const char* pattern, int state) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  ContentSettingsType type = baa::TypeOf(kind);
  if (!chrome_profile || !pattern || kind == BAA_PERMISSION_OTHER || type == ContentSettingsType::DEFAULT ||
      state < BAA_SITE_UNSET || state > BAA_SITE_SESSION_ONLY) {
    return false;
  }
  ContentSetting setting = baa::SettingOf(state);
  bool is_default = std::string_view(pattern) == "*";
  if (!baa::Takes(type, setting, is_default)) {
    return false;
  }
  if (is_default) {
    // An off-the-record map takes no defaults and inherits the parent's.
    HostContentSettingsMapFactory::GetForProfile(baa::DefaultsProfile(chrome_profile))
        ->SetDefaultContentSetting(type, setting);
    return true;
  }
  ContentSettingsPattern primary = ContentSettingsPattern::FromString(pattern);
  if (!primary.IsValid()) {
    return false;
  }
  HostContentSettingsMapFactory::GetForProfile(chrome_profile)
      ->SetContentSettingCustomScope(primary, ContentSettingsPattern::Wildcard(), type, setting);
  return true;
}

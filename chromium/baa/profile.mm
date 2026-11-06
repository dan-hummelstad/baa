// BaaProfile and every baa_profile_* call that is not a feature's own
// (permissions.mm, extensions.mm, net.mm). PLAN-chromium-phase3.md "baa::Profile".

#import <Cocoa/Cocoa.h>

#include <memory>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/logging.h"
#include "base/no_destructor.h"
#include "base/threading/thread_restrictions.h"
#include "base/timer/elapsed_timer.h"
#include "base/values.h"
#include "chrome/browser/browser_process.h"
#include "chrome/browser/profiles/keep_alive/profile_keep_alive_types.h"
#include "chrome/browser/profiles/keep_alive/scoped_profile_keep_alive.h"
#include "chrome/browser/extensions/extension_util.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/baa_hooks.h"
#include "content/public/browser/security_principal.h"
#include "content/public/browser/web_contents.h"
#include "chrome/browser/profiles/profile_destroyer.h"
#include "chrome/browser/profiles/profile_manager.h"
#include "chrome/browser/profiles/profile_observer.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/browser_window/public/create_browser_window.h"
#include "chrome/common/chrome_constants.h"
#include "chrome/common/pref_names.h"
#include "components/prefs/pref_service.h"

struct BaaProfile : ProfileObserver {
  BaaProfile(Profile* profile, bool ephemeral)
      : profile(profile), ephemeral(ephemeral) {
    observation.Observe(profile);
    if (!ephemeral) {
      keep_alive = std::make_unique<ScopedProfileKeepAlive>(
          profile, ProfileKeepAliveOrigin::kBrowserWindow);
    }
    // Printing goes through the system panel (printing::StartBasicPrint);
    // the in-page preview needs a views dialog host.
    profile->GetPrefs()->SetBoolean(prefs::kPrintPreviewDisabled, true);
    // Autofill and password bubbles need a BrowserView (the window has no
    // autofill bubble handler).
    for (const char* name : {"autofill.profile_enabled", "autofill.credit_card_enabled",
                             "autofill.payment_methods_enabled", "credentials_enable_service",
                             "credentials_enable_autosignin"}) {
      baa::SetPrefBool(profile, name, false);
    }
  }

  void OnProfileWillBeDestroyed(Profile*) override {
    observation.Reset();
    profile = nullptr;
    browser = nullptr;
    browser_closed = {};
  }

  // The Browser holding this profile's tabs, made on first need and again
  // after it closed. Null when the profile is gone or Chrome refuses.
  BrowserWindowInterface* GetBrowser() {
    // A Browser whose last tab just closed is on its way out (Window::Close
    // is posted); a tab put in it now would be closed with it.
    if (browser && baa::BrowserIsClosing(browser)) {
      browser = nullptr;
      browser_closed = {};
    }
    if (browser) {
      return browser;
    }
    if (!profile || baa_is_quitting() ||
        GetBrowserWindowCreationStatusForProfile(*profile) !=
            BrowserWindowInterface::CreationStatus::kOk) {
      return nullptr;
    }
    BrowserWindowCreateParams params(BrowserWindowInterface::TYPE_NORMAL,
                                     *profile, /*from_user_gesture=*/false);
    // baa-model is the session, not Chrome's.
    params.should_trigger_session_restore = false;
    params.omit_from_session_restore = true;
    browser = CreateBrowserWindow(std::move(params));
    browser_closed = browser->RegisterBrowserDidClose(base::BindRepeating(
        [](BaaProfile* self, BrowserWindowInterface*) {
          self->browser = nullptr;
          self->browser_closed = {};
        },
        base::Unretained(this)));
    return browser;
  }

  raw_ptr<Profile> profile;
  const bool ephemeral;
  // baa_profile_set_color_schemes.
  baa::ColorScheme website_scheme = baa::ColorScheme::kSystem;
  baa::ColorScheme webui_scheme = baa::ColorScheme::kSystem;
  std::string name;
  std::unique_ptr<ScopedProfileKeepAlive> keep_alive;
  baa::DownloadWatch downloads{profile};
  raw_ptr<BrowserWindowInterface> browser;
  base::CallbackListSubscription browser_closed;
  base::ScopedObservation<Profile, ProfileObserver> observation{this};
};

namespace {

std::vector<std::unique_ptr<BaaProfile>>& Profiles() {
  static base::NoDestructor<std::vector<std::unique_ptr<BaaProfile>>> profiles;
  return *profiles;
}

BaaProfile* AliveProfile(BaaProfile* profile) {
  for (auto& p : Profiles()) {
    if (p.get() == profile) {
      return profile;
    }
  }
  return nullptr;
}

}  // namespace

// ---- C ABI: profiles -----------------------------------------------------------

BaaProfile* baa_profile_open(const char* dir_name) {
  std::string name = dir_name ? dir_name : "";
  if (!g_browser_process || !g_browser_process->profile_manager() ||
      baa_is_quitting() || name.empty() || name.find('/') != std::string::npos ||
      name.find("..") != std::string::npos) {
    return nullptr;
  }
  baa::EnsureStarted();
  for (auto& p : Profiles()) {
    if (!p->ephemeral && p->name == name && p->profile) {
      return p.get();
    }
  }
  // Loading a profile reads its directory; Chrome's UI thread forbids that
  // by default (DisallowUnresponsiveTasks) because it never loads one here.
  // ponytail: the ForTesting scope is the only public one; the real fix is
  // ProfileManager::CreateProfileAsync (an ABI change; risk 5).
  base::ScopedAllowBlockingForTesting allow_blocking;
  base::ElapsedTimer timer;
  ProfileManager* manager = g_browser_process->profile_manager();
  Profile* profile =
      manager->GetProfile(manager->user_data_dir().AppendASCII(name));
  if (!profile) {
    return nullptr;
  }
  LOG(INFO) << "baa: profile_open " << name << " took "
            << timer.Elapsed().InMilliseconds() << " ms";
  auto owned = std::make_unique<BaaProfile>(profile, /*ephemeral=*/false);
  owned->name = name;
  Profiles().push_back(std::move(owned));
  return Profiles().back().get();
}

BaaProfile* baa_profile_open_ephemeral(void) {
  if (!g_browser_process || !g_browser_process->profile_manager() ||
      baa_is_quitting()) {
    return nullptr;
  }
  baa::EnsureStarted();
  base::ScopedAllowBlockingForTesting allow_blocking;
  // The Default profile, never GetLastUsedProfile(): Chrome moves that to
  // whatever Space was active.
  ProfileManager* manager = g_browser_process->profile_manager();
  Profile* base =
      manager->GetProfile(manager->user_data_dir().AppendASCII(chrome::kInitialProfile));
  if (!base) {
    return nullptr;
  }
  Profile* profile = base->GetOffTheRecordProfile(
      Profile::OTRProfileID::CreateUnique("baa::space"),
      /*create_if_needed=*/true);
  if (!profile) {
    return nullptr;
  }
  Profiles().push_back(std::make_unique<BaaProfile>(profile, /*ephemeral=*/true));
  return Profiles().back().get();
}

void baa_profile_close(BaaProfile* profile) {
  if (!AliveProfile(profile)) {
    return;
  }
  Profile* chrome_profile = profile->profile;
  baa::AnswerPendingOf(chrome_profile);
  baa::CloseTabsOf(chrome_profile);
  bool ephemeral = profile->ephemeral;
  std::erase_if(Profiles(), [&](auto& p) { return p.get() == profile; });
  if (chrome_profile) {
    // The tabs are gone but their Browsers close by a posted task, and
    // destroying an OTR profile while one lives CHECKs. Ours go now.
    baa::DestroyBrowsersOf(chrome_profile);
    if (ephemeral) {
      ProfileDestroyer::DestroyOTRProfileWhenAppropriate(chrome_profile);
    }
  }
}

// ponytail: M3 stub (WP-C5); autocomplete is not sent yet.
void baa_profile_autocomplete(BaaProfile*, uint64_t, const char*) {}


namespace {

// The pref if it is registered, user-modifiable and of `type`.
PrefService* UserPref(PrefService* prefs, const char* name, base::Value::Type type) {
  const PrefService::Preference* pref = prefs && name ? prefs->FindPreference(name) : nullptr;
  return pref && pref->IsUserModifiable() && pref->GetType() == type ? prefs : nullptr;
}

// NULL `profile` is Local State; a handle that is gone gives null.
PrefService* PrefsOf(BaaProfile* profile) {
  if (!profile) {
    return g_browser_process->local_state();
  }
  return AliveProfile(profile) ? profile->profile->GetPrefs() : nullptr;
}

}  // namespace

namespace baa {

bool SetPrefBool(Profile* profile, const char* name, bool value) {
  PrefService* prefs = UserPref(profile ? profile->GetPrefs() : nullptr, name, base::Value::Type::BOOLEAN);
  if (prefs) {
    prefs->SetBoolean(name, value);
  }
  return prefs;
}

}  // namespace baa

bool baa_pref_set_bool(BaaProfile* profile, const char* name, bool value) {
  PrefService* prefs = UserPref(PrefsOf(profile), name, base::Value::Type::BOOLEAN);
  LOG(INFO) << "baa: set_pref_bool " << name << " = " << value << " -> " << (prefs != nullptr);
  if (!prefs) {
    return false;
  }
  // Also tells the renderers (userScripts, extension APIs) about the mode.
  if (profile && !profile->ephemeral && std::string_view(name) == "extensions.ui.developer_mode") {
    extensions::util::SetDeveloperModeForProfile(profile->profile, value);
  } else {
    prefs->SetBoolean(name, value);
  }
  return true;
}

bool baa_pref_set_int(BaaProfile* profile, const char* name, int value) {
  PrefService* prefs = UserPref(PrefsOf(profile), name, base::Value::Type::INTEGER);
  LOG(INFO) << "baa: set_pref_int " << name << " = " << value << " -> " << (prefs != nullptr);
  if (prefs) {
    prefs->SetInteger(name, value);
  }
  return prefs;
}

bool baa_pref_set_string(BaaProfile* profile, const char* name, const char* value) {
  PrefService* prefs = value ? UserPref(PrefsOf(profile), name, base::Value::Type::STRING) : nullptr;
  LOG(INFO) << "baa: set_pref_string " << name << " -> " << (prefs != nullptr);
  if (prefs) {
    prefs->SetString(name, value);
  }
  return prefs;
}

bool baa_pref_clear(BaaProfile* profile, const char* name) {
  PrefService* prefs = PrefsOf(profile);
  const PrefService::Preference* pref = prefs && name ? prefs->FindPreference(name) : nullptr;
  LOG(INFO) << "baa: clear_pref " << (name ? name : "") << " -> " << (pref && pref->IsUserModifiable());
  if (!pref || !pref->IsUserModifiable()) {
    return false;
  }
  if (profile && !profile->ephemeral && std::string_view(name) == "extensions.ui.developer_mode") {
    extensions::util::SetDeveloperModeForProfile(profile->profile, false);  // its default
  }
  prefs->ClearPref(name);
  return true;
}

void baa_profile_set_color_schemes(BaaProfile* profile, int website, int webui) {
  if (!AliveProfile(profile)) {
    return;
  }
  profile->website_scheme = baa::ColorSchemeOf(website);
  profile->webui_scheme = baa::ColorSchemeOf(webui);
  baa::ReapplyColorSchemes(profile->profile);
}

namespace baa {

namespace {

// Chrome asks this for every WebPreferences recompute (navigations, theme changes, font prefs),
// so nothing is cached per tab. patches/baa/color-scheme-hook.patch.
bool PreferredColorScheme(content::WebContents* contents,
                          const content::SecurityPrincipal& principal,
                          blink::mojom::PreferredColorScheme* scheme) {
  BaaProfile* profile = ProfileFor(Profile::FromBrowserContext(contents->GetBrowserContext()));
  if (!profile) {
    return false;
  }
  // IsWebUI covers chrome:, chrome-untrusted:, chrome-search: and devtools: (the DevTools front-end
  // is Chrome's own page, so it follows the WebUI choice).
  ColorScheme choice = principal.IsWebUI() ? profile->webui_scheme : profile->website_scheme;
  if (choice == ColorScheme::kSystem) {
    return false;
  }
  *scheme = choice == ColorScheme::kDark ? blink::mojom::PreferredColorScheme::kDark
                                         : blink::mojom::PreferredColorScheme::kLight;
  return true;
}

}  // namespace

void InstallColorSchemeHook() {
  g_baa_preferred_color_scheme = &PreferredColorScheme;
}

Profile* DefaultsProfile(Profile* profile) {
  return profile->IsOffTheRecord() ? profile->GetOriginalProfile() : profile;
}

}  // namespace baa

namespace baa {

Profile* ChromeProfileOf(BaaProfile* profile) {
  return AliveProfile(profile) ? profile->profile.get() : nullptr;
}

BaaProfile* ProfileFor(Profile* profile) {
  for (auto& p : Profiles()) {
    if (p->profile == profile) {
      return p.get();
    }
  }
  return nullptr;
}

BrowserWindowInterface* BrowserOf(BaaProfile* profile) {
  return AliveProfile(profile) ? profile->GetBrowser() : nullptr;
}

BrowserWindowInterface* ExistingBrowserOf(BaaProfile* profile) {
  return AliveProfile(profile) && profile->browser && !BrowserIsClosing(profile->browser)
             ? profile->browser.get()
             : nullptr;
}

}  // namespace baa

// Extensions: list, change, toolbar popups, and the ExtensionsContainer the
// window registers. PLAN-chromium-phase4.md step 6.

#import <Cocoa/Cocoa.h>

#include <algorithm>
#include <memory>
#include <string>
#include <vector>

#include "baa/baa.h"
#include "baa/internal.h"
#include "base/functional/bind.h"
#include "base/no_destructor.h"
#include "base/files/file_path.h"
#include "base/scoped_observation.h"
#include "base/strings/utf_string_conversions.h"
#include "chrome/browser/extensions/extension_action_runner.h"
#include "chrome/browser/extensions/extension_view.h"
#include "chrome/browser/extensions/extension_view_host.h"
#include "chrome/browser/extensions/extension_view_host_factory.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/common/pref_names.h"
#include "components/prefs/pref_service.h"
#include "chrome/browser/ui/extensions/extension_popup_types.h"
#include "chrome/browser/ui/extensions/extensions_container.h"
#include "components/sessions/content/session_tab_helper.h"
#include "content/public/browser/render_frame_host.h"
#include "content/public/browser/render_widget_host_view.h"
#include "content/public/browser/web_contents.h"
#include "extensions/browser/disable_reason.h"
#include "extensions/browser/extension_action.h"
#include "extensions/browser/extension_action_manager.h"
#include "extensions/browser/extension_prefs.h"
#include "extensions/browser/extension_registrar.h"
#include "extensions/browser/extension_registry.h"
#include "extensions/browser/extension_registry_observer.h"
#include "extensions/browser/extension_util.h"
#include "extensions/browser/permissions/scripting_permissions_modifier.h"
#include "extensions/browser/permissions_manager.h"
#include "extensions/browser/unpacked_installer.h"
#include "extensions/browser/uninstall_reason.h"
#include "extensions/common/api/extension_action/action_info.h"
#include "extensions/common/extension.h"
#include "extensions/common/icons/extension_icon_set.h"
#include "extensions/common/manifest.h"
#include "extensions/common/manifest_constants.h"
#include "extensions/common/manifest_handlers/icons_handler.h"
#include "extensions/common/manifest_handlers/options_page_info.h"
#include "extensions/common/permissions/permission_message_provider.h"
#include "extensions/common/permissions/permission_set.h"
#include "extensions/common/permissions/permissions_data.h"
#include "ui/gfx/geometry/size.h"
#include "ui/gfx/native_ui_types.h"

namespace baa {
namespace {

std::string IconPath(const extensions::Extension& ext) {
  const extensions::ActionInfo* action = extensions::ActionInfo::GetExtensionActionInfo(&ext);
  std::string relative;
  if (action) {
    relative = action->default_icon.Get(32, ExtensionIconSet::Match::kBigger);
  }
  if (relative.empty()) {
    relative = extensions::IconsInfo::GetIcons(&ext).Get(32, ExtensionIconSet::Match::kBigger);
  }
  if (relative.empty()) {
    return "";
  }
  return ext.path().AppendASCII(relative).AsUTF8Unsafe();
}

// The messages and host access as the extensions page computes them
// (developer_private's ExtensionInfoGenerator).
std::string PermissionLines(Profile* profile, const extensions::Extension& ext, int* site_access) {
  extensions::ExtensionPrefs* prefs = extensions::ExtensionPrefs::Get(profile);
  extensions::PermissionsManager* manager = extensions::PermissionsManager::Get(profile);
  std::unique_ptr<const extensions::PermissionSet> granted = prefs->GetGrantedPermissions(ext.id());
  if (!granted) {
    granted = std::make_unique<extensions::PermissionSet>();
  }
  const extensions::PermissionMessageProvider* provider = extensions::PermissionMessageProvider::Get();
  bool runtime_hosts = manager->CanAffectExtension(ext);

  extensions::PermissionIDSet ids;
  if (!runtime_hosts) {
    // Without runtime host permissions everything is one list.
    ids = provider->GetAllPermissionIDs(*granted, ext.GetType());
    *site_access = BAA_SITE_ACCESS_NONE;
  } else {
    extensions::PermissionSet non_host(granted->apis().Clone(), granted->manifest_permissions().Clone(),
                                       extensions::URLPatternSet(), extensions::URLPatternSet());
    ids = provider->GetAllPermissionIDs(non_host, ext.GetType());
    std::unique_ptr<const extensions::PermissionSet> runtime_granted;
    if (!manager->HasWithheldHostPermissions(ext)) {
      *site_access = BAA_SITE_ACCESS_ON_ALL_SITES;
    } else {
      runtime_granted = prefs->GetRuntimeGrantedPermissions(ext.id());
      if (!runtime_granted || runtime_granted->effective_hosts().is_empty()) {
        *site_access = BAA_SITE_ACCESS_ON_CLICK;
      } else if (runtime_granted->ShouldWarnAllHosts(false)) {
        *site_access = BAA_SITE_ACCESS_ON_ALL_SITES;
      } else {
        *site_access = BAA_SITE_ACCESS_ON_SPECIFIC_SITES;
      }
    }
  }
  std::string lines;
  for (const auto& message : provider->GetPermissionMessages(ids)) {
    if (!lines.empty()) {
      lines += "\n";
    }
    lines += base::UTF16ToUTF8(message.message());
  }
  return lines;
}

// ---- Popups --------------------------------------------------------------------

class Popup;
std::vector<Popup*>& Popups() {
  static base::NoDestructor<std::vector<Popup*>> popups;
  return *popups;
}

// Deletes a popup once; a posted deletion may find it already gone (quit).
void DestroyPopup(Popup* popup);

class Popup : public extensions::ExtensionView,
              public extensions::ExtensionRegistryObserver {
 public:
  Popup(Profile* profile, BrowserWindowInterface* browser, std::unique_ptr<extensions::ExtensionViewHost> host)
      : profile_(profile), host_(std::move(host)) {
    registry_.Observe(extensions::ExtensionRegistry::Get(profile));
    Popups().push_back(this);
    // The host's delegate holds the Browser: when it goes, so does the popup,
    // now (not from a task).
    browser_closed_ = browser->RegisterBrowserDidClose(
        base::BindRepeating([](Popup* self, BrowserWindowInterface*) { self->Closed(); }, base::Unretained(this)));
  }

  ~Popup() override {
    std::erase(Popups(), this);
    host_.reset();
  }

  void Start() {
    tab_ = CreateBareTab(host_->host_contents(),
                         base::BindOnce([](Popup* self) { delete self; }, base::Unretained(this)));
    // Before the renderer exists: the host dereferences its view unchecked.
    host_->set_view(this);
    host_->SetCloseHandler(base::BindOnce(&Popup::OnHostClose, base::Unretained(this)));
    host_->CreateRendererSoon();
  }

  BaaTab* tab() const { return tab_; }

  // extensions::ExtensionView:
  gfx::NativeView GetNativeView() override { return host_->host_contents()->GetNativeView(); }
  void ResizeDueToAutoResize(content::WebContents*, const gfx::Size& size) override {
    if (tab_) {
      Callbacks().resized(CallbackContext(), IdOf(tab_), size.width(), size.height());
    }
  }
  void RenderFrameCreated(content::RenderFrameHost* frame) override {
    if (content::RenderWidgetHostView* view = frame->GetView()) {
      view->EnableAutoResize(gfx::Size(25, 25), gfx::Size(800, 600));
    }
  }
  bool HandleKeyboardEvent(content::WebContents*, const input::NativeWebKeyboardEvent& event) override {
    return OfferKey(event, true);
  }
  void OnLoaded() override { SendTabState(tab_); }

  // extensions::ExtensionRegistryObserver:
  void OnExtensionUnloaded(content::BrowserContext*, const extensions::Extension* extension,
                           extensions::UnloadedExtensionReason) override {
    if (extension->id() == host_->extension_id()) {
      Closed();
    }
  }

 private:
  void OnHostClose(extensions::ExtensionHost*) { Closed(); }

  // Chrome (the page, or its extension going away) closed the popup.
  void Closed() {
    if (closed_) {
      return;
    }
    closed_ = true;
    BaaTab* tab = tab_;
    tab_ = nullptr;
    BareTabClosedByChrome(tab);
    browser_closed_ = {};
    // The contents go now (they hold the Browser or the extension); the shell
    // goes from a task, not from inside the host's or the Browser's callback.
    host_.reset();
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(FROM_HERE, base::BindOnce(&DestroyPopup, this));
  }

  raw_ptr<Profile> profile_;
  std::unique_ptr<extensions::ExtensionViewHost> host_;
  raw_ptr<BaaTab> tab_ = nullptr;
  bool closed_ = false;
  base::CallbackListSubscription browser_closed_;
  base::ScopedObservation<extensions::ExtensionRegistry, extensions::ExtensionRegistryObserver> registry_{this};
};

void DestroyPopup(Popup* popup) {
  if (std::find(Popups().begin(), Popups().end(), popup) != Popups().end()) {
    delete popup;
  }
}

// The ExtensionsContainer of a baa window: enough that Chrome's extension code
// can ask it, and answer "no" (there is no toolbar).
class Container : public ExtensionsContainer {
 public:
  explicit Container(BrowserWindowInterface* browser) : browser_(browser) {}
  ToolbarActionViewModel* GetActionForId(const std::string&) override { return nullptr; }
  void HideActivePopup() override {}
  void CloseExtensionsMenuIfOpen() override {}
  bool ShowToolbarActionPopupForAPICall(const std::string&, ShowPopupCallback) override {
    return false;
  }
  void ToggleExtensionsMenu() override {}
  bool HasAnyExtensions() const override {
    return !extensions::ExtensionRegistry::Get(browser_->GetProfile())->enabled_extensions().empty();
  }

 private:
  raw_ptr<BrowserWindowInterface> browser_;
};

}  // namespace

void DestroyAllPopups() {
  while (!Popups().empty()) {
    delete Popups().back();
  }
}

std::unique_ptr<ExtensionsContainer> MakeExtensionsContainer(BrowserWindowInterface* browser) {
  return std::make_unique<Container>(browser);
}

}  // namespace baa

int baa_profile_extensions(BaaProfile* profile, BaaExtensionVisitor visit, void* ctx) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  if (!chrome_profile) {
    return -1;
  }
  extensions::ExtensionRegistry* registry = extensions::ExtensionRegistry::Get(chrome_profile);
  int count = 0;
  for (const auto& ref : registry->GenerateInstalledExtensionsSet()) {
    const extensions::Extension& ext = *ref;
    if (extensions::Manifest::IsComponentLocation(ext.location()) || ext.is_theme()) {
      continue;
    }
    const extensions::ActionInfo* action = extensions::ActionInfo::GetExtensionActionInfo(&ext);
    int site_access = BAA_SITE_ACCESS_NONE;
    const std::string* desc = ext.manifest()->FindStringPath(extensions::manifest_keys::kDescription);
    std::string name = ext.name(), description = desc ? *desc : "", version = ext.VersionString();
    std::string icon = baa::IconPath(ext);
    std::string popup = action ? action->default_popup_url.spec() : "";
    std::string options = extensions::OptionsPageInfo::GetOptionsPage(&ext).spec();
    std::string permissions = baa::PermissionLines(chrome_profile, ext, &site_access);
    BaaExtension out = {};
    out.id = ext.id().c_str();
    out.name = name.c_str();
    out.description = description.c_str();
    out.version = version.c_str();
    out.icon_path = icon.c_str();
    out.popup_url = popup.c_str();
    out.options_url = options.c_str();
    out.permissions = permissions.c_str();
    out.site_access = site_access;
    out.enabled = registry->enabled_extensions().Contains(ext.id());
    out.unpacked = extensions::Manifest::IsUnpackedLocation(ext.location());
    visit(ctx, &out);
    ++count;
  }
  return count;
}

bool baa_profile_extension_change(BaaProfile* profile, const char* extension_id, int change, int value) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  if (!chrome_profile || !extension_id) {
    return false;
  }
  extensions::ExtensionRegistry* registry = extensions::ExtensionRegistry::Get(chrome_profile);
  std::string id = extension_id;
  const extensions::Extension* ext = registry->GetInstalledExtension(id);
  if (!ext || extensions::Manifest::IsComponentLocation(ext->location())) {
    return false;
  }
  extensions::ExtensionRegistrar* registrar = extensions::ExtensionRegistrar::Get(chrome_profile);
  switch (change) {
    case BAA_EXTENSION_RELOAD:
      if (!extensions::Manifest::IsUnpackedLocation(ext->location())) {
        return false;
      }
      registrar->ReloadExtensionWithQuietFailure(id);
      return true;
    case BAA_EXTENSION_ENABLE:
      if (value) {
        registrar->EnableExtension(id);
      } else {
        registrar->DisableExtension(id, {extensions::disable_reason::DISABLE_USER_ACTION});
      }
      return true;
    case BAA_EXTENSION_REMOVE: {
      std::u16string error;
      return registrar->UninstallExtension(id, extensions::UNINSTALL_REASON_USER_INITIATED, &error);
    }
    case BAA_EXTENSION_SITE_ACCESS: {
      extensions::PermissionsManager* manager = extensions::PermissionsManager::Get(chrome_profile);
      if (!manager->CanAffectExtension(*ext)) {
        return false;
      }
      extensions::ScriptingPermissionsModifier modifier(chrome_profile, base::WrapRefCounted(ext));
      switch (value) {
        case BAA_SITE_ACCESS_ON_CLICK:
          modifier.SetWithholdHostPermissions(true);
          modifier.RemoveAllGrantedHostPermissions();
          return true;
        case BAA_SITE_ACCESS_ON_SPECIFIC_SITES:
          if (manager->HasBroadGrantedHostPermissions(*ext)) {
            modifier.RemoveBroadGrantedHostPermissions();
          }
          modifier.SetWithholdHostPermissions(true);
          return true;
        case BAA_SITE_ACCESS_ON_ALL_SITES:
          modifier.SetWithholdHostPermissions(false);
          return true;
      }
      return false;
    }
  }
  return false;
}

BaaTabId baa_extension_popup(BaaProfile* profile, const char* extension_id) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  BrowserWindowInterface* browser = baa::ExistingBrowserOf(profile);
  if (!chrome_profile || !browser || !extension_id || baa_is_quitting()) {
    return 0;
  }
  const extensions::Extension* ext =
      extensions::ExtensionRegistry::Get(chrome_profile)->enabled_extensions().GetByID(extension_id);
  // An OTR profile only runs the extensions the user allowed in incognito.
  if (ext && chrome_profile->IsOffTheRecord() && !extensions::util::IsIncognitoEnabled(ext, chrome_profile)) {
    return 0;
  }
  content::WebContents* active = browser->GetTabStripModel()->GetActiveWebContents();
  if (!ext || !active) {
    return 0;
  }
  extensions::ExtensionActionRunner* runner = extensions::ExtensionActionRunner::GetForWebContents(active);
  if (!runner) {
    return 0;
  }
  if (runner->RunAction(ext, /*grant_tab_permissions=*/true) != extensions::ExtensionAction::ShowAction::kShowPopup) {
    return 0;
  }
  extensions::ExtensionAction* action =
      extensions::ExtensionActionManager::Get(chrome_profile)->GetExtensionAction(*ext);
  if (!action) {
    return 0;
  }
  GURL url = action->GetPopupUrl(sessions::SessionTabHelper::IdForTab(active).id());
  std::unique_ptr<extensions::ExtensionViewHost> host =
      extensions::ExtensionViewHostFactory::CreatePopupHost(*ext, url, browser);
  if (!host) {
    return 0;
  }
  auto* popup = new baa::Popup(chrome_profile, browser, std::move(host));
  popup->Start();
  return baa::IdOf(popup->tab());
}

void baa_profile_load_unpacked(BaaProfile* profile, uint64_t id, const char* path) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  baa::BeginRequest(baa::RequestKind::kLoadUnpacked, id, chrome_profile);
  const char* error = !chrome_profile || !path || !*path ? "No such profile or path"
                      : !chrome_profile->GetPrefs()->GetBoolean(prefs::kExtensionsUIDeveloperMode)
                          ? "Must be in developer mode to load unpacked extensions."
                          : nullptr;
  if (error) {
    baa::FinishRequestLater(baa::RequestKind::kLoadUnpacked, id, false, error);
    return;
  }
  auto installer = extensions::UnpackedInstaller::Create(chrome_profile);
  installer->set_be_noisy_on_failure(false);
  // Posted: the installer reports some failures before Load returns.
  installer->set_completion_callback(base::BindOnce(
      [](uint64_t id, const extensions::Extension* extension, const base::FilePath&, const std::u16string& error) {
        std::string message = extension ? extension->id() : base::UTF16ToUTF8(error);
        baa::FinishRequestLater(baa::RequestKind::kLoadUnpacked, id, extension != nullptr,
                                message.empty() ? "Could not load the extension" : message);
      },
      id));
  installer->Load(base::FilePath(path));
}

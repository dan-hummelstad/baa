// Extensions: list, change, toolbar popups, and the ExtensionsContainer the
// window registers. PLAN-chromium-phase4.md step 6.

#import <Cocoa/Cocoa.h>

#include <algorithm>
#include <map>
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
#include "chrome/browser/extensions/extension_action_dispatcher.h"
#include "chrome/browser/extensions/extension_action_runner.h"
#include "chrome/browser/extensions/extension_view.h"
#include "chrome/browser/extensions/extension_view_host.h"
#include "chrome/browser/extensions/extension_view_host_factory.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/common/pref_names.h"
#include "components/prefs/pref_service.h"
// Like extensions_container.h, without a GN dep (//chrome/browser/ui/extensions links them).
#include "chrome/browser/ui/extensions/extension_enable_flow.h"
#include "chrome/browser/ui/extensions/extension_enable_flow_delegate.h"
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
#include "extensions/browser/install_approval.h"
#include "extensions/browser/webstore_installer.h"
#include "base/task/single_thread_task_runner.h"
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
#include "third_party/skia/include/core/SkColor.h"
#include "ui/gfx/geometry/size.h"
#include "ui/gfx/native_ui_types.h"

namespace baa {
namespace {

// Chrome's own component extensions stay out of baa's lists. Helium's uBlock Origin is the one
// a user sees, pauses and opens; Chrome's policy still refuses to uninstall it.
bool HiddenExtension(const extensions::Extension& ext) {
  return extensions::Manifest::IsComponentLocation(ext.location()) &&
         !extensions::Manifest::IsUBlockComponent(ext.id());
}

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

// ---- Re-enable -----------------------------------------------------------------

class Reenable;
std::vector<std::unique_ptr<Reenable>>& Reenables() {
  static base::NoDestructor<std::vector<std::unique_ptr<Reenable>>> reenables;
  return *reenables;
}

void DestroyReenable(Reenable* reenable);

// One BAA_EXTENSION_ENABLE through Chrome's ExtensionEnableFlow: it enables at once unless the
// extension was disabled for asking more than the user approved (an update), and then asks with
// the install prompt (asks.mm's TakeInstall draws it) and enables only on accept.
class Reenable final : public ExtensionEnableFlowDelegate {
 public:
  Reenable(BaaProfile* owner, Profile* profile, const std::string& id)
      : owner_(owner), profile_(profile), flow_(profile, id, this) {}

  BaaProfile* owner() const { return owner_; }
  Profile* profile() const { return profile_; }
  const std::string& id() const { return flow_.extension_id(); }
  bool done() const { return state_ != State::kPending; }

  void DestroyLater() {
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(
                       [](base::WeakPtr<Reenable> reenable) {
                         if (reenable) {
                           DestroyReenable(reenable.get());
                         }
                       },
                       weak_factory_.GetWeakPtr()));
  }

  // True when the extension is enabled now or its prompt is up. The delegate may run inside
  // Start*: the caller deletes a done flow after this returns, never the callback.
  bool Start(content::WebContents* anchor) {
    starting_ = true;
    if (anchor) {
      flow_.StartForWebContents(anchor);
    } else {
      flow_.Start();
    }
    starting_ = false;
    return state_ != State::kAborted;
  }

  // ExtensionEnableFlowDelegate:
  void ExtensionEnableFlowFinished() override { End(State::kFinished); }
  void ExtensionEnableFlowAborted(bool) override { End(State::kAborted); }

 private:
  enum class State { kPending, kFinished, kAborted };

  // Runs inside the flow: deleted from a task (Start's caller deletes it when it ran inside Start).
  void End(State state) {
    state_ = state;
    if (!starting_) {
      DestroyLater();
    }
  }

  const raw_ptr<BaaProfile> owner_;
  const raw_ptr<Profile> profile_;
  ExtensionEnableFlow flow_;
  State state_ = State::kPending;
  bool starting_ = false;
  base::WeakPtrFactory<Reenable> weak_factory_{this};
};

void DestroyReenable(Reenable* reenable) {
  std::erase_if(Reenables(), [&](const auto& r) { return r.get() == reenable; });
}

bool StartReenable(BaaProfile* owner, Profile* chrome_profile, const std::string& id) {
  // An incognito Space shares its parent's extensions.
  Profile* profile = chrome_profile->GetOriginalProfile();
  extensions::ExtensionRegistry* registry = extensions::ExtensionRegistry::Get(profile);
  if (registry->enabled_extensions().Contains(id)) {
    return true;
  }
  // Only a disabled extension starts the flow: it would reload a terminated one, and never call
  // back for one that is gone.
  if (!registry->disabled_extensions().GetByID(id)) {
    return false;
  }
  for (const auto& r : Reenables()) {
    if (r->profile() == profile && r->id() == id) {
      return true;  // its prompt is up
    }
  }
  content::WebContents* anchor = nullptr;
  if (BrowserWindowInterface* browser = ExistingBrowserOf(owner)) {
    content::WebContents* active = browser->GetTabStripModel()->GetActiveWebContents();
    if (TabAnnounced(TabFor(active))) {
      anchor = active;
    }
  }
  // With no tab to show the prompt on, Chrome would build its views dialog with no BrowserView:
  // an extension that asks for more stays disabled.
  if (!anchor && extensions::ExtensionPrefs::Get(profile)->DidExtensionEscalatePermissions(id)) {
    return false;
  }
  Reenables().push_back(std::make_unique<Reenable>(owner, profile, id));
  Reenable* reenable = Reenables().back().get();
  bool ok = reenable->Start(anchor);
  // ponytail: a flow that neither finishes nor prompts (a locked profile, which baa never has)
  // waits for baa_profile_close.
  if (reenable->done()) {
    DestroyReenable(reenable);
  }
  return ok;
}

// ---- extensions_changed ----------------------------------------------------------

// Tells the app to list a profile's extensions again: one callback per turn, however many
// changes (an update disabling an extension for new permissions shows up this way).
class ExtensionsWatch final : public extensions::ExtensionRegistryObserver {
 public:
  ExtensionsWatch(BaaProfile* owner, Profile* profile) : owner_(owner) {
    registry_.Observe(extensions::ExtensionRegistry::Get(profile));
  }

  // extensions::ExtensionRegistryObserver:
  void OnExtensionLoaded(content::BrowserContext*, const extensions::Extension*) override { Changed(); }
  void OnExtensionUnloaded(content::BrowserContext*, const extensions::Extension*,
                           extensions::UnloadedExtensionReason) override {
    Changed();
  }
  void OnExtensionInstalled(content::BrowserContext*, const extensions::Extension*, bool) override {
    Changed();
  }
  void OnExtensionUninstalled(content::BrowserContext*, const extensions::Extension* extension,
                              extensions::UninstallReason) override {
    // A flow of the extension waits on nothing now; deleted from a task, as this may run
    // inside one.
    for (const auto& r : Reenables()) {
      if (r->owner() == owner_ && r->id() == extension->id()) {
        r->DestroyLater();
      }
    }
    Changed();
  }
  void OnShutdown(extensions::ExtensionRegistry*) override { registry_.Reset(); }

 private:
  void Changed() {
    if (pending_) {
      return;
    }
    pending_ = true;
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&ExtensionsWatch::Send, weak_factory_.GetWeakPtr()));
  }
  void Send() {
    pending_ = false;
    Callbacks().extensions_changed(CallbackContext(), owner_);
  }

  const raw_ptr<BaaProfile> owner_;
  bool pending_ = false;
  base::ScopedObservation<extensions::ExtensionRegistry, extensions::ExtensionRegistryObserver> registry_{this};
  base::WeakPtrFactory<ExtensionsWatch> weak_factory_{this};
};

std::map<BaaProfile*, std::unique_ptr<ExtensionsWatch>>& ExtensionsWatches() {
  static base::NoDestructor<std::map<BaaProfile*, std::unique_ptr<ExtensionsWatch>>> watches;
  return *watches;
}

// ---- extension_action ----------------------------------------------------------

class ActionWatch;
std::map<extensions::ExtensionActionDispatcher*, std::unique_ptr<ActionWatch>>& ActionWatches() {
  static base::NoDestructor<std::map<extensions::ExtensionActionDispatcher*, std::unique_ptr<ActionWatch>>>
      watches;
  return *watches;
}

// One per dispatcher: a profile and its incognito profiles share one. It lives until the
// dispatcher shuts down, since its observer list is unchecked.
class ActionWatch final : public extensions::ExtensionActionDispatcher::Observer {
 public:
  explicit ActionWatch(extensions::ExtensionActionDispatcher* dispatcher) : dispatcher_(dispatcher) {
    dispatcher_->AddObserver(this);
  }
  ~ActionWatch() override { dispatcher_->RemoveObserver(this); }

  // extensions::ExtensionActionDispatcher::Observer:
  void OnExtensionActionUpdated(extensions::ExtensionAction* action, content::WebContents* contents,
                                content::BrowserContext* context) override {
    const std::string id = action->extension_id();
    if (contents) {
      BaaTab* tab = TabFor(contents);
      BaaProfile* owner = ProfileFor(Profile::FromBrowserContext(contents->GetBrowserContext()));
      if (owner && TabAnnounced(tab)) {
        Callbacks().extension_action(CallbackContext(), owner, id.c_str(), IdOf(tab));
      }
      return;
    }
    if (!context) {
      return;
    }
    // The default for every tab: each Space of the profile hears it, the incognito ones too.
    Profile* original = Profile::FromBrowserContext(context)->GetOriginalProfile();
    std::vector<BaaProfile*> owners;
    for (const auto& [owner, watch] : ExtensionsWatches()) {
      Profile* profile = ChromeProfileOf(owner);
      if (profile && profile->GetOriginalProfile() == original) {
        owners.push_back(owner);
      }
    }
    for (BaaProfile* owner : owners) {
      Callbacks().extension_action(CallbackContext(), owner, id.c_str(), 0);
    }
  }
  void OnShuttingDown() override {
    extensions::ExtensionActionDispatcher* dispatcher = dispatcher_;
    ActionWatches().erase(dispatcher);  // deletes this
  }

 private:
  const raw_ptr<extensions::ExtensionActionDispatcher> dispatcher_;
};

void WatchActions(Profile* profile) {
  extensions::ExtensionActionDispatcher* dispatcher = extensions::ExtensionActionDispatcher::Get(profile);
  if (dispatcher && !ActionWatches().contains(dispatcher)) {
    ActionWatches()[dispatcher] = std::make_unique<ActionWatch>(dispatcher);
  }
}

// SkColor is ARGB; the ABI's colours are 0xRRGGBBAA.
uint32_t Rgba(SkColor color) {
  return (SkColorGetR(color) << 24) | (SkColorGetG(color) << 16) | (SkColorGetB(color) << 8) | SkColorGetA(color);
}

// The ExtensionsContainer of a baa window: enough that Chrome's extension code
// can ask it, and answer "no" (there is no toolbar).
class Container final : public ExtensionsContainer {
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

void WatchExtensions(BaaProfile* owner, Profile* profile) {
  if (profile->IsOffTheRecord()) {
    return;
  }
  ExtensionsWatches()[owner] = std::make_unique<ExtensionsWatch>(owner, profile);
  WatchActions(profile);
}

void UnwatchExtensions(BaaProfile* owner) {
  ExtensionsWatches().erase(owner);
  // A pending prompt's answer is bound to the flow's WeakPtr: dropping it is safe.
  std::erase_if(Reenables(), [&](const auto& r) { return r->owner() == owner; });
}

void DeleteExtensionsContainer::operator()(ExtensionsContainer* container) const {
  delete static_cast<Container*>(container);
}

ExtensionsContainerPtr MakeExtensionsContainer(BrowserWindowInterface* browser) {
  return ExtensionsContainerPtr(new Container(browser));
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
    if (baa::HiddenExtension(ext) || ext.is_theme()) {
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
    out.needs_approval =
        !out.enabled && extensions::ExtensionPrefs::Get(chrome_profile)->DidExtensionEscalatePermissions(ext.id());
    visit(ctx, &out);
    ++count;
  }
  return count;
}

int baa_profile_extension_actions(BaaProfile* profile, BaaTabId tab, BaaActionVisitor visit, void* ctx) {
  Profile* chrome_profile = baa::ChromeProfileOf(profile);
  if (!chrome_profile) {
    return -1;
  }
  int tab_id = extensions::ExtensionAction::kDefaultTabId;
  if (tab) {
    content::WebContents* contents = baa::ContentsOf(baa::TabFromId(tab));
    if (!contents || Profile::FromBrowserContext(contents->GetBrowserContext()) != chrome_profile) {
      return -1;
    }
    tab_id = sessions::SessionTabHelper::IdForTab(contents).id();
  }
  extensions::ExtensionActionManager* manager = extensions::ExtensionActionManager::Get(chrome_profile);
  int count = 0;
  for (const auto& ext : extensions::ExtensionRegistry::Get(chrome_profile)->enabled_extensions()) {
    if (baa::HiddenExtension(*ext) ||
        (chrome_profile->IsOffTheRecord() && !extensions::util::IsIncognitoEnabled(ext->id(), chrome_profile))) {
      continue;
    }
    extensions::ExtensionAction* action = manager->GetExtensionAction(*ext);
    if (!action) {
      continue;
    }
    std::string badge = action->GetDisplayBadgeText(tab_id), title = action->GetTitle(tab_id);
    BaaAction out = {};
    out.extension_id = ext->id().c_str();
    out.badge_text = badge.c_str();
    out.title = title.c_str();
    out.badge_rgba = baa::Rgba(action->GetBadgeBackgroundColor(tab_id));
    out.text_rgba = baa::Rgba(action->GetBadgeTextColor(tab_id));
    out.enabled = action->GetIsVisible(tab_id);
    out.has_popup = action->HasPopup(tab_id);
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
  if (!ext || baa::HiddenExtension(*ext)) {
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
        // Not ExtensionRegistrar::EnableExtension: it clears every disable reason, which would
        // approve an update's new permissions without asking.
        return baa::StartReenable(profile, chrome_profile, id);
      }
      registrar->DisableExtension(id, {extensions::disable_reason::DISABLE_USER_ACTION});
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

namespace {

// Answers request `id` (kLoadUnpacked) with the extension's id or Chrome's error.
void LoadUnpacked(Profile* profile, uint64_t id, const base::FilePath& path) {
  auto installer = extensions::UnpackedInstaller::Create(profile);
  installer->set_be_noisy_on_failure(false);
  // Posted: the installer reports some failures before Load returns.
  installer->set_completion_callback(base::BindOnce(
      [](uint64_t id, const extensions::Extension* extension, const base::FilePath&, const std::u16string& error) {
        std::string message = extension ? extension->id() : base::UTF16ToUTF8(error);
        baa::FinishRequestLater(baa::RequestKind::kLoadUnpacked, id, extension != nullptr,
                                message.empty() ? "Could not load the extension" : message);
      },
      id));
  installer->Load(path);
}

// baa_profile_copy_extension's end: frees its blank page and answers the request.
void CopyDone(content::WebContents* contents, uint64_t id, bool ok, const std::string& message) {
  base::SingleThreadTaskRunner::GetCurrentDefault()->DeleteSoon(FROM_HERE, contents);
  baa::FinishRequestLater(baa::RequestKind::kLoadUnpacked, id, ok,
                          message.empty() ? "Could not install the extension" : message);
}

}  // namespace

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
  LoadUnpacked(chrome_profile, id, base::FilePath(path));
}

void baa_profile_copy_extension(BaaProfile* to, uint64_t id, BaaProfile* from, const char* extension_id) {
  Profile* target = baa::ChromeProfileOf(to);
  Profile* source = baa::ChromeProfileOf(from);
  baa::BeginRequest(baa::RequestKind::kLoadUnpacked, id, target);
  // Not a blocklisted or policy-blocked copy: Chrome would refuse to run it there either.
  const extensions::Extension* ext =
      source && extension_id
          ? extensions::ExtensionRegistry::Get(source)->GetExtensionById(
                extension_id, extensions::ExtensionRegistry::ENABLED | extensions::ExtensionRegistry::DISABLED |
                                  extensions::ExtensionRegistry::TERMINATED)
          : nullptr;
  if (!target || !ext || extensions::Manifest::IsComponentLocation(ext->location())) {
    baa::FinishRequestLater(baa::RequestKind::kLoadUnpacked, id, false, "No such profile or extension");
    return;
  }
  if (extensions::ExtensionRegistry::Get(target)->GetInstalledExtension(ext->id())) {
    baa::FinishRequestLater(baa::RequestKind::kLoadUnpacked, id, true, ext->id());
    return;
  }
  if (extensions::Manifest::IsUnpackedLocation(ext->location())) {
    // security: Chrome loads unpacked code only in developer mode; the target Space's own
    // switch decides, as for baa_profile_load_unpacked.
    if (!target->GetPrefs()->GetBoolean(prefs::kExtensionsUIDeveloperMode)) {
      baa::FinishRequestLater(baa::RequestKind::kLoadUnpacked, id, false,
                              "Must be in developer mode to load unpacked extensions.");
      return;
    }
    LoadUnpacked(target, id, ext->path());
    return;
  }
  // The user already approved this extension in `source`: no prompt, and the store's current
  // version may differ from that copy but not ask for more (a loose manifest check).
  auto approval = extensions::InstallApproval::CreateWithNoInstallPrompt(
      target, ext->id(), ext->manifest()->value()->Clone(), /*strict_manifest_check=*/false);
  approval->skip_post_install_ui = true;
  // WebstoreInstaller downloads through a page of the target profile; a blank one serves.
  // Whichever callback runs frees it.
  content::WebContents* contents =
      content::WebContents::Create(content::WebContents::CreateParams(target)).release();
  auto installer = base::MakeRefCounted<extensions::WebstoreInstaller>(
      target,
      base::BindOnce(
          [](content::WebContents* contents, uint64_t id, const std::string& ext_id) {
            CopyDone(contents, id, true, ext_id);
          },
          contents, id),
      base::BindOnce(
          [](content::WebContents* contents, uint64_t id, const std::string&, const std::string& error,
             extensions::WebstoreInstaller::FailureReason) { CopyDone(contents, id, false, error); },
          contents, id),
      contents, ext->id(), std::move(approval), extensions::WebstoreInstaller::INSTALL_SOURCE_OTHER);
  installer->Start();
}

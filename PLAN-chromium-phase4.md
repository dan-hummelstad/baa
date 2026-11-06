# Phase 4, milestone M2: the delegate surface (parity with the CEF build)

Status: done 2026-11-06 (checked end to end on the existing UI); the CEF deletions happened in phase 5 (`PLAN-chromium-phase5.md`).

Plan for phase 4 of `PLAN-chromium.md`, written 2026-11-06 against Chromium 156 (Helium tree at
`/Users/Shared/hb/helium-macos/build/src`; paths starting `chrome/`, `content/`, `components/`,
`extensions/`, `services/` are in that tree) and M1 (`PLAN-chromium-phase3.md`, `chromium/baa/baa.h`
ABI 3, the `//baa` files `app_mac.mm`, `tab.mm`, `window.mm`, `internal.h`). M2 starts after M1 lands.
The C ABI for M2 is ABI 4 (the header diff below). It is the contract between the C++ and the Rust
package; neither edits the other's files.

**M2 is done when** every row of the phase 5 parity gate (`PLAN-chromium-phase5.md` §5.0) that
depends on the engine works on the core build with the UI code unchanged: permission prompts and
per-site settings, page dialogs including "leave site?", context menus (Chrome's items, Inspect
opens baa's docked DevTools), downloads list + cancel, DevTools docked and in a window, the
extensions page (details, enable, remove, site access) and toolbar popups that act on the real
active tab, the chrome://settings sheet with baa's skin, search suggestions (`fetch`), de-Googling,
and 1Password signing in and seeing tabs with no shim. Nothing the CEF build does is lost.

**Not in M2** (M3, "later" below): `chrome.tabGroups` ↔ folders, the command bar on
`AutocompleteController`, GPUI-drawn HTTP auth / external protocol / hung renderer / extension
install prompts (Chrome's own dialogs show in M2), page fullscreen of the window, `chrome.commands`
shortcuts, `chrome.action.openPopup`, dynamic toolbar icons and badges, find match counts. None of
them exists in the CEF build either.

## Decisions

| # | Question | Decision |
|---|---|---|
| 1 | One handle for every page | `BaaTab` now means any WebContents baa shows: a tab in a Browser (M1), an extension popup, a DevTools front-end. Rust keeps one `CorePage` type. `baa::Tab` gets a second constructor for a bare `WebContents*` (popups) and is keyed by `WebContents*` for those; tab-strip calls (`baa_tab_activate`) do nothing on them. Same lifetime rules as M1 (`tab_closed` when Chrome closes it, `baa_tab_close` silent). |
| 2 | Permissions | Per tab, after tab helpers: `PermissionRequestManager::set_view_factory_for_testing(factory)` (`components/permissions/permission_request_manager.h:264`) and `clear_permission_ui_selector_for_testing()` (`:254`, so the quiet UI is never chosen). The factory returns `baa::PermissionPrompt : permissions::PermissionPrompt` (`permission_prompt.h:173-196`) which sends `permission(id, origin, kinds)`. No patch. Chrome persists the decision (camera+mic arrive as one prompt with two requests, `permission_request_manager.cc:139-171`). |
| 3 | Site permissions | `HostContentSettingsMapFactory::GetForProfile(p)` `GetContentSetting(url, url, type)` / `SetContentSettingDefaultScope(url, url, type, setting)` (`components/content_settings/core/browser/host_content_settings_map.h:132,254`); "Ask" sets `CONTENT_SETTING_DEFAULT` (back to the default). |
| 4 | "Leave site?" | Chrome sends beforeunload to the app-modal manager (`components/javascript_dialogs/tab_modal_dialog_manager.cc:221-229`). At start-up, after Chrome installed its Cocoa factory (`chrome/browser/chrome_browser_main.cc:1567`), //baa saves `*AppModalDialogManager::GetInstance()->view_factory()` and sets its own (`SetNativeDialogFactory`, `app_modal_dialog_manager.h:43`). For a controller whose `web_contents()` is a `baa::Tab` it returns a `baa::AppModalDialog : AppModalDialogView` that sends `dialog` with kind `BAA_DIALOG_BEFOREUNLOAD[_RELOAD]` (ids shared with M1's dialogs; `baa_dialog_answer` → `controller->OnAccept(text, false)` / `OnCancel(false)`); anything else goes to Chrome's saved factory. No patch. Same for alerts that come app-modal (extension popups). |
| 5 | Context menu | One patch (`context-menu-hook.patch`): at the top of `ChromeWebContentsViewDelegateViewsMac::ShowMenu` (`chrome/browser/ui/views/tab_contents/chrome_web_contents_view_delegate_views_mac.mm:195`), a global hook may take the fully `Init()`ed `RenderViewContextMenuBase` (clipboard state, extension items, page's custom items). There is no non-patch interception (`HandleContextMenu` is not overridden by `BrowserWebContentsDelegate`; the view delegate has no setter, `chrome_content_browser_client.cc:1856`). Without it, M1's menu silently shows nothing (`render_view_context_menu_mac_cocoa.mm:77-83` returns when there is no views Widget). //baa flattens `menu_model()` into `BaaMenuItem`s; Rust shows them as a native NSMenu (M1's `menu.rs`), then `baa_context_menu_done(id, command)` → `ExecuteCommand` + `MenuClosed`. Inspect is marked (`IDC_CONTENT_CONTEXT_INSPECTELEMENT`, `chrome_command_ids.h:378`) so Rust opens baa's docked DevTools, as CEF's `inspect_id` did, without matching the English label. |
| 6 | Downloads | Per profile `content::DownloadManager::Observer` + per item `download::DownloadItem::Observer` (`components/download/public/common/download_item.h:135-142`). Report only items that are `IN_PROGRESS` when created (history loads arrive as complete/cancelled items and are skipped; baa-model keeps its own list). Source page: `DownloadItemUtils::GetWebContents`. Chrome shows no UI and does not crash (`browser_window_features.cc:826-833`, bubble lookups null-safe). Dangerous / insecure files: //baa calls `ValidateDangerousDownload()` / `ValidateInsecureDownload()` at once, as CEF kept every download (ponytail: ceiling is a keep/discard row in the tray). Save As: Chrome's picker works on our window (`ui/shell_dialogs/select_file_dialog_mac.mm:101-110`). |
| 7 | DevTools docked | Chrome never docks into a non-views window: `can_dock` is false without a `DevtoolsUIController` (`chrome/browser/devtools/devtools_window.cc:1395-1407`), so Chrome makes a TYPE_DEVTOOLS Browser synchronously inside `OpenDevToolsWindow` (`ScheduleShow` → `Show` → `CreateDevToolsBrowser`, `:1018-1031,1100-1124,2054-2097`). `baa_tab_open_devtools` sets a thread-local "docking" flag around that call; the window hook (M1) then returns a **hidden** `baa::ui::Window` for that TYPE_DEVTOOLS Browser (never activated, never "last active"), and the front-end WebContents is returned as a `BaaTab`. Rust puts its NSView next to the page, as CEF's `new_dev_tools` did. Without the flag the hook returns null as in M1: Chrome's views DevTools window (the "undocked" setting, Chrome-opened DevTools such as a popup's Inspect). No patch. |
| 8 | DevTools dock picker | As in CEF: Rust evals the dock script (CEF's `dock_script`, `vendor/gpui-cef/.../handlers.rs:454-484`) in the front-end once it has loaded (eval is allowed on `devtools://`, `render_frame_host_impl.cc:14289-14300`). One change: the wrapped `setIsDocked` always passes `false` to Chrome (`SetIsDocked(true)` hits `DCHECK(can_dock_ \|\| !dock_requested)`, `devtools_window.cc:1782`). The script reports the side with `console.debug("baa:devtools-dock:<side>")`; //baa forwards console messages that start with `baa:` from any baa page as `page_message`. |
| 9 | Extension list and changes | Direct C++ calls, no hidden chrome://extensions page: `ExtensionRegistry` (enabled + disabled + terminated, no component extensions, no themes), permission messages and host access computed exactly as `developerPrivate` does (`chrome/browser/extensions/api/developer_private/extension_info_generator.cc:300-339,375-400`), enable/disable/uninstall through `ExtensionRegistrar` (`extensions/browser/extension_registrar.h:224,228,290`), site access through `ScriptingPermissionsModifier` as `developer_private_functions.cc:711-735`. One visitor call serves both `extensions()` (toolbar) and `extension_details()` (page), replacing the disk reader `installed_extensions`. |
| 10 | Toolbar popups | `baa_extension_popup(profile, id)` runs the action on the profile's active tab like a click: `ExtensionActionRunner::GetForWebContents(wc)->RunAction(ext, /*grant_tab_permissions=*/true)` (`extension_action_runner.h:63,75`: grants activeTab, fires `action.onClicked`). On `kShowPopup`, `ExtensionViewHostFactory::CreatePopupHost(ext, action->GetPopupUrl(tab_id), browser)` (`extension_view_host_factory.h:37-40`), hosted by `baa::Popup : extensions::ExtensionView` (Android's `extension_action_popup_contents.cc` is the non-views model): `set_view(this)` **before** `CreateRendererSoon()` (`ExtensionViewHost` derefs `view_` unchecked), `SetCloseHandler` → `tab_closed`, auto-resize 25×25…800×600 (`extension_popup_types.h:14-15`) → `resized`. The tabs shim goes: `chrome.tabs.query({active})` is the real tab. |
| 11 | ExtensionsContainer | `baa::ui::Window` registers a `baa::ExtensionsContainer` (`chrome/browser/ui/extensions/extensions_container.h:25-61`) with `ui::ScopedUnownedUserData` like `webui_browser_window.cc:190-195`. Unregistered, `extension_context_menu_model.cc:706-708` and the CDP `Extensions.triggerAction` handler deref null. M2 bodies: `GetActionForId` null, `ShowToolbarActionPopupForAPICall` false, `HasAnyExtensions` from the registry, the rest no-ops. `IsToolbarVisible()` stays false, so `chrome.action.openPopup` fails cleanly ("no toolbar"). |
| 12 | Extension install UI | Web Store installs and `chrome.permissions.request` show Chrome's views install dialog as a browser-modal sheet on the gpui window (`extension_install_dialog_view.cc:85-110`, `constrained_window_views.cc:348-352`). The post-install dialog derefs a null BrowserView when it closes (`chrome/browser/ui/extensions/extension_install_ui_desktop.cc:161-164`): //baa keeps `ExtensionInstallUI::disable_ui_for_tests(true)` (an `AutoReset`, `extension_install_ui.h:59`) alive for the process. GPUI prompts: M3. |
| 13 | chrome://settings sheet | A real tab in the profile's Browser (`baa_tab_create(profile, "chrome://settings/…")`), shown by Rust as a floating page over the sheet bounds. Not a bare `WebContents::Create`: `SettingsUI` derefs the autofill client (`settings_ui.cc:431,599`) and the passwords/import handlers CHECK or deref `FindBrowserWithTab` (`password_manager_handler.cc:41-44`, `import_data_handler.cc:223-230`). Skin: `baa_tab_set_init_script(tab, script)` runs the CEF `skin_loader` script (unchanged) in every new primary main document (`PrimaryMainDocumentElementAvailable`); `ExecuteJavaScript` is allowed on WebUI. Dark: `baa_tab_set_color_scheme` sets `WebPreferences::preferred_color_scheme` (`GetOrCreateWebPreferences` → `SetWebPreferences`). Rust maps `baa://` to `chrome://` (a raw `LoadURL` does not rewrite Helium's scheme). Extensions see the settings tab in `chrome.tabs`; accepted. |
| 14 | fetch | `baa_profile_fetch(profile, url, id)`: `network::SimpleURLLoader` (`services/network/public/cpp/simple_url_loader.h:188,235`) on `profile->GetDefaultStoragePartition()->GetURLLoaderFactoryForBrowserProcess()`, `credentials_mode = kOmit`, 1 MiB cap, 10 s timeout → `fetch_done(id, ok, body)`. |
| 15 | De-Googling | Helium already covers most switches and prefs (table below). No ContentBrowserClient hook sees browser-process requests (`content/browser/storage_partition_impl.cc:3618-3631`); ungoogled's domain substitution + `trk:` rewrite in `net/url_request/url_request.cc` blocks hard-coded Google hosts in every process. M2 drops `block_browser_request` on the core build and keeps a short switch/feature list; the CI proxy test decides. Fallback only if it fails: a patch in `services/network/url_loader_factory.cc:131` for `process_id == kBrowserProcessId` with the host list compiled in (the network service is a separate process; a //baa function pointer does not reach it). |
| 16 | Native messaging (1Password) | Nothing new. With `--user-data-dir`, `DIR_USER_NATIVE_MESSAGING` = `<data>/chromium/NativeMessagingHosts` (`chrome/common/chrome_paths.cc:458-505`, `chrome_main_delegate.cc:661,690`), which M1's `share_native_messaging_hosts` fills; Helium also searches `~/Library/Application Support/Google/Chrome/NativeMessagingHosts` and `/Library/Google/Chrome/NativeMessagingHosts` (`launch_context_posix.cc:39-67`). Chromium checks only `allowed_origins` (`launch_context.cc:169`); 1Password checks the browser's signature in its own host. Real tabs (M1) are what removes the shim. |
| 17 | Popups in Peek / lil-baa | Unchanged UI rules: Peek and lil-baa deny permissions and cancel dialogs (they already do). |
| 18 | UI contract | **No change to `WebHost`/`PageHandle`/`PageEvent` in M2.** On the core build `PageEvent::ContextMenu` and `ActiveTabLoad` are never sent, and `create_popup`'s `tab_url`/`tab_title` are unused; phase 5 removes them with CEF. M3 proposes two additions (autocomplete, tab groups), listed there. |

### Who draws what in M2

| Feature | Drawn by | Note |
|---|---|---|
| Permission prompt, site panel | GPUI (`prompts.rs`) | as CEF |
| alert/confirm/prompt, leave site? | GPUI (`prompts.rs`) | as CEF; M1 had Chrome's sheet for leave site |
| Context menu | native NSMenu with Chrome's items (`menu.rs`) | as CEF's Chrome tabs |
| Downloads tray, Library | GPUI | as CEF |
| DevTools | front-end WebContents in a GPUI dock, or Chrome's views window | as CEF |
| Extensions page | GPUI (`extensions.rs`) | data now from C++, not a hidden page |
| Toolbar popup | `ExtensionViewHost` WebContents in the Peek card | real activeTab |
| chrome://settings sheet | Chrome's WebUI tab in a GPUI sheet | as CEF |
| HTTP auth, external protocol, hung renderer, extension install, print preview (off) | Chrome views dialogs, parentless windows placed by M1's modal host | M3 moves them to GPUI |
| Video PiP | Chrome's views overlay window (`video_overlay_window_views.cc:322-421`, needs no BrowserView) | as CEF |

## ABI 4 (proposed diff to `chromium/baa/baa.h`)

The manager applies this after M1 lands; both packages build against it. Every callback is
appended after `tab_closed`, so the struct keeps M1's prefix.

```diff
@@
-#define BAA_ABI_VERSION 3
+#define BAA_ABI_VERSION 4
 BAA_EXPORT int baa_abi_version(void);
 
 typedef struct BaaProfile BaaProfile;
+// Any page baa shows: a tab in a Browser, an extension popup, or a DevTools
+// front-end. Calls that only make sense for a tab (baa_tab_activate) do
+// nothing on the others.
 typedef struct BaaTab BaaTab;
@@
+// One row of a page's context menu, flattened depth-first. A SUBMENU row is
+// followed by its items at depth + 1.
+typedef struct {
+  const char* label;  // "" for a separator; no '&' mnemonics
+  int command;        // for baa_context_menu_done; -1 on separators/submenus
+  uint8_t depth;
+  uint8_t type;       // BAA_MENU_*
+  bool enabled;
+  bool checked;
+} BaaMenuItem;
+
+enum {
+  BAA_MENU_COMMAND = 0,
+  BAA_MENU_SEPARATOR = 1,
+  BAA_MENU_SUBMENU = 2,
+  BAA_MENU_INSPECT = 3,  // a COMMAND that is "Inspect" (Chrome would open DevTools)
+};
+
+// Permission kinds, as bits.
+enum {
+  BAA_PERMISSION_CAMERA = 1 << 0,
+  BAA_PERMISSION_MICROPHONE = 1 << 1,
+  BAA_PERMISSION_LOCATION = 1 << 2,
+  BAA_PERMISSION_NOTIFICATIONS = 1 << 3,
+  BAA_PERMISSION_CLIPBOARD = 1 << 4,
+  BAA_PERMISSION_MIDI = 1 << 5,
+  BAA_PERMISSION_OTHER = 1 << 6,  // any request type not above
+};
+
+enum { BAA_SITE_ASK = 0, BAA_SITE_ALLOW = 1, BAA_SITE_BLOCK = 2 };
+
+typedef struct {
+  uint32_t id;           // unique per run; baa_download_cancel takes it
+  const char* url;
+  const char* path;      // target file; "" until chosen
+  int64_t received_bytes;
+  int64_t total_bytes;   // -1 or 0 when unknown
+  int state;             // BAA_DOWNLOAD_*
+} BaaDownload;
+
+enum {
+  BAA_DOWNLOAD_IN_PROGRESS = 0,
+  BAA_DOWNLOAD_COMPLETE = 1,
+  BAA_DOWNLOAD_CANCELED = 2,  // also interrupted
+};
+
 typedef struct {
@@  (after tab_closed)
   void (*tab_closed)(void* ctx, BaaTab* tab);
+
+  // A site asks for `kinds` (BAA_PERMISSION_* bits; camera + microphone come
+  // as one request). It waits until baa_permission_answer(id, ...);
+  // `permission_closed` with the same id if Chrome dropped the request first
+  // (navigation, tab closed). Only sent while the tab is its window's active
+  // tab: Chrome holds requests from background tabs until activation.
+  void (*permission)(void* ctx, BaaTab* tab, uint64_t id, const char* origin,
+                     uint32_t kinds);
+  void (*permission_closed)(void* ctx, BaaTab* tab, uint64_t id);
+
+  // Right-click in a page. (x, y) in the tab's view, points, top-left origin.
+  // Answer with baa_context_menu_done(id, command or -1) exactly once; until
+  // then the page counts the menu as open.
+  void (*context_menu)(void* ctx, BaaTab* tab, uint64_t id, float x, float y,
+                       const BaaMenuItem* items, size_t count);
+
+  // A download started or changed. `tab` is the page it came from, NULL if
+  // none (a service worker, a closed tab).
+  void (*download)(void* ctx, BaaTab* tab, const BaaDownload* download);
+
+  // An extension popup's content wants this size (points).
+  void (*resized)(void* ctx, BaaTab* tab, int width, int height);
+
+  // A console message from the page that starts with "baa:", without the
+  // prefix (DevTools' dock picker sends "devtools-dock:<side>").
+  void (*page_message)(void* ctx, BaaTab* tab, const char* message);
+
+  // baa_profile_fetch finished. `body` is NULL when !ok.
+  void (*fetch_done)(void* ctx, uint64_t id, bool ok, const char* body,
+                     size_t len);
 } BaaCallbacks;
@@  (Profiles)
+// Sets a boolean pref if the profile registers it (unknown names are ignored).
+BAA_EXPORT void baa_profile_set_pref_bool(BaaProfile* profile,
+                                          const char* name, bool value);
+
+// What the profile remembers for `kind` (one BAA_PERMISSION_* bit, not OTHER)
+// on `url`'s site: BAA_SITE_*.
+BAA_EXPORT int baa_profile_site_permission(BaaProfile* profile,
+                                           const char* url, uint32_t kind);
+BAA_EXPORT void baa_profile_set_site_permission(BaaProfile* profile,
+                                                const char* url,
+                                                uint32_t kind, int state);
+
+// GET `url` through the profile's network stack without cookies;
+// fetch_done(id, ...) later, never inside this call.
+BAA_EXPORT void baa_profile_fetch(BaaProfile* profile, const char* url,
+                                  uint64_t id);
+
+// ---- Extensions ------------------------------------------------------------
+
+enum {
+  BAA_SITE_ACCESS_NONE = 0,  // not the user's choice (no host permissions)
+  BAA_SITE_ACCESS_ON_CLICK = 1,
+  BAA_SITE_ACCESS_ON_SPECIFIC_SITES = 2,
+  BAA_SITE_ACCESS_ON_ALL_SITES = 3,
+};
+
+typedef struct {
+  const char* id;
+  const char* name;
+  const char* description;
+  const char* version;
+  const char* icon_path;    // absolute; action's default icon, else manifest
+                            // icon, nearest 32 px; "" if none
+  const char* popup_url;    // the action's default popup; "" if none
+  const char* options_url;  // "" if none
+  const char* permissions;  // Chrome's permission messages, '\n'-separated
+  int site_access;          // BAA_SITE_ACCESS_*
+  bool enabled;
+} BaaExtension;
+
+typedef void (*BaaExtensionVisitor)(void* ctx, const BaaExtension* extension);
+
+// Every user-visible extension of the profile (no component extensions, no
+// themes), synchronously. Returns the count, -1 for a gone profile.
+BAA_EXPORT int baa_profile_extensions(BaaProfile* profile,
+                                      BaaExtensionVisitor visit, void* ctx);
+
+enum {
+  BAA_EXTENSION_ENABLE = 0,       // value: 1 enable, 0 disable
+  BAA_EXTENSION_REMOVE = 1,       // value ignored
+  BAA_EXTENSION_SITE_ACCESS = 2,  // value: BAA_SITE_ACCESS_ON_*
+};
+
+// Carries out the change at once; false if it was refused or the id is
+// unknown.
+BAA_EXPORT bool baa_profile_extension_change(BaaProfile* profile,
+                                             const char* extension_id,
+                                             int change, int value);
+
+// Clicks the extension's toolbar action on the profile's active tab (grants
+// activeTab, fires action.onClicked). Returns the popup page when the action
+// has one (it reports tab_state/resized and tab_closed when it closes
+// itself), else NULL.
+BAA_EXPORT BaaTab* baa_extension_popup(BaaProfile* profile,
+                                       const char* extension_id);
@@  (Tabs)
+// Runs `script` in the main world of every new main-frame document of the
+// tab, as soon as its root element exists. NULL or "" stops it.
+BAA_EXPORT void baa_tab_set_init_script(BaaTab* tab, const char* script);
+
+// prefers-color-scheme for the page: 0 follows the system, 1 light, 2 dark.
+BAA_EXPORT void baa_tab_set_color_scheme(BaaTab* tab, int scheme);
+
+// DevTools for `tab`, to be docked by the caller: returns the front-end page
+// (place its state.view; baa_tab_close closes DevTools). With `inspect`,
+// selects the element at (x, y) in the page's view. Closes DevTools already
+// open for the tab first. NULL if Chrome refused.
+BAA_EXPORT BaaTab* baa_tab_open_devtools(BaaTab* tab, bool inspect, int x,
+                                         int y);
+
+// DevTools for `tab` in Chrome's own window.
+BAA_EXPORT void baa_tab_show_devtools_window(BaaTab* tab, bool inspect, int x,
+                                             int y);
@@  (Dialogs)
 enum {
   BAA_DIALOG_ALERT = 0,
   BAA_DIALOG_CONFIRM = 1,
   BAA_DIALOG_PROMPT = 2,
+  BAA_DIALOG_BEFOREUNLOAD = 3,         // OK leaves the page
+  BAA_DIALOG_BEFOREUNLOAD_RELOAD = 4,  // OK reloads it
 };
@@
+// ---- Permissions, menus, downloads -----------------------------------------
+
+// Answers `permission`. Unknown or answered ids do nothing.
+BAA_EXPORT void baa_permission_answer(uint64_t id, bool allow);
+
+// Closes the menu of a `context_menu` callback; `command` >= 0 runs that
+// item first. Unknown ids do nothing.
+BAA_EXPORT void baa_context_menu_done(uint64_t id, int command);
+
+// Stops a running download; finished ones are left alone.
+BAA_EXPORT void baa_download_cancel(uint32_t id);
-
-// ---- M2 --------------------------------------------------------------------
-// Not here on purpose: ...
```

Also update the `beforeunload` sentence of the `dialog` comment: beforeunload is now sent with kinds 3/4.

## Features: hook, //baa, ABI, Rust, risks, test

The C++ package owns `chromium/baa/*`, `chromium/patches/baa/*`, `crates/baa-chromium/*` (the
harness). The Rust package owns `crates/baa-web/*` (`core_host*`), `crates/baa/*`, and the
two-line `baa-privacy` change in step 7. New //baa files: `permissions.mm` (prompt, site
settings), `menu.mm` (context menu hook, app-modal dialogs), `downloads.mm`, `extensions.mm`
(list, change, popup, container), `devtools.mm`, `net.mm` (fetch). Keep them small; move code
between files only when one passes ~600 lines.

Harness (`crates/baa-chromium`) grows the same way in each step: it prints each new callback as one
line (`baa: permission <tab> id=3 origin=https://… kinds=0x3`), auto-answers by env
(`BAA_HARNESS_PERMISSION=allow|deny`, `BAA_HARNESS_DIALOG=ok|cancel`, `BAA_HARNESS_MENU=<label>|none`),
and `BAA_SPIKE_SCRIPT` gains steps `menu <x> <y>` (synthesised right-click), `devtools`,
`devtools-window`, `popup <ext-id>`, `extensions`, `ext <id> enable|disable|remove|access <n>`,
`site <kind> <url> [ask|allow|block]`, `fetch <url>`, `pref <name> <bool>`. Page-side checks use
`cdp.mjs` as in M1.

### 1. Permissions and site permissions

- **Hook:** factory type `base::RepeatingCallback<std::unique_ptr<PermissionPrompt>(WebContents*, PermissionPrompt::Delegate*)>`
  (`permission_prompt.h:159`), called at `permission_request_manager.cc:933`. Set when `baa::Tab`
  attaches (M1 sets a null-returning factory; M2 returns the prompt), and again after `kReplaced`.
- **//baa:** `baa::PermissionPrompt` ctor sends `permission(tab, id, origin, kinds)` with
  `origin = delegate->GetRequestingOrigin().spec()` and kinds from each `Requests()[i]->request_type()`:
  kCameraStream→CAMERA, kMicStream→MICROPHONE, kGeolocation→LOCATION, kNotifications→NOTIFICATIONS,
  kClipboard→CLIPBOARD, kMidiSysex→MIDI, anything else→OTHER (`components/permissions/request_type.cc:392-462`).
  Overrides: `UpdateAnchor` true, `GetTabSwitchingBehavior` `kKeepPromptAlive` (the UI keeps asks per
  tab), `GetPromptDisposition` `ANCHORED_BUBBLE`, `IsAskPrompt` true, bounds/variants/position empty.
  `baa_permission_answer(id, allow)` → `delegate->Accept(std::monostate())` or `Deny(std::monostate())`.
  Accept deletes the prompt synchronously (`FinalizeCurrentRequests`, `.cc:1504`): look the prompt up
  by id in a map, erase it **before** calling Accept/Deny, never touch it after. Destructor: if still
  in the map → `permission_closed`. Site settings: kind → `MEDIASTREAM_CAMERA`, `MEDIASTREAM_MIC`,
  `GEOLOCATION`, `NOTIFICATIONS`, `CLIPBOARD_READ_WRITE`, `MIDI_SYSEX`; ASK/ALLOW/BLOCK ↔
  `CONTENT_SETTING_ASK/ALLOW/BLOCK` (DEFAULT reads as the resolved value; writing Ask writes `CONTENT_SETTING_DEFAULT`).
- **Rust:** `permission` → `PageEvent::PermissionRequested { id, origin, kinds }` (bits → `Permission`
  in bit order); `permission_closed` → `PermissionDismissed(id)`; `answer_permission` →
  `baa_permission_answer`; `site_permission`/`set_site_permission` → the two calls (ephemeral
  profiles work the same; `Permission::ScreenShare`/`Other` → `Ask`, set is a no-op).
- **Risks:** prompts for a tab in a TYPE_POPUP Browser (`window.open(…,'popup')` adopted into Peek)
  wait until that tab is active in its Browser: `baa_tab_activate` on show covers it.
  `getDisplayMedia` does not use the prompt: Chrome's views `DesktopMediaPicker` shows as a
  parentless window (M1 modal host); test it doesn't crash.
- **Test (harness):** `BAA_HARNESS_PERMISSION=allow`, page calls `navigator.geolocation.getCurrentPosition`
  → line `permission … kinds=0x4`; second call → no line (remembered); `site location <url>` prints
  `allow`; `site location <url> block` then a call → page error callback, no line.
  `getUserMedia({audio:true,video:true})` → one line `kinds=0x3`. Navigate away while pending →
  `permission_closed`. `Notification.requestPermission()` deny → page sees `"denied"` (cdp).
  App: the parity rows "permission prompts and per-site settings".

### 2. Page dialogs: "leave site?"

- **Hook:** `AppModalDialogManager::SetNativeDialogFactory`; install from //baa's first
  `baa_profile_open` (after `PreMainMessageLoopRun`, so Chrome's factory is already in).
- **//baa:** `baa::AppModalDialog : javascript_dialogs::AppModalDialogView` (`app_modal_dialog_view.h:12-28`)
  holding the `AppModalDialogController*`. `ShowAppModalDialog` → `dialog(tab, id, kind, message, default)`
  (kind from `is_before_unload_dialog()`/`is_reload()`/`javascript_dialog_type()`), `CloseAppModalDialog`
  → `dialog_closed`, `AcceptAppModalDialog`/`CancelAppModalDialog` (CDP `Page.handleJavaScriptDialog`)
  → `OnAccept`/`OnCancel` + `dialog_closed`. `baa_dialog_answer` dispatches by id to M1's tab-modal map
  or this one.
- **Rust:** kinds 3/4 → `DialogKind::BeforeUnload { is_reload }`; answers as M1.
- **Risks:** ⌘Q with a beforeunload page in a background tab waits for an answer the UI shows only
  when that tab is active. Fallback: Rust answers OK (leave) for BeforeUnload dialogs on pages that
  are not visible while the app is quitting (one flag set from gpui's quit path).
- **Test:** harness page with `onbeforeunload = e => e.preventDefault()`, a click, then `load <url>` →
  `dialog … kind=3`; `BAA_HARNESS_DIALOG=cancel` → url unchanged; ok → navigates. `reload` → kind 4.
  `alert()` inside an extension popup → `dialog` on the popup's tab handle.

### 3. Context menus

- **Patch `context-menu-hook.patch`:** new header `chrome/browser/ui/views/tab_contents/baa_context_menu_hook.h`:
  `using BaaShowContextMenuFn = bool (*)(std::unique_ptr<RenderViewContextMenuBase>& menu); extern BaaShowContextMenuFn g_baa_show_context_menu;`
  Defined (null) in `chrome_web_contents_view_delegate_views_mac.mm`; first lines of `ShowMenu`:
  `if (g_baa_show_context_menu && g_baa_show_context_menu(menu)) return;` (the hook moves `menu` out
  when it returns true). Header added to that file's GN target's `sources`.
- **//baa (`menu.mm`):** the hook returns false unless `menu->GetWebContents()` (or `source_web_contents_`)
  is a `baa::Tab`. Else: store the menu under a new id, `OnMenuWillShow(&model)`, flatten
  `menu_model()` (visible items only; `GetTypeAt`, `GetLabelAt` without `&` mnemonics, `GetCommandIdAt`,
  `IsEnabledAt`, `IsItemCheckedAt`, recurse into `GetSubmenuModelAt` with depth+1; mark
  `IDC_CONTENT_CONTEXT_INSPECTELEMENT` as `BAA_MENU_INSPECT`), call `context_menu(tab, id, params.x, params.y, …)`.
  `baa_context_menu_done(id, cmd)`: if `cmd >= 0` → `menu->ExecuteCommand(cmd, 0)` (before close:
  custom page items need the menu client, `render_view_context_menu_base.cc:435-443`), then
  `menu->MenuClosed(const_cast<ui::SimpleMenuModel*>(&menu->menu_model()))` (sends
  `NotifyContextMenuClosed`), delete. A `baa::Tab` going away (`WebContentsDestroyed`, `kReplaced`)
  deletes its pending menus without running them.
- **Rust:** `context_menu` → map rows to `MenuItem` (tree from depth) and run `menu.rs`'s popup at
  container origin + (x, y) via `popup_soon` (never inside a callback). Picked INSPECT on a tab page →
  `baa_context_menu_done(id, -1)` + `PageEvent::DevToolsRequested { x, y }`; on a popup/DevTools page →
  `baa_context_menu_done(id, command)` (Chrome's DevTools window). Anything else → `done(id, command)`;
  closed with nothing → `done(id, -1)`.
- **Risks:** Emoji (`ShowEmojiPanel` on our window, a no-op in M1) and Compose items
  (`render_view_context_menu.cc:4950`, `BrowserUserEducationInterface::From`) — hide by not offering
  them: //baa skips `IDC_CONTENT_CONTEXT_EMOJI` and `IDC_CONTENT_CONTEXT_COMPOSE` rows if present.
  "Open link in new tab/window" create tabs through Chrome → `tab_created` (M1) → background tab /
  ChromeTabs: correct. "Print…" → `chrome::Print` → basic print with preview off.
- **Test (harness):** `BAA_HARNESS_MENU=Copy Link Address`, `menu <x> <y>` over a link → line
  `context_menu … rows=N inspect=1`, then clipboard holds the URL (`pbpaste`). A page with
  `oncontextmenu` that calls `preventDefault` → no line. Selecting text + `menu` → row "Copy" enabled.
  App: right-click in a page shows Chrome's items; Inspect docks DevTools on the element; an
  extension's `chrome.contextMenus` item appears and runs.

### 4. Downloads

- **Hook:** `profile->GetDownloadManager()->AddObserver` in `baa::Profile::Open` (OTR too); remove in
  `ManagerGoingDown` and profile close.
- **//baa (`downloads.mm`):** `OnDownloadCreated`: skip unless `GetState() == IN_PROGRESS`; observe
  the item; if `IsDangerous()` → `ValidateDangerousDownload()`, if `IsInsecure()` →
  `ValidateInsecureDownload()`; send. `OnDownloadUpdated` → send; `OnDownloadDestroyed` → forget.
  `tab` = the `baa::Tab` for `DownloadItemUtils::GetWebContents(item)` or NULL. `baa_download_cancel(id)`
  → find in a `flat_map<uint32_t, DownloadItem*>`, `Cancel(true)` if in progress.
- **Rust:** `download` → `Downloads::upsert(Download { … })` on the host's entity; first sight of an
  id with a known tab → `PageEvent::DownloadStarted` on it. `cancel_download` → `baa_download_cancel`.
  (Chrome also closes a new tab that only served a download, `download_ui_controller.cc:263-279`:
  `tab_closed` → `Closed`; the UI's own blank-tab close then finds nothing. Fine.)
- **Risks:** "Ask where to save" pref on → Chrome's NSSavePanel sheet on the gpui window (works);
  ephemeral profiles' downloads go to ~/Downloads too (as CEF).
- **Test (harness):** load a URL with `Content-Disposition: attachment` (a local `python3 -m http.server`
  file with a `.zip` name works via `<a download>` + click) → `download … state=0` lines then
  `state=1`, file in `~/Downloads`; a large/slow file + `cancel` step → `state=2`, partial file gone.
  App: downloads button lists it, Library keeps it after relaunch, cancel works.

### 5. DevTools

- **Hook:** M1's window hook: `TYPE_DEVTOOLS && baa::devtools::Docking()` → `new Window(b, /*hidden=*/true)`.
  A hidden window: `Show`/`Activate`/`ShowInactive` do nothing, `IsActive` false, it never calls
  `BrowserActiveStateManager::DidBecomeActive` (extensions' "current window" must stay the page's),
  `GetNativeWindow` = its front-end view's window.
- **//baa (`devtools.mm`):** `baa_tab_open_devtools(tab, inspect, x, y)`: if
  `DevToolsWindow::GetInstanceForInspectedWebContents(wc)` exists, close it (its front-end's strip
  `CloseWebContentsAt`, or for a views window `chrome::CloseWindow`-equivalent through its Browser);
  set `Docking()`; `inspect ? DevToolsWindow::InspectElement(wc->GetPrimaryMainFrame(), x, y)
  : DevToolsWindow::OpenDevToolsWindow(wc, DevToolsToggleAction::Show(), DevToolsOpenedByAction::kUnknown)`
  (`devtools_window.h:145,202`); clear; front = `DevToolsWindow::GetDevtoolsWebContentsForInspectedWebContents(wc)`;
  make it a `baa::Tab` before the window's `kInserted` sees it (or let `kInserted` in a hidden window
  create it without `tab_created`); return it. `baa_tab_close` on a front-end: `DetachAndDeleteWebContentsAt`
  on its Browser's strip (the strip empties, the hidden Browser closes as M1). Inspected tab closing:
  Chrome closes DevTools → `tab_closed` for the front-end. `baa_tab_show_devtools_window`: same calls
  without the flag.
  Console messages: `baa::Tab` overrides `OnDidAddMessageToConsole`; text starting `baa:` →
  `page_message(tab, rest)`.
- **Rust:** `create_devtools(of, inspect_at, side)` → `CorePage` around `baa_tab_open_devtools`
  (a tab-less `CorePage` that emits `Closed` if NULL). On its first `tab_state` with `loading=false`
  and a `devtools://` URL, eval the dock script (CEF's, with the `setIsDocked(false, done)` change and
  the `baa:` prefix); later loads (the script's own `location.replace`) eval the second half.
  `page_message("devtools-dock:<side>")` → `PageEvent::DevToolsDock(side)`. `open_devtools` →
  `baa_tab_show_devtools_window`. Square corners etc. unchanged (Container).
- **Risks:** (a) `InspectElement` may not be synchronous for the window creation (it opens with an
  Inspect action); check `front != NULL` right after; fallback: open first, then
  `InspectElement` on the existing window. (b) Keys in the front-end: undocked DevTools forwards
  unhandled keys to its own Browser's window (our hidden one, which runs M1's key filter → `key`);
  test ⌘W/⌘L/⌥⌘I with DevTools focused. (c) ⌘F/⌘P inside DevTools are its own. (d) Closing and
  reopening fast (Inspect while open) — the close must be synchronous before reopening; test.
- **Test (harness):** `devtools` step → `tab_created` absent, a line `devtools <inspected> -> <front>`,
  the front-end's `tab_state` reaches `devtools://devtools/…` loading=false; `close` of the front →
  `cdp.mjs` `Target.getTargets` no longer lists a DevTools page; `devtools-window` → a views window
  appears (screenshot), no `Check failed`. App: ⌥⌘I docks right/bottom/left per settings; the picker
  in DevTools moves it; Inspect from the context menu selects the element; "undocked" setting opens
  Chrome's window.

### 6. Extensions: list, change, popups

- **//baa (`extensions.mm`):**
  - `baa_profile_extensions`: `ExtensionRegistry::Get(p)->GenerateInstalledExtensionsSet()` minus
    `Manifest::IsComponentLocation(ext->location())` and `ext->is_theme()`. Per extension: `name()`,
    `description()`, `VersionString()`, enabled = `enabled_extensions().Contains(id)`; icon:
    `ActionInfo::GetExtensionActionInfo(ext)->default_icon.Get(32, ExtensionIconSet::Match::kBigger)`
    else `IconsInfo::GetIcons(ext).Get(32, kBigger)`, made absolute with `ext->path()`; popup =
    `ActionInfo` default popup URL; options = `OptionsPageInfo::GetOptionsPage(ext)`
    (`options_page_info.h:39`); permissions and site access: copy the two blocks of
    `extension_info_generator.cc:300-339` and `:375-400` (same messages and `hostAccess` the CEF build
    read from `developerPrivate`).
  - `baa_profile_extension_change`: ENABLE → `ExtensionRegistrar::Get(p)->EnableExtension(id)` /
    `DisableExtension(id, {disable_reason::DISABLE_USER_ACTION})`; REMOVE → `UninstallExtension(id,
    UNINSTALL_REASON_USER_INITIATED, &error)` (no Chrome dialog); SITE_ACCESS → refuse unless
    `PermissionsManager::Get(p)->CanAffectExtension(*ext)`, then the three branches of
    `developer_private_functions.cc:711-735` on `ScriptingPermissionsModifier(p, ext)`.
  - `baa_extension_popup(profile, id)`: active tab = the profile Browser's active `WebContents`
    (`baa_tab_activate` keeps it the shown tab); `RunAction(ext, true)`; on `kShowPopup`
    `CreatePopupHost(*ext, action->GetPopupUrl(SessionTabHelper::IdForTab(wc).id()), browser)`;
    `baa::Popup` (`ExtensionView`): `GetNativeView` = host contents' view, `ResizeDueToAutoResize`
    → `resized`, `RenderFrameCreated(rfh)` → `rfh->GetView()->EnableAutoResize(min, max)`,
    `HandleKeyboardEvent` → M1's `OfferKey(event, true)`, `OnLoaded` no-op. `set_view` →
    `SetCloseHandler` (→ `tab_closed`, delete) → `CreateRendererSoon()`. Close on unload of that
    extension (`OnExtensionUnloaded`). `baa_tab_close` on a popup deletes the host.
  - `baa::ExtensionsContainer` (#11) and the `disable_ui_for_tests` AutoReset (#12) in `app_mac.mm`
    / `window.mm`.
- **Rust:** one helper `read_extensions(profile) -> Vec<(Extension, ExtensionInfo)>` from the
  visitor. `extensions()` → enabled ones' `Extension { id, name, icon, popup, options }`
  (`installed_extensions` disk reader no longer used on core); `extension_details` →
  `done(Some(infos))` at once (sorted by lowercased name, as `parse_extension_details`);
  `change_extension` → `done(baa_profile_extension_change(..))`. Ephemeral profiles: empty / false,
  as CEF. `create_popup(profile, url, ..)`: id = host of `url`; `baa_extension_popup` → `CorePage`
  (corner radius 8, `popup` flag so its context menu Inspect goes to Chrome's window), `resized` →
  `PageEvent::PreferredSize`; NULL → a tab-less `CorePage` that emits `Closed` on the next turn (the
  UI closes the Peek card; the action already ran).
- **Risks:** (a) a popup whose renderer never loads never resizes: the UI's 400 ms fallback size
  stays. (b) `RunAction` with a tab in a non-active Browser window: activeTab grant is per tab, fine.
  (c) The install sheet (#12) needs `GetNativeWindow` non-nil: the Web Store tab is visible when the
  user clicks Install, so it is. (d) `chrome.runtime.openOptionsPage` / popup links → `tab_created`
  (opener NULL) → `ChromeTabs` → the UI opens a tab: as CEF's shim did.
- **Test (harness + test-ext from M1, given an action popup that calls `chrome.tabs.query({active:true,currentWindow:true})`
  and writes the result into its title):** `extensions` step prints test-ext with `enabled=1`;
  `popup <id>` → a popup tab handle, `resized w h`, popup title = the active tab's URL (no shim);
  `ext <id> disable` → `extensions` shows `enabled=0`; `ext <id> access 1` → `site_access=1`; `ext
  <id> remove` → gone, `Extensions/<id>` deleted. App: uBlock Origin Lite + 1Password from the Web
  Store (Chrome's install sheet), extensions page details/enable/remove/site access, toolbar popups.

### 7. chrome://settings sheet, fetch, de-Googling

- **//baa:** `baa_tab_set_init_script`: store on `baa::Tab`; `PrimaryMainDocumentElementAvailable()`
  → `GetPrimaryMainFrame()->ExecuteJavaScript(script, {})`. `baa_tab_set_color_scheme`:
  `auto prefs = wc->GetOrCreateWebPreferences(); prefs.preferred_color_scheme = …; wc->SetWebPreferences(prefs);`
  (`blink::mojom::PreferredColorScheme`). `baa_profile_set_pref_bool`: `prefs->FindPreference(name)`
  and `IsUserModifiable()` → `SetBoolean`. `net.mm`: `baa_profile_fetch` as #14, keep the loader in a
  `flat_map<uint64_t, unique_ptr<SimpleURLLoader>>` until the callback, post `fetch_done` (never
  synchronous). Traffic annotation `baa_suggest` (`net::DefineNetworkTrafficAnnotation`).
- **Rust:** `show_chrome_page`: per profile one `CorePage` from `baa_tab_create(profile, url with
  baa:// → chrome://)` + `set_init_script(skin_loader(css))` + `set_color_scheme(dark ? 2 : 1)`;
  `set_floating(true)`, `set_corner_radius`, placed over `sheet.bounds`; URL change → `load_url`; CSS
  change → `eval("window.__baaSkin && __baaSkin(…)")` and a new init script; `None` → `set_visible(false)`.
  Returns true once the page exists (always, unless the profile is gone). The `ChromeSheetPage`
  struct stays, holding a `CorePage`. Its `tab_created`s (links opening tabs) go to `ChromeTabs`.
  `fetch` → id → `HashMap<u64, Box<dyn FnOnce(Option<String>)>>`; `fetch_done` queues and calls it.
  **Privacy** (`baa-privacy`, two lines, core build only): `pub const CORE_SWITCHES = ["disable-background-networking", "no-pings"]`
  and `pub const CORE_DISABLED_FEATURES = ["MediaRouter", "OptimizationHints", "AutofillServerCommunication", "AimEnabled"]`
  (names that exist in 156; the others in `DISABLED_FEATURES` are covered or gone, table below);
  `main` (core) uses them instead of `SWITCHES`/`merge_disabled_features`. After every
  `baa_profile_open`/`open_ephemeral`, `set_pref_bool(name, false)` for each `PREFS_OFF` entry (most
  are already Helium defaults; this pins them if a user flipped one in chrome://settings).
  `block_browser_request` is not called on core.
- **Coverage on the own build** (from the Helium tree; `HP` = `chromium/helium/patches`):

  | baa-privacy item | Own build |
  |---|---|
  | `disable-component-update`, `component-updater=url-source` | covered: `helium/core/component-updates.patch` + `remove-helium-services.patch` (dummy origin) |
  | `gaia-url` | covered: `ungoogled-chromium/disable-gaia.patch`, empty `google_api_key` |
  | `disable-field-trial-config` | covered: `disable_fieldtrial_testing_config=true`, `bromite/disable-fetching-field-trials.patch` |
  | `disable-domain-reliability` | covered: `ungoogled-chromium/disable-domain-reliability.patch` |
  | `disable-breakpad` | covered: `no-crash-uploads.patch` |
  | `disable-client-side-phishing-detection`, `safebrowsing.enabled` | compiled out (`safe_browsing_mode=0`) |
  | `disable-sync` | Google sync replaced by Helium's, Google endpoints substituted |
  | `disable-default-apps`, `no-default-browser-check`, `no-first-run` | moot (no default apps; M1 passes `--no-first-run`) |
  | `disable-background-networking`, `no-pings` | **keep** (`CORE_SWITCHES`) |
  | `MediaRouter`, `OptimizationHints`, `AutofillServerCommunication`, `AimEnabled` | **keep** (`CORE_DISABLED_FEATURES`; endpoints are substituted anyway) |
  | `NetworkTimeServiceQuerying`, `BrowsingTopics` | covered (`disable-network-time-tracker.patch`, `disable-ad-topics-and-etc.patch`) |
  | `DialMediaRouteProvider`, `CastMediaRouteProvider`, `OptimizationHintsFetching`, `OptimizationGuideModelDownloading`, `Fledge`, `InterestGroupStorage` | not features in 156 (no-ops) |
  | `PREFS_OFF` | defaults off already (`inox-patchset/modify-default-prefs.patch`, upstream defaults); set anyway |
  | host blocklist | domain substitution + `trk:` (`block-trk-and-subdomains.patch`) for hard-coded hosts; runtime-built Google URLs (a Google search engine the user picks, Web Store CRX) are allowed, as in CEF |
- **Risks:** `SetWebPreferences` is recomputed on theme changes (`OverrideWebkitPrefs`): re-apply the
  scheme in `OnWebContentsFocused`/after each commit if the sheet flips back. Settings "Import
  bookmarks from HTML" → `browser->GetWindow()` = our window: file picker sheet, works.
- **Test:** harness `fetch https://suggestqueries.google.com/complete/search?client=firefox&q=a` →
  `fetch_done ok=1` with JSON; a bad host → `ok=0`. `pref search.suggest_enabled 0` → cdp
  `chrome://settings` read-back. App: ⌘, → Chrome settings sheet in baa's skin, light and dark;
  password and autofill pages open without crash; command bar suggestions appear. CI: the
  `privacy` action (`PLAN-chromium-phase5.md` §6.4) on the core build is green; if it is not, the
  fallback patch (#15) is the next step.

### 8. 1Password and native messaging (acceptance only)

No code. Manual: fresh `BAA_DATA_DIR`, install 1Password from the Web Store in a Space, 1Password
app running with browser integration on → the extension shows "connected to the app", unlocks with
the app, fills a login form, and its popup offers items for the current tab's site (it sees the real
tab). Check `ls <data>/chromium/NativeMessagingHosts/com.1password.1password.json`. If the app
refuses the browser, that is 1Password's signature allowlist (it asks to trust "baa" once, as it did
for the CEF build); not a Chromium issue.

## Patch list (M2)

| Patch | File(s) | Lines | Why |
|---|---|---|---|
| `context-menu-hook.patch` (new) | new `chrome/browser/ui/views/tab_contents/baa_context_menu_hook.h`; `chrome_web_contents_view_delegate_views_mac.mm`; its GN target | ~10 | #5; no other interception exists |
| `no-views-guards.patch` (M1) | add guards found by M2 tests only | — | risk 1 below |

Nothing else: permissions, beforeunload, downloads, DevTools, extensions, settings, fetch use
setters, factories, observers and user data. Later (M3) candidates: `login-handler-hook.patch`
(`chrome/browser/ui/views/login_handler_views.cc:230`, 3 lines), `install-dialog-hook.patch`
(`GetDefaultShowDialogCallback`, `extension_install_dialog_view.cc:733-736`, 3 lines), and only if
the proxy test fails, `browser-request-filter.patch` (`services/network/url_loader_factory.cc:131`).

## Steps

Each step: a C++ package and a Rust package, in parallel against ABI 4; the manager integrates at
the end of each step with `crates/baa/core.sh`. Step 0 first; then 1–6 in order of value (each
depends only on step 0); 7 last.

| Step | C++ package | Rust package | Depends on |
|---|---|---|---|
| 0 | Apply ABI 4 to `baa.h` (manager). Export stubs for every new function (no-ops / NULL / -1), `baa_abi_version` 4; harness `ffi.rs` to v4; `ExtensionsContainer` + `disable_ui_for_tests` (crash fixes, #11-12); `GetDialogPosition` returns screen coordinates for the parentless views dialogs (`constrained_window_views.cc:288-306`, `native_widget_mac.mm:277-316`). Build, smoke.sh. | `core_host/ffi.rs` to v4 (version check 4, new fn pointers, `BaaMenuItem`, `BaaDownload`, `BaaExtension`, callbacks); fake `Core` gets the new stubs; `CorePage` gains `popup: bool`; dispatch arms that ignore the new callbacks. `cargo test -p baa-web --features core`. | M1 merged |
| 1 | `permissions.mm` (#1), app-modal factory (#2, in `menu.mm`). | Permission events/answers, site permissions, BeforeUnload kinds, quit fallback. Tests: fake `permission` → `PermissionRequested` with kinds in order; answer once calls `baa_permission_answer` once; kind 4 → `BeforeUnload { is_reload: true }`. | 0 |
| 2 | `context-menu-hook.patch` + `menu.mm` (#3). Re-prep (~5 min). | Context menu: rows → `MenuItem` tree (unit test: depth/separators/submenus/inspect), popup via `popup_soon`, `done` exactly once (test with fake). | 0 |
| 3 | `downloads.mm` (#4). | Downloads entity + `DownloadStarted` once per id (test), `cancel_download`. | 0 |
| 4 | `devtools.mm`, hidden TYPE_DEVTOOLS window, `page_message` (#5). | `create_devtools`, dock script (moved from gpui-cef with the two changes), `DevToolsDock`, `open_devtools`. Test: `page_message("devtools-dock:left")` → `DevToolsDock("left")`; other messages dropped. | 0 |
| 5 | `extensions.mm` (#6). | `extensions`/`extension_details`/`change_extension`/`create_popup` on the visitor; `resized` → `PreferredSize`; NULL popup → `Closed`. Test: visitor rows → both shapes, ephemeral → empty. | 0 |
| 6 | `set_init_script`, `set_color_scheme`, `set_pref_bool`, `net.mm` (#7). | Settings sheet on a `CorePage`, `fetch` id map (test: `fetch_done` resolves the right closure once), `PREFS_OFF` per profile, `baa-privacy` core lists + `main` (core) switch set. | 0 |
| 7 | Run all harness scripts of steps 1–6 in one smoke run; fix guards. | `cargo test` for baa-ui/baa-model/baa-web, `cargo check -p baa` (CEF) and `--features core`. Manager: the phase 5 §5.0 parity list on the core build, privacy action, 1Password (#8). | 1–6 |

## M3 (rest of phase 4, after M2; outline)

Concrete enough to plan from, not to build from.

- **`chrome.tabGroups` ↔ folders.** Groups work on our Browsers already (`TabStripModel::SupportsTabGroups`,
  `browser.cc:273-279`; `IsTabStripEditable` true in M1). Mirror direction: baa-model → Chrome for
  live tabs in a folder (`AddToNewGroup`/`AddToExistingGroup`/`ChangeTabGroupVisuals`,
  `tab_strip_model.h:688,697,763`); Chrome → baa-model from `OnTabGroupChanged` (`kCreated`,
  `kVisualsChanged`, `kMoved`, `kClosed`) and `TabGroupedStateChanged`
  (`tab_strip_model_observer.h:554-576`), so an extension's `chrome.tabs.group` makes a folder. Snag
  to settle first: baa folders hold pinned tabs only, most of them not live; Chrome groups hold live
  tabs. Proposed: a group exists in Chrome only while ≥1 of the folder's tabs is live. Needs a
  `WebHost` addition (`folders_changed` entity, like `ChromeTabs`) and tab order in the strip
  mirroring the sidebar (`MoveWebContentsAt`). ABI: `tab_group(ctx, profile, op, …)` +
  `baa_tab_set_group(tab, group_key, title, color)`.
- **Command bar on `AutocompleteController`.** `AutocompleteController(make_unique<ChromeAutocompleteProviderClient>(profile), config)`
  (`autocomplete_controller.h:173`, `chrome_autocomplete_provider_client.h:47`) with providers
  `kHistoryQuick|kHistoryUrl|kBookmark|kShortcuts|kOpenTab|kSearch|kBuiltin|kCalculator`;
  `AutocompleteInput(text, OmniboxEventProto::OTHER, ChromeAutocompleteSchemeClassifier(profile))`;
  `OnResultChanged` → `autocomplete(ctx, id, matches, n, done)`; open = load `destination_url` +
  `ShortcutsBackend::AddOrUpdateShortcut`. History must then be Chrome's (it already records
  visits); baa-model's own history search becomes the fallback. Network suggestions need
  `search.suggest_enabled` true (conflicts with `PREFS_OFF`: the command bar's "suggest" setting
  drives the pref instead of `fetch`). UI-contract change (strictly better ranking):
  `WebHost::autocomplete(profile, text, done)` replacing `fetch` + `parse_suggestions` in `command_bar.rs`.
- **GPUI dialogs (Arc's list):** HTTP auth (`login-handler-hook.patch`, `LoginHandler::SetAuth/CancelAuth`),
  external protocol (`ExternalProtocolHandler::SetDelegateForTesting`, all six methods),
  hung renderer (a `TabDialogs` subclass via `SetUserData`), sad tab (`PrimaryMainFrameRenderProcessGone`
  → `tab_crashed`), extension install/permission prompt (`install-dialog-hook.patch`, `InstallPromptData`
  fields), page fullscreen (`ExclusiveAccessContext::EnterFullscreen` → `fullscreen` callback, Rust
  toggles the window and calls `baa_tab_fullscreen_done` → `WindowFullscreenStateChanged`),
  `chrome.action.openPopup` (`IsToolbarVisible` true + container → `extension_popup_request`),
  dynamic action icons/badges (`ExtensionActionDispatcher::Observer`, `ExtensionActionViewModel::GetIcon`),
  `chrome.commands` (an `ExtensionKeybindingRegistry` subclass; none exists without a focus manager),
  find match counts (`FindResultObserver`), dangerous-download keep/discard.

## Risks and cheapest tests

| # | Risk | Cheapest test | Fallback |
|---|---|---|---|
| 1 | A new path derefs a null BrowserView with our window (the audit found `extension_install_ui_desktop.cc:161`, `extension_context_menu_model.cc:708`, `devtools/protocol/extensions_handler.cc:191-198`; there may be more). | Step 7's smoke run with a debug-ish build; grep `Check failed`; right-click an extension's own context menu item ("Manage extension"). | A guard line in `no-views-guards.patch`. |
| 2 | The hidden TYPE_DEVTOOLS window is chosen as "last active" or steals key status, so `chrome.tabs.query({currentWindow})` answers DevTools. | test-ext `probe()` with DevTools docked. | Exclude TYPE_DEVTOOLS in `BrowserActiveStateManager` calls (already spec'd) and in M1's static active pointer. |
| 3 | `InspectElement` creates the DevTools Browser later than the call returns. | `devtools` harness step with `inspect`. | Open, then inspect the open window. |
| 4 | Chrome's views dialogs (auth, external protocol, install, getDisplayMedia picker) appear off-screen or behind gpui. | httpbin basic-auth, `mailto:` link, Web Store install, `getDisplayMedia`. | Accept for M2 (CEF had worse); M3 draws them. |
| 5 | Popup `ExtensionViewHost` with no Browser window focus closes itself (`ExtensionViewHost` watches deactivation) or never gets a renderer. | `popup <id>` step, then click into it. | Keep the gpui window key while the popup shows; set `host_contents()` focus on show. |
| 6 | The settings tab shows up in an extension's tab list or in `tab_created` when Chrome reloads it. | test-ext `probe()` after ⌘,. | Accept (it is a real tab); or close it when the sheet hides. |
| 7 | Skin script runs after Settings' first paint (flash of Chrome's style). | Open ⌘, on a slow profile; screenshot. | Inject at `ReadyToCommitNavigation` via `blink::mojom` is not public; fall back to hiding the sheet until `tab_state` loading=false (UI already waits on `show_chrome_page` false). |
| 8 | Without CEF's request filter, a background fetch to Google slips through. | Privacy action on the core build. | `browser-request-filter.patch` (#15). |
| 9 | Context menu while the hook's menu outlives its frame (navigate during the menu). | `menu` step then `load` before answering. | Pending menus die with frame/tab (spec'd); done is a no-op. |

# Phase 3, milestone M1: the baa app on BaaCore

Plan for phase 3 of `PLAN-chromium.md`, written 2026-11-06 from Chromium 156 (Helium tree at
`/Users/Shared/hb/helium-macos/build/src`; paths starting with `chrome/`, `content/`, `components/`
are in that tree) and the phase 2 results (`PLAN-chromium-phase2.md`). The C ABI is
`chromium/baa/baa.h` (ABI version 3); it is the contract between the two work packages below.

**M1 is done when** the existing app (`crates/baa` + `crates/baa-ui`, UI code unchanged) runs on
BaaCore instead of CEF: pages per Space profile, navigation, url/title/favicon/loading/history/
audible/closed events, zoom, find, eval (with and without gesture), print, mute, is_secure,
visibility, focus, the page in the gpui hole, `move_to_window`, corner radius / square corners /
floating, ⌘-key routing with the edit fallback, JS dialogs, and tabs Chrome creates itself arriving
at the UI. Pages are real tabs in a real `Browser` + `TabStripModel` whose `BrowserWindow` is baa's
`baa::ui::Window`, so extensions see real tabs.

**Not in M1** (stubs, see "WebHost in M1"): permissions UI, baa-drawn context menus (Chrome's
native NSMenu shows instead), the downloads list (files still land in ~/Downloads),
extension list/popups/toolbar, chrome://settings sheet, DevTools, `fetch`, site permissions, request
filter. Outline in "M2".

## Decisions

| # | Question | Decision |
|---|---|---|
| 1 | Who owns tabs | Chrome. Each `BaaProfile` owns one TYPE_NORMAL `Browser` (created lazily on the first tab, again after it closed). Every tab baa creates goes in it with `chrome::AddAndReturnTabAt(..., foreground=false)`. Chrome-made Browsers (popups, `chrome.windows.create`) also get our window; their tabs are reported like any Chrome-made tab. |
| 2 | How a Browser gets our window | One hook patch: `BrowserWindow::CreateBrowserWindow` asks `g_baa_create_browser_window` first. `BrowserWindowCreateParams::window` is not usable: it is `CHECK_IS_TEST()` (`chrome/browser/ui/browser.cc:310`). |
| 3 | Which Browsers use it | TYPE_NORMAL and TYPE_POPUP. The hook returns null for DevTools, PiP, app windows: Chrome's views windows are used for those, as in the stock `baa.app`. |
| 4 | Shape of `baa::ui::Window` | `chrome/browser/ui/webui_browser/webui_browser_window.{h,cc}` (production, non-views) for close/activation/modal host; `chrome/test/base/test_browser_window.{h,cc}` for the long tail of no-op overrides. Not TestBrowserWindow alone: its null modal host crashes Chrome (#7). |
| 5 | Tab identity | `BaaTab` = `baa::Tab`, keyed by `tabs::TabInterface*`. It survives WebContents replacement (discard: `kReplaced`) and moves between Browsers (`kInsertedIntoOtherTabStrip`). Its NSView can change on replacement, so the view travels in `BaaTabState`. |
| 6 | Events | One `tab_state` snapshot callback (url, title, favicon url, view, flags); Rust diffs it into `PageEvent`s and caches it for the `PageHandle` getters. Plus `tab_favicon` (PNG), `tab_created`, `tab_closed`, `dialog`/`dialog_closed`, `key`. |
| 7 | New tabs from Chrome | Not blocked (unlike CEF, which cancelled them and lost `window.opener`). C++ reports every tab inserted that it did not create (`tab_created`, with opener). Rust emits `OpenRequested` on the opener page, or pushes the URL into `ChromeTabs` when there is no known opener, and keeps the live tab **pending adoption**: the next `create_page(profile, url)` with that exact `(profile, url)` takes the pending tab instead of creating one. Pending tabs not adopted within 60 s are closed. The UI does not change. |
| 8 | Ephemeral spaces | Off-the-record profiles `OTRProfileID::CreateUnique("baa::space")` off the `Default` profile. Chrome refuses Browsers for non-primary OTR profiles (`chrome/browser/profiles/profile.cc:123-138`, CHECK in `Browser::Create` `browser.cc:237`), so a patch allows the `baa::` prefix. Primary OTR would make all incognito Spaces share storage. |
| 9 | Profile lifetime | `ProfileManager::GetProfile(path)` (sync; `profile_manager.h:199`) under `--user-data-dir`, directory `space-<id>`. Each `baa::Profile` holds a `ScopedProfileKeepAlive`: Mac destroys a profile when its last Browser closes (`kDestroyProfileOnBrowserClose`, `chrome/browser/browser_features.cc:79`). Ephemeral: `ProfileDestroyer::DestroyOTRProfileWhenAppropriate` on `baa_profile_close`. |
| 10 | Session restore | baa's Browsers set `should_trigger_session_restore = false` and `omit_from_session_restore = true` (`create_browser_window.h:123,128`; `chrome/browser/sessions/session_service.cc:122-133,419-424`). baa-model is the session. |
| 11 | Keys | The Browser's delegate forwards to `window_->PreHandleKeyboardEvent` / `HandleKeyboardEvent` (`chrome/browser/ui/browser_web_contents_delegate/browser_web_contents_delegate.cc:311-331`). `baa::ui::Window` keeps the spike's filter and calls `key`. The key callback no longer carries a tab: the edit fallback is `[NSApp sendAction:@selector(copy:) to:nil from:nil]` from Rust, which reaches the page's `RenderWidgetHostViewCocoa` (`copy:`, `cut:`, `paste:`, `selectAll:`, `undo:`, `redo:` at `content/app_shim_remote_cocoa/render_widget_host_view_cocoa.mm:2920-2952`). `baa_web_contents_edit` goes. |
| 12 | Visibility | No call. `WebContentsViewCocoa` derives visibility from its NSView (hidden or windowless = hidden; `content/app_shim_remote_cocoa/web_contents_view_cocoa.mm:527-595`). Rust hides the container, as with CEF. `set_visible(true)` also calls `baa_tab_activate`. |
| 13 | Eval | `content::RenderFrameHost::AllowInjectingJavaScript()` once at start-up (process-wide; `render_frame_host.h:183`), then `ExecuteJavaScript` on the primary main frame. Without it `ExecuteJavaScript` CHECKs outside WebUI/about:blank (`render_frame_host_impl.cc:14289-14301`). Gesture: `NotifyUserActivation(kInteraction)` first (`render_frame_host.h:1065`). Same semantics as CEF (main world). |
| 14 | JS dialogs | In M1, because Chrome's own path CHECK-crashes with our window and the UI already draws them. After tab helpers run, replace the tab's `javascript_dialogs::TabModalDialogManager` with one whose delegate is a `JavaScriptTabModalDialogManagerDelegateDesktop` subclass overriding `CreateNewDialog` (no patch). beforeunload stays Chrome's: the Cocoa app-modal `NSAlert` works without views (`components/remote_cocoa/app_shim/alert.mm:223`). |
| 15a | Must-not-be-null getters | From the source audit of `BrowserWindowFeatures::InitPostWindowConstruction` (`chrome/browser/ui/browser_window/internal/browser_window_features.cc:693-1137`, which already skips BrowserView-only features for other windows) and the Browser/delegate paths: `GetExclusiveAccessContext()` (first key press goes through `ExclusiveAccessManager::OnUserInput`, `exclusive_access_manager.cc:266`; popups through `IsFullscreenForBrowser`, `browser_web_contents_delegate.cc:691`), `GetLocationBar()` (`browser.cc:748`), `GetWebContentsModalDialogHost[For]()` (#15), `CreateFindBar()` (`find_bar_controller.cc:56-60` derefs it if `IDC_FIND` ever runs or a tab with an active find moves). `GetNativeWindow()` returns the gpui NSWindow; `IsActive()` is the real key state (`extensions/window_controller_list.cc:112` picks `currentWindow` by it). |
| 15b | Context menu | M1 keeps Chrome's: `WebContentsImpl::ShowContextMenu` → `ChromeWebContentsViewDelegateViewsMac` → `RenderViewContextMenuMacCocoa`, a native NSMenu on the RWHV's view (`chrome/browser/ui/views/tab_contents/chrome_web_contents_view_delegate_views_mac.mm:70,178-188`), which works in a foreign window. `PageEvent::ContextMenu` is not sent in M1. |
| 15c | Downloads | No crash: no shelf in 156, the bubble controller is only made for BrowserView and is null-checked (`download_bubble_update_service.cc:213-218`). Files go to ~/Downloads; Save As uses `SelectFileDialogBridge` on the gpui window. `ConfirmBrowserCloseWithPendingDownloads` must run its callback with `true`, or closing hangs (`unload_controller.cc:1063`). |
| 15 | Everything else Chrome-drawn | `baa::ui::Window` implements a real `web_modal::WebContentsModalDialogHost` (like `webui_browser_modal_dialog_host.cc`). With a null host every tab-modal views dialog CHECKs (`components/constrained_window/constrained_window_views.cc:295`). With ours they show as loose top-level windows (wrong place, but no crash); M2 intercepts each. Permission prompts: a per-tab factory that returns no view, so requests end as "ignored" (the site sees a dismissal). Print: `kPrintPreviewDisabled` per profile + `printing::StartBasicPrint` (system `NSPrintPanel`); print preview needs a modal host. |
| 16 | User data | `<baa data>/chromium` (not CEF's `<baa data>/profile`), so the CEF build keeps working beside it. Migration is a phase 5 question. |
| 17 | Rust home | `crates/baa-web/src/core_host.rs` + `core_host/{ffi,native,menu}.rs`, behind cargo feature `core` (macOS only). `crates/baa` gets feature `core` that switches `main`. gpui-cef stays a dependency until phase 5 (baa-web still re-exports its plain types). |

## Evidence not in the table

- Chrome creates every Browser through `CreateBrowserWindow` → `Browser::Create`
  (`chrome/browser/ui/browser_window/internal/create_browser_window_non_android.cc:140-145`), and the
  Browser ctor calls `BrowserWindow::CreateBrowserWindow(this, user_gesture, in_tab_dragging)`
  (`browser.cc:313-316`), implemented in `chrome/browser/ui/views/frame/browser_window_factory.cc:32`.
  So one hook covers baa's Browsers, `window.open` popups (`NEW_POPUP` always makes a TYPE_POPUP
  Browser, `chrome/browser/ui/navigator/browser_navigator.cc:419-455`), `NEW_WINDOW` (`:456-467`),
  `chrome.tabs.create` with no Browser (`chrome/browser/extensions/api/tabs/tabs_api.cc:2357`),
  `chrome.windows.create` (`:1189`), `chrome.runtime.openOptionsPage`
  (`chrome/browser/extensions/api/runtime/chrome_runtime_api_delegate.cc:412-453`).
- Every tab passes `TabStripModelObserver::OnTabStripModelChanged` `kInserted` on its Browser. The
  window registers as observer in its constructor (the `TabStripModel` already exists; `browser.cc:267-280`).
- Opener: `TabStripModel::GetOpenerOfTabAt` (`tab_strip_model.h:489`), set for `window.open` and link
  clicks with a gesture (`browser_navigator.cc:963-985`). Fallback: `WebContents::GetOpener()`.
  "background" = the inserted tab did not become active in the same change.
- Browser teardown: `Browser::TabStripEmpty` → `window_->Close()` (`browser.cc:617-630`); WebUI
  window's close: `UnloadController::HandleBeforeClose` then `OnWindowClosing`
  (`webui_browser_window.cc:1250-1271`), which closes the tabs or posts
  `SynchronouslyDestroyBrowser` (`chrome/browser/ui/unload_controller.cc:172-221,1200-1235`).
  `~Browser` resets `window_` → `DeleteBrowserWindow` (`browser_window.h:522`).
- Mac does not quit when the last Browser closes: `kBrowserAliveWithNoWindows`
  (`chrome/browser/defaults.cc:10-12`) and AppController's keep-alive (`app_controller_mac.mm:847`).
- Activation: JS dialogs only show for the "foremost" tab, judged by
  `GetLastActiveBrowserWindowInterfaceWithAnyProfile()`
  (`javascript_tab_modal_dialog_manager_delegate_desktop.cc:84`). The window calls
  `BrowserActiveStateManager::From(browser)->DidBecomeActive()` on activation, as
  `webui_browser_window.cc:298,322` does.
- File chooser: `FileSelectHelper` → `SelectFileDialogBridge` with any NSWindow (sheet on a titled
  window), `ui/shell_dialogs/select_file_dialog_mac.mm:100-110`. Works in the gpui window as is.
- Permissions: Chrome's bubble null-derefs without a `BrowserView`
  (`chrome/browser/ui/views/bubble_anchor_util_views.cc:63,80`). Hook:
  `PermissionRequestManager::set_view_factory_for_testing` (`permission_request_manager.h:264`; a plain
  setter, only the name is test-flavoured).

## C++ layout (`chromium/baa/`, few files)

| File | Contents |
|---|---|
| `baa.h` | The ABI (written). |
| `BUILD.gn` | Sources below; deps add `//chrome/browser/ui`, `//components/javascript_dialogs`, `//components/favicon/content`, `//components/zoom`, `//components/find_in_page`, `//components/permissions`, `//components/web_modal`, `//chrome/browser/printing` as GN asks. No `gn check` runs in our gen, so a missing dep shows as a link error, not a gen error. |
| `internal.h` | `baa::Callbacks()` (the copied `BaaCallbacks` + ctx), `baa::Profile`, `baa::Tab`, `baa::ui::Window` declarations, `baa::Terminating()`. |
| `app_mac.mm` | `BaaApplication`, `baa_install_app`, `baa_exit_code`, `baa_abi_version` (from today's `baa_mac.mm` + `baa.cc`). Installs the window hook, calls `AllowInjectingJavaScript()`, registers `AddAppTerminatingCallback` that sets `Terminating()` (no new Browsers/tabs after it). |
| `tab.mm` | `baa::Profile` (path or OTR id, `Profile*`, keep-alive, its Browser), `baa::Tab` (`WebContentsObserver`, `FaviconDriverObserver`, state snapshot, find text), the JS dialog delegate + `baa_dialog_answer`, the permission factory, every `baa_profile_*` / `baa_tab_*` function. |
| `window.mm` | `baa::ui::Window : BrowserWindow, TabStripModelObserver, web_modal::WebContentsModalDialogHost, ExclusiveAccessContext`, stub `FindBar`, its stub `LocationBar`, `CreateWindow()` for the hook. |

Delete `baa.cc` and `baa_mac.mm` (contents move).

### `baa::Profile`
- `Open(dir_name)`: `path = user_data_dir.Append(dir_name)`; reject names with `/` or `..`;
  `g_browser_process->profile_manager()->GetProfile(path)`; same name → same object.
  `ScopedProfileKeepAlive(profile, ProfileKeepAliveOrigin::kBrowserWindow)` (origin enum next to
  `chrome/browser/profiles/scoped_profile_keep_alive.h`; use an existing one, add none). Set `prefs::kPrintPreviewDisabled` true.
- `OpenEphemeral()`: `Default` profile (`ProfileManager::GetLastUsedProfile()`)
  → `GetOffTheRecordProfile(OTRProfileID::CreateUnique("baa::space"), /*create_if_needed=*/true)`.
- `browser()`: the live Browser, or `Browser::Create(params)` with TYPE_NORMAL,
  `should_trigger_session_restore=false`, `omit_from_session_restore=true`, after checking
  `GetBrowserWindowCreationStatusForProfile == kOk` (else null; `Browser::Create` CHECKs it).
  Forget it in `RegisterBrowserDidClose`.
- Observe `ProfileObserver::OnProfileWillBeDestroyed` and null the `Profile*`.
- `Close()`: `DetachAndDeleteWebContentsAt` for each of its tabs without callbacks, drop the
  keep-alive, ephemeral → `ProfileDestroyer::DestroyOTRProfileWhenAppropriate`.
- `baa_profile_*` return/accept `baa::Profile*` checked against a `flat_set` (as the spike's `Alive`).

### `baa::Tab`
- Created for each tab the first time any of our windows sees it inserted (registry
  `flat_map<tabs::TabInterface*, unique_ptr<Tab>>`). A thread-local "creating" flag, set around
  `baa_tab_create`'s `AddAndReturnTabAt`, suppresses `tab_created` for baa's own tabs.
- On creation: `Observe(contents)`, add favicon observer
  (`favicon::ContentFaviconDriver::FromWebContents`), replace the dialog manager
  (`RemoveUserData(TabModalDialogManager::UserDataKey())` then `CreateForWebContents` with
  `baa::DialogDelegate`), set the permission view factory to one returning null.
- State snapshot: `GetVisibleURL`, `GetTitle`, last favicon icon URL (cleared on a cross-document
  primary commit), `GetNativeView().GetNativeNSView()`, `IsLoading()`, `CanGoBack/Forward`,
  `IsCurrentlyAudible()`, `GetSecurityLevel(wc) == SECURE` (`chrome/browser/ssl/chrome_security_state_util.h:27`).
  Send it from `DidStartLoading`, `DidStopLoading`, `TitleWasSet`, `DidFinishNavigation` (primary main
  frame), `NavigationEntryCommitted`, `NavigationListPruned`, `NavigationEntryChanged`,
  `OnAudioStateChanged`, `DidChangeVisibleSecurityState`, favicon updates, and after `kReplaced`
  (re-`Observe` the new contents, re-attach the observers/dialog manager/factory).
- Favicon PNG: from `OnFaviconUpdated(..., image)`: the 2x representation when present
  (`image.AsImageSkia().GetRepresentation(2.0f)` → `gfx::PNGCodec::EncodeBGRASkBitmap`), else
  `image.As1xPNGBytes()`.
- Calls: `load_url` → `NavigationController::LoadURLWithParams` (TYPED); `reload` →
  `Reload(ReloadType::NORMAL, true)`; `stop` → `Stop()`; `go_to_offset` → `CanGoToOffset` then
  `GoToOffset`; `history` → `GetEntryCount/GetEntryAtIndex/GetCurrentEntryIndex` (title = entry
  title or URL); `eval` → primary main frame, `NotifyUserActivation` if gesture, `ExecuteJavaScript`;
  `zoom` → `zoom::ZoomController::FromWebContents(wc)->SetZoomLevel(blink::ZoomFactorToZoomLevel(f))`;
  `find` → `find_in_page::FindTabHelper` (same text = find next; empty → `StopFinding(kClearSelection)`);
  `print` → `printing::StartBasicPrint(wc)`; `mute` → `SetTabAudioMuted(wc, m, TabMutedReason::kAudioIndicator, "")`
  (`chrome/browser/ui/tabs/tab_muted_utils.h:34`); `activate` → `tab_strip_model->ActivateTabAt(index)` and
  the window's `Activate()`; `focus` → `wc->Focus()`; `close` → mark "closing by baa", then
  `DetachAndDeleteWebContentsAt(index)` (`tab_strip_model.h:368`).
- `tab_closed` from the window's `kRemoved` with reason `kDeleted`, unless closing by baa.

### `baa::DialogDelegate` (JS dialogs)
Subclass of `JavaScriptTabModalDialogManagerDelegateDesktop`; overrides `CreateNewDialog(...)` only.
It returns a `TabModalDialogView` (weak) that stores `DialogClosedCallback` and the force-close
closure under a new `uint64_t` id, then calls `dialog(ctx, tab, id, kind, message, default)`.
`CloseDialogWithoutCallback` → `dialog_closed(ctx, tab, id)` and forget the id.
`baa_dialog_answer(id, ok, text)` → run the callback `(ok, text)` and forget the id. Kinds map from
`content::JavaScriptDialogType`.

### `baa::ui::Window`
- Ctor `(BrowserWindowInterface* browser)`: keep it; `browser->GetTabStripModel()->AddObserver(this)`.
- `Close()`: posts (weak pointer; `TabStripEmpty` calls it from inside a tab-strip notification)
  `UnloadController::From(browser_)->OnWindowClosing()`, which runs beforeunload, closes the tabs,
  and on the second call (strip empty) schedules `SynchronouslyDestroyBrowser`. Repeat calls are
  no-ops once delete is scheduled (`unload_controller.cc:178`). Never delete the Browser itself.
  Quit (`BrowserCloseManager::CloseBrowsers`, `lifetime/browser_close_manager.cc:218-238`) relies on it.
- `ConfirmBrowserCloseWithPendingDownloads`: `std::move(callback).Run(true)`.
- `DeleteBrowserWindow()`: `browser_->GetFeatures().TearDownPreBrowserWindowDestruction()` then
  `delete this` (TestBrowserWindow's destructor does the teardown call).
- `Activate()` / `IsActive()`: `BrowserActiveStateManager::From(browser_)->DidBecomeActive()` and
  remember it; `Deactivate` the reverse for the previously active window (keep one static "active"
  window pointer).
- `GetNativeWindow()`: `[active tab NSView window]` (nil when not in a window). `GetBounds()`: that
  window's frame in screen coordinates (CDP's `Browser.getWindowForTarget` and
  `chrome.windows.get` read it).
- `GetWebContentsModalDialogHost[For]()`: `this`. Host: `GetHostView` = active tab's
  `GetNativeView()`, `GetDialogPosition` = centred horizontally at the top of the host,
  `GetMaximumDialogSize` = host size, observers in a `base::ObserverList`.
- `GetLocationBar()`: a stub `LocationBar` (copy TestBrowserWindow's `TestLocationBar`; never null:
  `Browser::OnTabDeactivated` dereferences it, `browser.cc:748`).
- `GetExclusiveAccessContext()`: `this` as `ExclusiveAccessContext`
  (`exclusive_access/exclusive_access_context.h`; shape: `WebUIBrowserExclusiveAccessContext`).
  M1: `IsFullscreen` false, `EnterFullscreen`/`ExitFullscreen` no-ops (ponytail: page fullscreen
  M2, toggling the gpui window and calling `fullscreen_controller()->WindowFullscreenStateChanged()`),
  `UpdateExclusiveAccessBubble` runs its callback at once, `GetWebContentsForExclusiveAccess` = active tab.
- `CreateFindBar()`: a stub `FindBar` whose methods do nothing (baa finds through `FindTabHelper`;
  `baa_tab_find("")` also runs before a tab is detached).
- `GetStatusBubbles()`: `{}`. `GetDownloadBubbleUIController()`, `AsBrowserView()`: null (checked).
- `PreHandleKeyboardEvent` / `HandleKeyboardEvent`: the spike's `Offer` (RawKeyDown, ⌘/⌃ or Escape,
  `os_event` present) → `key(ctx, ns_event, after_page)`.
- `OnTabStripModelChanged`: `kInserted` → new `baa::Tab` + `tab_created` if not ours and not
  already known; `kRemoved` → per reason; `kReplaced` → `Tab::Replace`.
- Everything else: TestBrowserWindow's bodies (no-ops, false, empty rects).
- `CreateWindow(BrowserWindowInterface* b, bool)`: `b->GetType()` is TYPE_NORMAL or TYPE_POPUP →
  `new Window(b)`, else nullptr.

## Chrome patches

All in `chromium/patches/baa/`, appended to `series`. A patch change re-preps the tree (~5 min) plus
the rebuild; `//baa` edits rebuild in ~20 s, so keep logic in `//baa`.

| Patch | File(s) | What | Why |
|---|---|---|---|
| `embed-in-gpui.patch` (exists) | `framework.exports`, `app_controller_mac.mm`, `chrome_browser_main_mac.mm` | unchanged | phase 2 |
| `browser-window-hook.patch` (new) | new `chrome/browser/ui/views/frame/baa_browser_window_hook.h`; `chrome/browser/ui/views/frame/browser_window_factory.cc`; `chrome/browser/ui/views/frame/BUILD.gn` (add the header to `browser_window_factory`'s sources) | Header declares `using BaaCreateBrowserWindowFn = BrowserWindow* (*)(BrowserWindowInterface*, bool user_gesture); extern BaaCreateBrowserWindowFn g_baa_create_browser_window;`. The .cc defines it (null) and, first thing in `CreateBrowserWindow`: `if (g_baa_create_browser_window) if (BrowserWindow* w = g_baa_create_browser_window(browser, user_gesture)) return std::unique_ptr<BrowserWindow, BrowserWindowDeleter>(w);` | //chrome cannot depend on //baa; a function pointer //baa sets keeps the direction. A new header (not `browser_window.h`) so the patch recompiles one file. |
| `no-views-guards.patch` (new) | `chrome/browser/ui/sad_tab_controller.cc` (`FindContentsWebView`, `:27-35`) | return null when `BrowserView::GetBrowserViewForBrowser` is null instead of `DCHECK` + deref | A renderer crash in any baa tab otherwise null-derefs (SadTabHelper is created unless the WebUI browser flag is on, `tabs/tab_features.cc:641`). Add further one-line guards here as the risk-1 test finds them. |
| `baa-otr-browsers.patch` (new) | `chrome/browser/profiles/profile.cc` `OTRProfileID::AllowsBrowserWindows` | also allow ids starting with `"baa::"` | Ephemeral Spaces need separate OTR profiles with Browsers (#8). |

That is all M1 needs (plus guards the risk-1 test finds). JS dialogs, permissions, print and eval need no patch.

## Rust layout

### `crates/baa-web`
- `Cargo.toml`: feature `core = []`; under `[target.'cfg(target_os = "macos")'.dependencies]` the
  spike's `raw-window-handle`, `core-video`, `core-foundation`, `objc2`, `libc` (same versions as
  vendor/gpui-cef; nothing new to the lock file).
- `src/lib.rs`: `#[cfg(all(feature = "core", target_os = "macos"))] mod core_host; pub use core_host::{CoreRuntime, CoreWebHost, CorePage};`. Nothing else changes; the trait stays as is.
- `src/core_host.rs`: `CoreRuntime`, `CoreWebHost: WebHost`, `CorePage: PageHandle + Render + Focusable + EventEmitter<PageEvent>`.
- `src/core_host/ffi.rs`: from `crates/baa-chromium/src/ffi.rs`: `load()` (dlopen, version check
  against `BAA_ABI_VERSION` 3), `Core` = struct of `extern "C" fn` pointers for every function in
  `baa.h`, `#[repr(C)] BaaTabState`, `#[repr(C)] BaaCallbacks`. Make `Core` constructible in tests
  from Rust `extern "C" fn` stubs (it is plain data; tests build one with fakes).
- `src/core_host/native.rs`: the spike's `native.rs` plus, from
  `vendor/gpui-cef/.../platform/cef/native.rs`, the `Container` features M1 needs:
  `set_corner_radius`, `set_square_corners`, `set_floating`, `move_below`, `set_visible`, `punch_buffer(radius, square)`.
  Add `swap_page(old, new)` for a changed `view`, and `send_edit_action(sel)` (`[NSApp sendAction:to:nil from:nil]`).
- `src/core_host/menu.rs`: copy of `vendor/gpui-cef/.../platform/cef/menu.rs` for `WebHost::show_menu`
  (pure AppKit).

`CorePage` mirrors gpui-cef's `Webview` minus CEF: fields `tab: *mut BaaTab`, cached
`BaaTabState` (owned strings), `container`, pointer/drag state, `gpui_view`, `corner_radius`,
`square_corners`, `focus_handle`, `pending_dialog: Option<u64>`, `this`. Render = gpui-cef's
`Render for Webview` + `PageSlot` element (`mod.rs:783-980`), with `baa_tab_focus` where CEF set
browser focus. Focus in → `baa_tab_focus` + first responder; focus out → gpui view first responder
(gpui-cef's `set_browser_focus`). `Drop` → hide container, `baa_tab_close`.

Dispatch: callbacks arrive with `ctx` = a leaked `Box<Dispatch>` holding an `AsyncApp`, a
`RefCell<HashMap<*mut BaaTab, WeakEntity<CorePage>>>` and the pending adoptions. Every callback
except `key` only queues (`app.spawn`), per the phase 2 re-entrancy rule. Before `start` there is
no `AsyncApp`; callbacks then do nothing (none can fire before gpui's launch closure: Chrome makes no
tabs on its own with `--no-startup-window`).

Events from `tab_state` (diff against the cache, in this order): `UrlChanged`, `TitleChanged`,
`FaviconChanged(None if "")`, `LoadingChanged`, `HistoryChanged`, `AudibleChanged`; a changed
`view` swaps it in the container (no event). `tab_favicon` → `FaviconImage`. `tab_closed` →
`Closed`. `dialog` → `Dialog { kind, message, default_text }` and remember the id; `dialog_closed`
→ `DialogClosed`; `answer_dialog` → `baa_dialog_answer(id, ok, text)`.

`tab_created(tab, opener, profile, url, background, gesture)`:
1. Insert `(profile, url) → (tab, Instant::now())` into pending; start a 60 s timer that
   `baa_tab_close`s it if still pending.
2. If `opener` maps to a live `CorePage`, emit `OpenRequested { url, from_user_gesture: gesture,
   background }` on it; else push `url` into `ChromeTabs` (which window takes it is the UI's call,
   as today). `profile == NULL`: close the tab now, push the URL only.
3. `create_page(profile, url)`: if pending has `(handle(profile), url)`, take it: build the
   `CorePage` around that tab (`baa_tab_get_state` for the first snapshot). Else `baa_tab_create`.

`WebHost` in M1:
| Method | M1 |
|---|---|
| `create_page` | as above; profile handle from `baa_profile_open("space-<id>")` or the ephemeral map. A null tab (terminating) gives a `CorePage` with no tab that renders empty. |
| `ephemeral_profile` | counts down from `u64::MAX` (as CEF); `baa_profile_open_ephemeral` on first use. |
| `delete_profile` | `baa_profile_close`; disk → `queue_delete` (CEF code, moved to a shared spot or copied) for the next launch's `purge_deleted_profiles`, which `main` runs before `baa_install_app`. |
| `show_menu` | `menu.rs` (real). |
| `extensions` | `installed_extensions(<chromium dir>/space-<id>)` (same on-disk layout; move the fn out of `cef_host.rs` so both use it). |
| `chrome_tabs`, `downloads` | real entities; downloads stays empty in M1. |
| `create_devtools` | M1: a `CorePage` on `about:blank` in the inspected page's profile; `open_devtools`: log. |
| `create_popup` | M1: `create_page(profile, url)` (the popup page as a background tab; `chrome.tabs.query({active})` still answers the shown tab because baa activates it). |
| `extension_details` | `done(Some(vec![]))`. `change_extension` → `done(false)`. |
| `show_chrome_page` | `true` (nothing shown), log once. |
| `fetch` | `done(None)`. `cancel_download`: no-op. |
| `site_permission` | `Ask`; `set_site_permission`: no-op. |
| `answer_permission` | no-op (no requests are sent in M1). |

### `crates/baa`
- `Cargo.toml`: `[features] core = ["baa-web/core"]`.
- `src/main.rs`: `#[cfg(feature = "core")] fn main()` next to the CEF one (CEF one under
  `not(feature = "core")`). Order (the first line is load-bearing):
  1. `let core = baa_web::CoreRuntime::load();` (dlopen; single-threaded; PartitionAlloc).
  2. `env_logger`, `data_dir()` (honour `BAA_DATA_DIR` if set, for tests), URL args, instance lock
     and `forward` exactly as today (the lock runs before Chrome, so Chrome's own process singleton
     never sees a second instance).
  3. `purge_deleted_profiles(<data>/chromium)`, `share_native_messaging_hosts(<data>/chromium)`.
  4. Chrome argv: `argv[0]`, every `-`-prefixed arg the user passed, `--no-startup-window`,
     `--no-first-run`, `--user-data-dir=<data>/chromium` unless given, `baa_privacy::SWITCHES`,
     `--disable-features=<merged>`. URLs are not passed to Chrome. Leak the `CString`s.
  5. `core.install(argv)` (→ `baa_install_app` with the dispatch callbacks).
  6. `listen()` (spawns its thread; after dlopen).
  7. `app.run(|cx| { let host = core.start(cx); ... })` — the rest of today's closure unchanged with
     `Browser<CoreWebHost>`.
  8. `std::process::exit(core.exit_code())`.
- `crates/baa/core.sh` (from `crates/baa-chromium/spike.sh`): `cargo build -p baa --features core`;
  refresh `out/Default/baa Dev.app` from `out/Default/baa.app` when the GN app is newer (it must
  stay in `out/Default`: component rpath); copy the binary over `Contents/MacOS/baa`; sign with the
  first Apple Development identity (`BAA_SIGN_IDENTITY` overrides); `exec` it with `"$@"`; `--no-run`
  stops after signing.

### Later deletions (phase 5, not M1)
`vendor/gpui-cef`, `cef_host.rs`, the CEF `main`, `crates/baa/src/bin/helper.rs`, the `cef`
dependency and `cargo bundle` metadata; move `DialogKind`, `Download`, `DownloadState`, `MenuItem`,
`Permission`, `PermissionState`, `Error` from gpui-cef into `baa-web/src/lib.rs` (plain enums,
copy); the feature gate. `crates/baa-chromium` goes at the end of M1 (its pieces moved; see the C++
package's harness note).

## Work packages

Both start from `baa.h` as committed. Neither edits the other's files. Questions about the ABI go
to the manager; an ABI change bumps `BAA_ABI_VERSION` and is announced to both.

### C++ package (owns `chromium/baa/*`, `chromium/patches/baa/*`, `crates/baa-chromium/*`)

1. Patches `browser-window-hook.patch`, `no-views-guards.patch`, `baa-otr-browsers.patch`; add to `series`. Re-prep and
   build once (`chromium/local-build.sh`, ~5 min + build). Check: stock `out/Default/baa.app` still
   starts with its views window (hook unset → stock path).
2. `app_mac.mm` (moved code + hook install + `AllowInjectingJavaScript` + terminating flag),
   `internal.h`, `BUILD.gn`. Build (~20 s per `//baa` edit from here on).
3. `window.mm`: `baa::ui::Window` with all overrides; the hook. `tab.mm`: profiles, tabs, state,
   favicon, calls. Then the dialog delegate and permission factory.
4. Harness: update `crates/baa-chromium` (`ffi.rs`, `main.rs`) to ABI 3 so it drives every call:
   open `space-1`, `space-2`, an ephemeral profile; create tabs; print every callback to stderr as
   one line (`baa: tab_state <tab> url=... loading=...`). It keeps the spike's single page in the
   hole (now `baa_tab_create` + `state.view`). Add `BAA_SPIKE_SCRIPT=<file>`: a list of steps
   (`load <url>`, `back`, `eval <js>`, `find <text>`, `zoom 1.5`, `close`, `quit`) run with 1 s
   gaps, so tests are scripted.
5. `crates/baa-chromium/test-ext/`: a MV3 extension whose service worker exposes
   `globalThis.probe = async () => ({ tabs: await chrome.tabs.query({}), windows: await chrome.windows.getAll() })`,
   loaded with `--load-extension` into the harness run.

Acceptance (all with the harness, `smoke.sh`, `cdp.mjs` on `--remote-debugging-port`):
- `nm -gU out/Default/libchrome_dll.dylib | grep _baa_` lists every function in `baa.h`.
- Start, 2-minute idle, ⌘Q: exit 0, no Crashpad report, no `Check failed` (smoke.sh).
- Two disk profiles + one ephemeral: `<user-data-dir>/space-1`, `space-2` exist; the ephemeral one
  writes nothing to disk; cookies set in `space-1` (`document.cookie` via cdp) are not visible in
  `space-2` or the ephemeral tab.
- The page renders in the hole; navigation steps produce the expected `tab_state` lines (url,
  title, loading true→false, can_go_back after a second load); `history` lists both entries;
  `tab_favicon` arrives with PNG magic bytes for a site with an icon.
- `eval` sets `document.title` (seen in `tab_state`); `eval` with gesture of
  `document.body.requestFullscreen()` does not reject (log the promise via title).
- `window.open('https://example.com')` from a page with a click → `tab_created` with `opener` =
  that tab and `background=false`; `window.open(url,'', 'popup')` → `tab_created` too (popup
  Browser through the hook, no views window appears); ⌘-click a link → `background=true`;
  `probe()` from the extension lists all tabs with their URLs and one window per profile;
  `chrome.tabs.create({url})` from the extension → `tab_created` with `opener=NULL`.
- `window.close()` in a script-opened tab → `tab_closed`. `baa_tab_close` → no callback, and later
  calls on that handle are no-ops (no crash).
- `alert('x')` → `dialog` kind 0; `baa_dialog_answer` resumes the page; `confirm` answered false
  returns false; navigating away while one is open → `dialog_closed`.
- `navigator.geolocation.getCurrentPosition` and `Notification.requestPermission()` → no crash,
  the page gets a denial/dismissal.
- `print` opens the system print panel; `<input type=file>` opens a sheet on the window.
- `onbeforeunload` page + `baa_tab_close` → closes without prompt; ⌘Q with such a page → Chrome's
  sheet appears or the app quits; never a hang (smoke.sh's 30 s limit).
- Close all tabs of a profile, then create one again in it → works (new Browser).
- Right-click shows Chrome's NSMenu at the cursor; a download lands in ~/Downloads; a renderer
  crash (`chrome://crash` in a tab) does not crash the browser.
- Key routing unchanged from phase 2 (spike self-test still passes, with the edit fallback now
  via `sendAction`).

### Rust package (owns `crates/baa-web/*` except `cef_host.rs` behaviour, `crates/baa/*`)

1. `core_host/ffi.rs`, `native.rs`, `menu.rs` (moves/copies as above). `cargo check -p baa-web --features core`.
2. `core_host.rs`: `CoreRuntime::load/install/start/exit_code`, dispatch, `CorePage`,
   `CoreWebHost`, pending adoption with expiry.
3. `crates/baa` feature `core`, `main.rs` core path, `BAA_DATA_DIR`, `core.sh`.
4. Unit tests with a fake `Core` (Rust `extern "C" fn` stubs recording calls; tabs are fake
   pointers) and gpui's `TestAppContext`:
   - a `tab_state` sequence yields exactly the expected `PageEvent`s (no duplicates on an unchanged
     snapshot; favicon `""` → `FaviconChanged(None)`);
   - `tab_created` with a known opener emits `OpenRequested` on that page; without one it pushes
     to `ChromeTabs`; `create_page` with the same profile+url adopts (no `baa_tab_create` call),
     with another url creates; an unadopted tab is closed after the expiry (inject the clock or
     make the expiry a const the test shortens);
   - `dialog` → `PageEvent::Dialog`, `answer_dialog` calls `baa_dialog_answer` with that id, a
     second answer does not;
   - dropping a `CorePage` calls `baa_tab_close` once; `tab_closed` for an unknown tab is ignored;
   - ephemeral ids count down and map to one `baa_profile_open_ephemeral` each.
   - `cargo test -p baa-web --features core`, `cargo test -p baa-ui` (unchanged UI still green),
     `cargo check -p baa` (CEF build) and `cargo check -p baa --features core`.

Acceptance for the package on its own: those tests green, `crates/baa/core.sh --no-run` produces a
signed bundle (it links and loads only when the C++ package's framework is built; until then the
version check fails loudly, which is the expected result).

## Integration (the manager, end to end)

Run `crates/baa/core.sh` with a fresh `BAA_DATA_DIR`, then:
1. The old UI comes up (sidebar, Spaces, the start page); log shows Chrome's browser process and
   `baa_profile_open("space-<id>")` for the active Space only.
2. Open three tabs, type a URL in the command bar, back/forward/reload, history menu, zoom ⌘+,
   find ⌘F, mute, print, reader/PiP tools: each works as on CEF.
3. Switch Space → another profile: a page logged in on Space A is logged out on Space B;
   `ls <data>/chromium` shows both `space-*` dirs. An incognito Space writes no `space-*` dir.
4. `window.open` with a click → Peek shows the popup and `window.opener` is non-null in it
   (cdp: `!!window.opener`); ⌘-click → background tab appears in the sidebar; extension
   `chrome.tabs.create` (test-ext) → the tab appears in the active Space.
5. Close tabs from the sidebar; `window.close()` in a script-opened tab closes its Peek/tab.
6. Keys: ⌘T/⌘L/⌘W go to gpui with the page focused; ⌘C/⌘V/⌘A/⌘Z work in a page textarea;
   Escape closes Peek and still reaches the page when nothing in gpui takes it.
7. `alert`/`confirm`/`prompt` show baa's dialog and resume the page with the answer.
8. Move a page to lil-baa and back (`move_to_window`): the page keeps state.
9. ⌘Q with tabs open: exit 0 within 5 s, no Crashpad report; relaunch restores the tabs from
   baa.db and their pages load.
10. `cargo test` for baa-ui, baa-model, baa-web; the CEF build (`cargo bundle` without `core`)
    still launches.

## Risks and cheapest tests

| # | Risk | Cheapest test | Fallback |
|---|---|---|---|
| 1 | A Browser feature null-derefs our window (views assumptions) during create/activate/close. | C++ step 3's first run with a debug build: create, activate, close a tab, ⌘Q. `Check failed` in the log names it. | Return a real object from the getter it uses (see the audit list) or a one-line guard patch. |
| 1b | A Chrome command or toast that assumes a BrowserView runs (pinned-tab close `CHECK(browser_view)` `browser_commands.cc:1535`, `SidePanelUI::From` unchecked in `browser_command_controller.cc`, toasts `CHECK_IS_TEST` at `toast_controller.cc:323`). baa runs no Chrome commands, so only extensions or chrome:// pages could. | grep the run's log for `Check failed`; try `chrome.tabs.update({pinned:true})` then remove. | A guard line in `no-views-guards.patch`. |
| 2 | A Chrome-drawn tab-modal dialog (HTTP auth, external protocol) appears as a loose window or behind gpui. | Visit `https://httpbin.org/basic-auth/a/b`. | Accept in M1; M2 intercepts (`HttpAuthCoordinator`, `ExternalProtocolHandler`). |
| 3 | Memory saver discards a hidden tab and the view swap path is wrong. | `chrome://discards` → "Discard" on a hidden tab, then show it. | Disable discarding with `--disable-features` for M1. |
| 4 | Pending-adoption mismatch (redirect before `create_page`, URL string differs) → duplicate tab. | Integration step 4 with a redirecting link. | Key by the URL string Rust emitted (it is) — the UI passes it back unchanged; if not, key by opener+order. |
| 5 | `ProfileManager::GetProfile` blocks noticeably on a Space switch to a never-loaded profile. | Time `baa_profile_open` on a fresh profile (log ms). | `CreateProfileAsync` + pending page that attaches when ready (an ABI change). |
| 6 | AppController edits gpui's main menu items by tag, or rebuilds the menu on profile change. | Switch Spaces, open the menu bar. | Stop AppController's menu updates in `embed-in-gpui.patch`. |
| 7 | Keychain prompts ("baa Storage Key") because the dev bundle id `me.baa.browser` is shared with the CEF build. | First launch of `baa Dev.app`. | Sign identity as in phase 2; accept "Always Allow" once. |
| 8 | `chrome.windows.create({incognito:true})` or a profile baa never opened gets a Browser (profile = NULL path). | test-ext call. | Tab is closed and its URL goes to ChromeTabs (spec'd). |
| 9 | `--remote-debugging-port` runs never quit on ⌘Q (phase 2 gotcha). | — | Tests that use cdp.mjs end with `Browser.close`, or smoke.sh's kill. |

## M2 outline (delegate surface)

Each item: C++ hook → ABI → Rust/UI. Answers that Chromium waits for use a `uint64_t` id resolved by
`baa_*_answer(id, ...)` (as `baa_dialog_answer`); unknown ids do nothing.

- **Permissions:** per-tab factory (`set_view_factory_for_testing`, or a one-line hook in
  `CreatePermissionPrompt`, `chrome/browser/ui/views/permissions/permission_prompt_factory.cc:292`)
  returns `baa::PermissionPrompt` (implements `components/permissions/permission_prompt.h:173-196`),
  which sends `permission(ctx, tab, id, origin, kinds[])`; `baa_permission_answer(id, allow)` →
  `delegate->Accept(std::monostate())` / `Deny()`; prompt destroyed → `permission_dismissed`.
  Site permissions: `HostContentSettingsMapFactory::GetForProfile(p)->Get/SetContentSettingDefaultScope`
  via `baa_profile_site_permission(profile, url, kind)` / `baa_profile_set_site_permission`.
- **Context menu:** `BrowserWebContentsDelegate` does not override `HandleContextMenu`
  (`content/public/browser/web_contents_delegate.h:329`); a small patch adds an override that asks
  `baa::ui::Window` and returns true, to send `context_menu(ctx, tab, x, y, link_url, selection)`.
  baa then draws Chrome's useful items (Inspect, Copy link, Open in new tab); custom page items and
  `NotifyContextMenuClosed` become baa's job.
- **Downloads:** `content::DownloadManager::Observer` + `download::DownloadItem::Observer` per profile
  → `download(ctx, profile, id, url, path, received, total, state)`; `baa_download_cancel(id)`.
  Also `ConfirmBrowserCloseWithPendingDownloads` → answer "close" for now.
- **HTTP auth / external protocol / file chooser:** `ChromeContentBrowserClient::CreateHttpAuthCoordinator`
  (`chrome_content_browser_client.cc:7662`) or a small patch; `ExternalProtocolHandler::SetDelegateForTesting`;
  `ui::SelectFileDialog::SetFactory` if baa wants its own picker.
- **Extensions:** `extensions::ExtensionRegistry` + `ExtensionRegistryObserver` per profile →
  `extensions_changed`; `baa_profile_extensions(profile, visitor)` (id, name, icon path, popup,
  options, enabled, permissions, site access from `ExtensionActionManager` / `PermissionsManager`);
  `ExtensionService` enable/disable/uninstall for `change_extension`; toolbar actions and badges via
  `ExtensionActionDispatcher` / `ExtensionAction` observers; popups via
  `ExtensionPopupHost`-style `ExtensionViewHost` (`ExtensionViewHostFactory::CreatePopupHost`) whose
  WebContents NSView goes in a gpui popover, size via `ExtensionViewHost::OnResizeDueToAutoResize`.
  Install/permission prompts: `ExtensionInstallPrompt` show-dialog callback (global
  `ExtensionInstallPrompt::GetDefaultShowDialogCallback` override).
- **chrome://settings sheet:** a tab-less `WebContents` in the profile (`WebContents::Create`), view
  in a gpui overlay, skin via `eval` on `DOMContentLoaded` (the CEF `skin_loader` script as is).
- **DevTools docked:** `DevToolsWindow` docking talks to `BrowserView`; check
  `DevToolsWindow::GetInTabWebContents(inspected, &strategy)` and patch the docking call to ask
  `baa::ui::Window`, then give Rust the DevTools WebContents' view as a `CorePage`.
- **fetch:** `network::SimpleURLLoader` on `profile->GetURLLoaderFactory()`;
  `baa_profile_fetch(profile, url, id)` → `fetch_done(ctx, id, body, len, ok)`.
- **Request filter / prefs off:** a `content::ContentBrowserClient` URLLoader throttle (patch
  `ChromeContentBrowserClient::CreateURLLoaderThrottles`) calling a `block_request` callback, or drop
  it if the CI proxy test passes on Helium's patches alone.
- **Omnibox:** `AutocompleteController` per profile for the command bar (phase 4).
- **Tab groups ↔ folders:** `chrome.tabGroups` through `TabStripModel` groups mirrored from baa-model.

## Tooling

- `chromium/local-build.sh`: `//baa` edit → ~20 s; a patch change → re-prep (~5 min) then build.
  Never two builds (`pgrep -x siso`). Only the manager runs it.
- `crates/baa-chromium/smoke.sh <exe> <secs>`: start, wait, quit like ⌘Q, report exit code, fatal
  lines, crash dumps. Works for `baa Dev.app` too (pass `BAA_DATA_DIR`).
- `crates/baa-chromium/cdp.mjs <port> '<expr>'`: page-side checks over DevTools.
- `crates/baa-chromium/spike.sh`: the C++ package's harness run.

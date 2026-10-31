# baa — own Chromium build + GPUI shell, ADK-style (draft, 2026-10-31)

Replaces PLAN.md §1, §2 (Engine host, De-Googling, Extensions rows), §5, §7 and the CEF parts of §13 once
phase 2 passes. Until then CEF stays the shipping engine. baa is open source under GPL-3.0.

## Goal

Build Chromium from source the way Helium does (Helium's patch set, chained CI), and draw the whole outer
chrome in Rust/GPUI the way Arc does it in Swift: an **ADK-style layer** inside Chromium that exposes
browsers, profiles and tabs to the UI, so UI work never touches C++. Chrome's views UI (`BrowserView`,
tab strip, toolbar, omnibox) is never created. Small patches to Chromium-drawn dialogs are fine.

## What Arc's ADK actually is (read from Arc 1.x on this Mac, 2026-10-31)

Public sources only say ADK is "an internal SDK for building browsers … lets ex-iOS engineers prototype
native browser UI quickly, without touching C++". The installed `Arc.app` shows how:

- **Bundle:** `MacOS/Arc` (Swift, 158 MB) links `Frameworks/ArcCore.framework` (Chromium + ADK, 500 MB,
  version 8078.x = Chromium build number). Chromium's helper apps live inside the framework
  (`ArcCore.framework/Helpers/Browser Helper (Renderer|GPU|Alerts).app`), plus `app_mode_loader` for PWAs.
- **ADK source lives in the Chromium tree at `//arc`** (paths `../../arc/browser/ui/window.cc`, …).
- **Three layers:**
  1. `arc::` C++ over `//chrome`: `arc::BrowserApplication`, `arc::BrowserContext` (= Profile),
     `arc::Browser`, `arc::SessionWindow`, `arc::PermissionRequest`, each a value type holding
     `shared_ptr<Impl>`. Delegates: `arc::BrowserApplicationDelegate` (`DidInitializeBrowserProcess`,
     `DidCreateNewBrowser`, `DidReceiveGPUCrashNotification`, …), `BrowserDelegate`,
     `BrowserContextDelegate`, `WebContentsUIDelegate`.
  2. **`arc::browser::ui::Window` implements Chrome's `BrowserWindow`** (`AsBrowserView`,
     `CreateFindBar`, `GetLocationBar`, `OnActiveTabChanged`, `PreHandleKeyboardEvent`,
     `ShowBookmarkBubble`, … mostly no-ops or forwards), and `arc::browser::ui::LocationBar` implements
     `LocationBar`. So Arc keeps real Chrome `Browser` objects with its own window.
  3. Objective-C wrappers exported from the framework for Swift: `ArcBrowserApplication`, `ArcBrowser`,
     `ArcBrowserContext` (+ `…Extensions/Downloads/History/Preferences/Autocomplete/DevTools`),
     `ArcWebContents` (+ `…NavigationController/MediaSession/PiP/PDF/DevTools`), and data classes
     (`ArcSessionTab`, `ArcExtensionToolbarAction`, `ArcPermissionRequest`, `ArcMenuModel`,
     `ArcAutocompleteMatch`, `ArcDownloadItem`, …). C++ `…DelegateBridge` objects turn Chromium events
     into ObjC delegate calls.
- **Delegate surface (selectors, abridged):**
  - browser: `didCreateNewWebContents:isActive:`, `didOpenExtensionPopup:webContents:`,
    `didResizeExtensionPopup:size:`, `didUpdateExtensionToolbarActions:`,
    `didRequestExtensionPermissions:completion:`, `createTabGroupWithSessionIds:properties:completion:`
    (+ get/update/move/remove tab groups: **`chrome.tabGroups` is answered by the Swift UI**),
    `didChangeFullscreenState:initiatedBy:`.
  - browserContext: extension install/uninstall/enable/errors, `extensionPermissionPrompt:…completion:`,
    downloads (`onDownloadCreated/Updated/Removed`, `onDeterminingFilename:…`), `didVisitURL:`,
    `showDevTools:for:`, search engine and preference changes.
  - webContents: navigation lifecycle, `willAskForPermission:withRequests:…completion:`, JS
    alert/confirm/prompt/beforeunload panels, `willShowContextMenu:withElement:`,
    `showHungRendererDialog:`, `showExternalProtocolDialog:`, `showWebLoginDialog:` (HTTP auth),
    translate, PiP, media session, audible/mute/camera/mic state, `didSubmitCredentials:` /
    `didSubmitCreditCardInfo:` (save prompts drawn by Swift), `didSampleTopBandColor:` (page colour for
    theming), overscroll/swipe events, `didCreateNewWebContent:targetURL:disposition:` (popups → Peek).
  - Every UI decision Chromium needs comes back through a `completion:` callback.
- **Run loop:** `NSPrincipalClass` is `Arc.BrowserApp`, a Swift `NSApplication` subclass implementing
  Chromium's `CrAppProtocol`/`CrAppControlProtocol` (`isHandlingSendEvent`, `setHandlingSendEvent:`).
  Chromium's main-thread pump is a CFRunLoop source on the same AppKit loop the Swift UI uses. There is
  one loop, and the UI owns `NSApp`.

## baa's version of it

```
baa.app/Contents/
  MacOS/baa                           Rust: GPUI shell (baa-ui, baa-model, baa-privacy, baa-web)
  Frameworks/BaaCore.framework        GN-built: //chrome + //baa, exports a C ABI
    Helpers/baa Helper (Renderer|GPU|Plugin|Alerts).app, app_mode_loader
```

| Arc | baa | Change and why |
|---|---|---|
| `//arc` in Chromium tree | `//baa` (repo `chromium/baa`, symlinked into `src/baa`) | same |
| `arc::` C++ layer, `shared_ptr<Impl>` handles, `…Delegate` interfaces | `baa::` C++ layer, same shape | same |
| `arc::browser::ui::Window : BrowserWindow`, `LocationBar` | `baa::ui::Window`, `baa::ui::LocationBar` | same; one patch makes `BrowserWindow::CreateBrowserWindow` build ours |
| ObjC wrappers + `DelegateBridge` for Swift | **C ABI** (`baa.h`: opaque handles, `extern "C"` calls, one struct of callback fn pointers per delegate) | Rust has no ObjC/C++ interop worth using; a C ABI is the stable boundary. Rust side hand-written in `baa-web`, not bindgen. |
| `completion:` blocks | callback + `BaaCompletion*` token Rust resolves later | same semantics, C-friendly |
| Swift `NSApplication` + `CrAppProtocol` | GPUI's NSApp + `CrAppProtocol` (exists: `vendor/gpui-cef/.../nsapp.rs`) | already proven with CEF |
| Swift views own pages' NSViews | GPUI page element places `WebContents`' NSView under GPUI's layer, punches the hole (exists) | same as today |
| `ArcBrowserContext` per Arc profile | `BaaProfile` per storage profile (Chrome `Profile` via `ProfileManager`) | same as today's model |
| Swift answers `chrome.tabGroups` | baa-model answers it: folders are tab groups | baa-model stays the source of truth for Spaces/pinned/folders |
| `ArcAutocompleteMatch` (Chrome's omnibox engine) | command bar uses Chrome's `AutocompleteController` | better ranking than our own history search, free |
| ObjC data classes | `#[repr(C)]` structs + borrowed UTF-8 strings | — |

**Threading:** every call and callback runs on the main thread (Chromium's UI thread on macOS).
**Start-up:** GPUI starts the app and owns `[NSApp run]`. In `applicationDidFinishLaunching`, Rust calls
`baa_application_start(argv, &delegate)`, which runs `ContentMainRunner` initialisation and
`BrowserMainRunner::Initialize` without calling its `Run`. Chromium's tasks then fire from the shared
run loop. Arc's exact entry function is not visible in the binary, so phase 2 confirms this; CEF's
`CefInitialize` already does the same thing inside baa today.

### Who draws what

Like Arc, everything with a delegate callback is drawn by GPUI: permissions, JS dialogs, context menus
(Chrome's `RenderViewContextMenu` model → GPUI or native NSMenu), find bar, downloads, extension
toolbar actions/badges, extension install and permission prompts, hung-renderer and external-protocol
dialogs, HTTP auth, password/card save prompts, translate, PiP controls, DevTools docking.
Chromium-drawn (minor re-skin patches allowed): extension popups' contents (a `WebContents` in a GPUI
popover), chrome:// WebUI pages, print preview, autofill dropdown at first (GPUI later via
`AutofillPopupView`).

## Source tree and patches

```
chromium/
  helium-macos.rev        pinned imputnet/helium-macos commit (it pins helium-chromium + Chromium version)
  patches/                baa's own patches (BrowserWindow factory, dialog skins), applied after Helium's
  baa/                    //baa: BUILD.gn, baa.h, baa::* layer, baa::ui::Window
```

Helium's own scripts do the work (`devutils/shared.sh`: `prepare_sources` downloads Helium's Chromium
tarball, prunes, applies patches, domain/name substitution; `write_gn_args <arch> dev|release`).
`chromium.yml` runs them; we add our patch folder and `//baa` on top.

- **Patch base: Helium (GPL-3.0, fine for baa).** Take its de-Googling, privacy and extension patches.
  Skip its views-UI patches, which change code we never create. Helium tracks ungoogled-chromium, so
  ungoogled's BSD patches come with it.
- Checkout (~100 GB) lives outside the repo at `~/chromium/src`.
- With CEF gone, runtime stripping (`baa-privacy` switches and blocklist) shrinks to whatever Helium
  doesn't already patch out. The CI proxy test stays.

## Build and CI

- **`chromium.yml`** builds the pinned Helium tree in self-chaining ~4.75 h phases on free runners.
  `mode=release` = Helium's official args (the timing run for the runner decision). `mode=dev` = component
  build on the `xcode-27` runner with Xcode 27.1 (27A9275, same as the dev Mac); the finished tree is kept
  7 days to unpack locally (`~/chromium`) and build incrementally (`//baa` edits rebuild in minutes).
- **Source reference:** Helium's CI publishes its prepared tree (`helium_macos_resources_arm64`); unpacked
  at `~/chromium/helium-ref` for reading/grepping while builds run.
- **Faster later:** Helium uses `sccache` in CI and NativeLink remote execution for dev; add a cache
  backend or a self-hosted runner if chained phases are too slow.
- **Engine bumps:** `helium-bump.yml` (to write) moves `chromium/helium-macos.rev` to Helium's next
  release, rebuilds and opens a PR; a failing baa patch stops for a human. The privacy test gates it.
  `cef-bump.yml` is gone; the app keeps its pinned CEF until phase 5.
- **Cost to accept:** security-fix latency = Helium's lag + our patch refresh + build time. With CEF it is
  Spotify's problem.

## Phases

| # | Phase | Done when |
|---|---|---|
| 0 | **Measure** | `chromium.yml` release run timed on free runners; dev tree downloaded and an incremental build confirmed locally. Runner decision. |
| 1 | **Tree tooling** | `chromium/patches` + `//baa` applied by `chromium.yml` and by a local script on the unpacked tree. |
| 2 | **Spike (shape check)** | `BaaCore.framework` with a minimal `//baa`; Rust `main` (GPUI, `CrAppProtocol` NSApp) starts it; one `WebContents` NSView in a GPUI hole; 120 Hz scroll, typing, IME, ⌘ keys to GPUI. |
| 3 | **ADK core** | `baa::BrowserApplication/Profile/Browser/WebContents` + `baa::ui::Window`; C ABI + Rust wrapper; Spaces as profiles; tabs in a real `TabStripModel`; 1Password signs in and sees tabs with no shim; Chrome-made tabs and `window.open` arrive via `didCreateNewWebContents`. |
| 4 | **Delegate surface** | The Arc delegate list above, drawn in GPUI; extension toolbar actions + popups; `chrome.tabGroups` ↔ folders; command bar on `AutocompleteController`. |
| 5 | **Parity + delete CEF** | `baa-web` backend on BaaCore behind `WebHost`; every CEF-build feature works. Remove `vendor/gpui-cef`, tabs shim, `ChromeTabClient`, hidden tab windows, Origin-null fix, hidden chrome://extensions page, ⌘C routing. |
| 6 | **Ship** | Signed + notarised from CI; `helium-bump.yml` live; privacy test green. |
| — | Windows | Later: same `//baa`, HWND instead of NSView (Arc did this, Swift on Windows). CEF/WebView2 keeps Windows alive until then. |

## Open questions

1. Release builds: free chained runners, paid runners, or this Mac self-hosted? Decide after phase 0.
2. Proprietary codecs (H.264/AAC) in our build?
3. Widevine needs Google's VMP signing agreement (Arc has one). Out of scope unless pursued.

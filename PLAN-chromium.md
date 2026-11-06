# baa — own Chromium build + GPUI shell, ADK-style (draft, 2026-10-31)

Replaced PLAN.md §1, §2 (Engine host, De-Googling, Extensions rows), §5, §7 and the CEF parts of §13;
done 2026-11-06 (PLAN.md now points here). baa is open source under GPL-3.0 (`GPL-3.0-only`).

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
| `//arc` in Chromium tree | `//baa` (repo `chromium/baa`, copied into `src/baa` last by `prepare_sources`; reached from `//chrome:dependencies` by `patches/baa/add-baa-layer.patch`) | same |
| `arc::` C++ layer, `shared_ptr<Impl>` handles, `…Delegate` interfaces | `baa::` C++ layer, same shape | same |
| `arc::browser::ui::Window : BrowserWindow`, `LocationBar` | `baa::ui::Window`, `baa::ui::LocationBar` | same; one patch makes `BrowserWindow::CreateBrowserWindow` build ours |
| ObjC wrappers + `DelegateBridge` for Swift | **C ABI** (`baa.h`: opaque handles, `extern "C"` calls, one struct of callback fn pointers per delegate) | Rust has no ObjC/C++ interop worth using; a C ABI is the stable boundary. Rust side hand-written in `baa-web`, not bindgen. |
| `completion:` blocks | callback + `BaaCompletion*` token Rust resolves later | same semantics, C-friendly |
| Swift `NSApplication` + `CrAppProtocol` | GPUI's NSApp + `CrAppProtocol` (proven in the phase 2 spike) | already proven with CEF |
| Swift views own pages' NSViews | GPUI page element places `WebContents`' NSView under GPUI's layer, punches the hole (exists) | same as today |
| `ArcBrowserContext` per Arc profile | `BaaProfile` per storage profile (Chrome `Profile` via `ProfileManager`) | same as today's model |
| Swift answers `chrome.tabGroups` | baa-model answers it: folders are tab groups | baa-model stays the source of truth for Spaces/pinned/folders |
| `ArcAutocompleteMatch` (Chrome's omnibox engine) | command bar uses Chrome's `AutocompleteController` | better ranking than our own history search, free |
| ObjC data classes | `#[repr(C)]` structs + borrowed UTF-8 strings | — |

**Threading:** every call and callback runs on the main thread (Chromium's UI thread on macOS).
**Start-up (revised 2026-11-06, see `PLAN-chromium-phase2.md`):** Chromium owns `[NSApp run]`, as in
Arc (`-[ArcBrowserApplication run]` returns the exit code) and in Chrome. NSApp is `BaaApplication :
BrowserCrApplication` from `//baa`, created before gpui. gpui's `[NSApp run]` lands in its `-run`
override, which calls `ChromeMain`. Chromium's `RunLoop::Run` then calls `[NSApp run]` again, which goes
to `[super run]`, and gpui's `didFinishLaunching` fires from there. "Initialize without Run" does not
work on Mac: the pump runs tasks only while `RunLoop::Run` is on the stack
(`base/message_loop/message_pump_apple.mm`).

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
  helium/                 vendored imputnet/helium (git subtree --squash): patches, utils, flags, resources
  helium-macos/           vendored imputnet/helium-macos: macOS patches, flags, build scripts
  baa/                    //baa (to write): BUILD.gn, baa.h, baa::* layer, baa::ui::Window
```

We own the vendored copies and edit them directly. baa's changes so far (2026-10-31):
- **Branding:** `BRANDING` → baa / `me.baa.browser` / no team ID (`helium/core/change-chromium-branding.patch`);
  name substitution rewrites Chrome/Chromium/Helium → baa and `chrome://`/`helium://` → `baa://` in
  user-visible strings, except "Helium services";
  `kHeliumUIScheme` = "baa" (`override-chrome-protocol.patch`); data folder `me.baa.browser`, Keychain item
  "baa Storage Key" (macOS patches). Helium's logo and app icon are not applied: Chromium's stock art until
  baa has its own.
- **Dropped:** 9 `helium/ui/*` patches that only touch views code and that nothing else depends on. The
  other 95 UI patches are tangled with prefs, strings and the WebUI pages baa still shows, so they stay.
- **Dropped too:** `helium/ui/crash-dialog.patch` (offers to send a report; nothing is sent).
- **baa's own patches** live in `chromium/patches/` (applied after both Helium series, by
  `prepare_sources`), so Helium merges never touch them:
  - `baa/remove-helium-services.patch`: Helium's servers are never contacted. `ShouldAccessServices` is
    false and the default origin is the dummy host, so every service takes its built-in "off" path
    (component updates, spellcheck dictionaries, bangs list, uBO asset config, browser updates);
    extension installs and updates go straight to the Chrome Web Store, as in stock Chromium; the
    settings entry and the services onboarding are gone. When baa runs its own services, point
    `kHeliumDefaultOrigin`/`ShouldAccessServices` in `components/helium_services` there.
  - `baa/no-crash-uploads.patch`: no upload URL and uploads forced off; crashpad keeps reports in
    `<user data>/Crashpad/`. The crash-reporting toggle in settings now does nothing (still to remove).

Taking a Helium release: `git subtree pull --prefix chromium/helium https://github.com/imputnet/helium.git
<commit> --squash`, same for `helium-macos` (then `git rm` its `helium-chromium` gitlink and `.gitmodules`
again), resolve conflicts with the edits above, and validate: `helium/devutils/validate_patches.py -l
<unmodified tree>` over both series (remote mode `-r` is broken for sub-DEPS).

- **Patch base: Helium (GPL-3.0, fine for baa).** Take its de-Googling, privacy and extension patches.
  Skip its views-UI patches, which change code we never create. Helium tracks ungoogled-chromium, so
  ungoogled's BSD patches come with it.
- Checkout (~100 GB) lives outside the repo at `~/chromium/src`.
- With CEF gone, runtime stripping shrinks to what is left in `baa-privacy`: `CORE_SWITCHES`,
  `CORE_DISABLED_FEATURES`, `PREFS_OFF` and `PROXY_ALLOWLIST`. The host blocklist and the request filter
  are gone. The CI proxy test stays.

## Build and CI

- **`chromium.yml`** builds the pinned Helium tree in one run of chained ~4.75 h phases on free runners, like Helium's `building.yml`.
  `mode=release` = Helium's official args (the timing run for the runner decision); it builds
  `chrome chrome/installer/mac`, uploads `baa-engine-<T>` (`<T>` = `git rev-parse HEAD:chromium`;
  `baa.app`, `baa Packaging`, `engine.json`; 90 days) and runs the `privacy` job on it. It starts by
  hand or from `helium-bump.yml`, not on every push (decided 2026-11-06). `mode=dev` = component
  build on the `xcode-27` runner with Xcode 27.2 (same as the dev Mac) at `/Users/Shared/hb`, a path that exists on both, since siso keys its records by absolute path; the finished tree is kept
  7 days to unpack locally (at `/Users/Shared/hb`) and build incrementally (`//baa` edits rebuild in minutes).
- **Local:** `chromium/local-build.sh` syncs the checkout's `chromium/` into `/Users/Shared/hb` by content
  (unchanged files keep their mtimes) and builds `chrome`. Only `//baa` changed: copied straight in. Any
  Helium/patch change: a fresh `prepare_sources` in `/Users/Shared/hb-new` (tarball cached in
  `/Users/Shared/hb-downloads`), synced over the tree like CI does.
- **Source reference:** Helium's CI publishes its prepared tree (`helium_macos_resources_arm64`); unpacked
  at `~/chromium/helium-ref` for reading/grepping while builds run.
- **Faster later:** Helium uses `sccache` in CI and NativeLink remote execution for dev; add a cache
  backend or a self-hosted runner if chained phases are too slow.
- **Engine bumps:** `helium-bump.yml` (weekly) subtree-pulls Helium's next release onto a bot branch,
  opens a PR and starts `chromium.yml` (release) and `ci.yml` on it; a failing baa patch stops for a
  human. The privacy test gates it. No auto-merge.
- **Release:** `release.yml` on a `vX.Y.Z` tag uses exactly the `baa-engine-<T>` of the tagged
  commit's `chromium/` tree (equal tree hashes mean the same `baa.h` ABI and patches), adds the Rust
  binary, signs, notarises and publishes. No such artifact: the run fails and names the command.
- **Cost to accept:** security-fix latency = Helium's lag + our patch refresh + build time. Before the move it
  was Spotify's problem.

## Phases

| # | Phase | Done when |
|---|---|---|
| 0 | **Measure** | `chromium.yml` release run timed on free runners; dev tree downloaded and an incremental build confirmed locally. Runner decision. |
| 1 | **Tree tooling** | `chromium/patches` + `//baa` applied by `chromium.yml` and by a local script on the unpacked tree. |
| 2 | **Spike (shape check)** | `BaaCore.framework` with a minimal `//baa`; Rust `main` (GPUI, `CrAppProtocol` NSApp) starts it; one `WebContents` NSView in a GPUI hole; 120 Hz scroll, typing, IME, ⌘ keys to GPUI. Plan: `PLAN-chromium-phase2.md`. |
| 3 | **ADK core** | `baa::BrowserApplication/Profile/Browser/WebContents` + `baa::ui::Window`; C ABI + Rust wrapper; Spaces as profiles; tabs in a real `TabStripModel`; 1Password signs in and sees tabs with no shim; Chrome-made tabs and `window.open` arrive via `didCreateNewWebContents`. |
| 4 | **Delegate surface** | The Arc delegate list above, drawn in GPUI; extension toolbar actions + popups; `chrome.tabGroups` ↔ folders; command bar on `AutocompleteController`. M3 done 2026-11-06 (`PLAN-m3.md`, ABI 6); deferred to M4: `chrome.commands`, `chrome.action.openPopup`, badges/dynamic icons, screen-share picker, dangerous-download keep/discard. |
| 5 | **Parity + delete CEF** | Done 2026-11-06 (`PLAN-chromium-phase5.md`): `baa-web` backend on BaaCore behind `WebHost`; `vendor/gpui-cef`, the tabs shim, `ChromeTabClient`, hidden tab windows, Origin-null fix, hidden chrome://extensions page and ⌘C routing are deleted. The §5.0 parity checklist runs in CI later. |
| 6 | **Ship** | Done 2026-11-06 as workflows (`PLAN-chromium-phase5.md` §6): `release.yml`, `helium-bump.yml`, the `privacy` job. The first release chain, tag and notarised DMG are still to run. |
| — | Windows | WebView2 was removed in phase 5 (tag `cef-final`); Windows returns with the `//baa` HWND port. |

## Open questions

1. Release builds: free chained runners, paid runners, or this Mac self-hosted? Answered by the first measured release chain (record the hours here).
2. Proprietary codecs (H.264/AAC) in our build? Kept, as Helium ships them (`PLAN-chromium-phase5.md` open question 2).
3. Widevine needs Google's VMP signing agreement (Arc has one). Out of scope unless pursued.

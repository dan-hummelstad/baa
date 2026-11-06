# Phase 2: spike (shape check)

Status: done 2026-11-06; the CEF deletions in "Later deletions" happened in phase 5 (`PLAN-chromium-phase5.md`).

Plan for phase 2 of `PLAN-chromium.md`, written 2026-11-06 from Chromium 156 (Helium tree at
`/Users/Shared/hb/helium-macos/build/src`), gpui at the pinned Zed rev, and `/Applications/Arc.app`.
Paths starting with `chrome/`, `content/` or `base/` are in the Chromium tree. `gpui_macos/` is
`~/.cargo/git/checkouts/zed-*/ef07591/crates/gpui_macos/`.

**Done when:** the framework has a minimal `//baa`; a Rust `main` (GPUI, with a `CrAppProtocol`
NSApplication) starts it; one `WebContents` NSView sits in a GPUI hole; 120 Hz scroll, typing, IME
and ⌘ keys work, and the ⌘ keys go to GPUI.

## Summary of decisions

| # | Question | Decision |
|---|---|---|
| 1 | Framework | Reuse Chrome's `chrome_framework` target, renamed to `BaaCore.framework` (`patches/baa/rename-framework-baacore.patch`; user decision 2026-11-06). `//baa` is already linked into it by phase 1. No content_shell. |
| 2 | How much Chrome | Full `//chrome` startup with a real `Profile`. The spike uses a bare `WebContents` with our own `WebContentsDelegate`. There is no `Browser` and no `BrowserWindow` yet. |
| 3 | Event loop | **Chromium calls `[NSApp run]`, not GPUI.** NSApp is `BaaApplication : BrowserCrApplication` from `//baa`. Its `-run` starts `ChromeMain` on the first call and calls `[super run]` when Chromium's pump calls it again. This corrects the start-up paragraph in `PLAN-chromium.md` (see "Why not Initialize without Run"). |
| 4 | View | Reuse the CEF windowed design: the `WebContents` NSView goes in a clipping container under GPUI's view, and GPUI punches a hole. The code is copied from `vendor/gpui-cef/.../native.rs`. |
| 5 | Input | GPUI's view keeps the mouse and forwards it to the page (existing code). Keyboard and IME go natively to `RenderWidgetHostViewCocoa`. ⌘ keys come back through `WebContentsDelegate::PreHandleKeyboardEvent` / `HandleKeyboardEvent` to a Rust callback that offers them to GPUI first, the same way CEF's `on_pre_key_event` does today. |
| 6 | C ABI and Rust | Seven `extern "C"` functions. A new crate `crates/baa-chromium` (owned by the baa-web orchestrator) calls `dlopen` on the framework at startup. The bundle is a copy of the GN `baa.app` inside `out/Default`, with our Rust binary in place of `MacOS/baa`, signed with the local Apple Development identity. |

## Evidence

### Who runs the loop

- `MessagePumpCFRunLoopBase::Run` (`base/message_loop/message_pump_apple.mm:95`) sets the pump's
  delegate and clears it again on return. With no delegate, the work source does nothing
  (`:412`, "can be reached with a nullptr delegate_ if Run is not on the stack"). So on Mac, no
  Chromium task runs unless a `RunLoop::Run` is on the stack. `Attach()`, which lets UIKit own the
  loop, exists only on iOS (`message_pump_apple.h:84`).
- `MessagePumpNSApplication::DoRun` (`:775`) calls `[NSApp run]` itself if `NSApp.running` is NO.
  Otherwise it spins its own nested `nextEventMatchingMask` loop.
- On Mac, `RunContentProcess` (`content/app/content_main.cc:189`) always calls `Run()` and then
  `Shutdown()`. `BrowserMain` (`content/browser/browser_main.cc`) does
  `Initialize → Run → Shutdown`. Chrome's normal start is therefore `ChromeMain → ContentMain →
  BrowserMain → RunLoop::Run → [NSApp run]`, and AppKit's `didFinishLaunching` fires from inside
  that call.
- Arc does the same. The `Arc` exe imports no C entry from ArcCore and does not call
  `NSApplicationMain`. `-[ArcBrowserApplication run]` returns an `NSInteger` exit code, and the
  configuration has a `quitRunLoopOnTermination` flag. ArcCore owns `[NSApp run]`, and Swift only
  supplies the NSApp class (`Arc.BrowserApp`, which conforms to `CrAppProtocol`).

#### Why not "Initialize without Run" (the current `PLAN-chromium.md` text)

That design would need a `base` patch that ports iOS's `Attach()` to `MessagePumpNSApplication`,
plus a split of `BrowserMainRunner` that no Chrome code path tests. CEF avoids the problem by
polling (`CefDoMessageLoopWork` on a timer, see `vendor/gpui-cef/.../pump.rs`). That polling is
what we want to get rid of.

### What `//chrome` expects of NSApp

- `ChromeMainDelegate` calls `RegisterBrowserCrApp()` (`chrome/app/chrome_main_delegate.cc:1939`).
  Then `+[BrowserCrApplication sharedApplication]` CHECKs `[NSApp isKindOfClass:BrowserCrApplication]`
  (`chrome/browser/chrome_browser_application_mac.mm:170-184`).
- content CHECKs that NSApp conforms to `NativeEventProcessor`
  (`content/browser/scheduler/responsiveness/native_event_observer_mac.mm:14`).
  `BrowserCrApplication` implements it.
- `-[BrowserCrApplication sendEvent:]` (`:328`) does the CrAppProtocol nesting flag, the crash keys,
  the Eisu/Kana IME fix and the ⌘-keyUp fix. `-terminate:` (`:318`) sends the quit to
  `AppController`.
- gpui's `GPUIApplication` is `NSApplication` plus one ivar, `void* platform`
  (`gpui_macos/src/platform.rs:67,73-77`). gpui gets NSApp with `[GPUIApplication sharedApplication]`,
  which returns any existing NSApp, and it only reads and writes that ivar.

So `//baa` defines `@interface BaaApplication : BrowserCrApplication { void* platform; }` and
creates NSApp before gpui does. Chrome gets a real `BrowserCrApplication`, and gpui finds its ivar.
The CEF retrofit (`nsapp.rs`, which adds CrAppProtocol to `GPUIApplication` at runtime) is not
needed.

### The app delegate

- `ChromeBrowserMainPartsMac::PreCreateThreads` CHECKs `NSApp.delegate == nil`
  (`chrome/browser/chrome_browser_main_mac.mm:160`). It then calls `AppController.sharedController`,
  which sets itself as the delegate and CHECKs on every later call that it still is
  (`chrome/browser/app_controller_mac.mm:799-809`, called from 14 files).
- gpui sets its own delegate in `MacPlatform::run` before `[app run]`
  (`gpui_macos/src/platform.rs:503-506`). gpui's menu items and `didFinishLaunching` closure depend
  on that delegate.
- Decision: a small patch. `sharedController` takes the delegate only if none is set, and the
  `CHECK_EQ(nil, NSApp.delegate)` goes. `AppController` still exists (keep-alive, menu helpers,
  `tryToTerminateApplication`) but does not get NSApplicationDelegate calls. Phase 3 forwards the
  ones it needs (open URLs, reopen, should-terminate) from gpui's delegate. Arc does the same
  thing: Swift's `AppDelegate` receives `applicationShouldTerminate:` and calls
  `-[ArcBrowserApplication tryToTerminate:]`.

### Framework and helpers

- `chrome_framework` (`chrome/BUILD.gn:1383`) already is "Chromium + helpers in one framework":
  `Helpers/`, `Libraries/`, `Resources/` and the versioned binary that exports `ChromeMain`. Helpers
  `dlopen` `../../../../baa Framework` and call `ChromeMain` (`chrome/app/chrome_exe_main_mac.cc`).
  Arc's helpers do exactly this with `../../../../ArcCore`. The helpers do not care what the main
  executable is.
- Phase 1's `chromium/patches/baa/add-baa-layer.patch` adds `//baa` to `chrome/BUILD.gn`'s
  `group("dependencies")`, which `chrome_dll` links. So `baa_*` symbols land in the framework with
  no further GN work. Non-component builds limit exports with `chrome/app/framework.exports`, so add
  `_baa_*` there now.
- In a component build, the framework re-exports `libchrome_dll.dylib` and finds it through
  `-rpath @loader_path/../../../../../..` (`chrome/BUILD.gn:1449-1458`). That rpath is `out/Default`.
  **A spike bundle must sit directly in `out/Default`.**
- content_shell is no use as a base. It proves only what CEF already proves (an NSView in a
  foreign window, `shell_platform_delegate_mac.mm:226`), and it does not exercise any of the
  `//chrome` conflicts above, which are what this spike must flush out.

### Keys

- With the page's view as first responder in the key window,
  `-[RenderWidgetHostViewCocoa performKeyEquivalent:]` takes every ⌘ key unless the system reserves
  it, and sends it to the renderer (`content/app_shim_remote_cocoa/render_widget_host_view_cocoa.mm:1354-1405`).
  content asks `WebContentsDelegate::PreHandleKeyboardEvent` first and calls
  `HandleKeyboardEvent` if the page does not consume the key
  (`content/public/browser/web_contents_delegate.h:343,351`). Both get
  `input::NativeWebKeyboardEvent`, whose `os_event` is the NSEvent.
- This is the same position CEF's `on_pre_key_event` is in today. Phase 3 changes only the caller:
  `Browser` becomes the delegate and forwards to `baa::ui::Window::PreHandleKeyboardEvent`, which
  Arc also implements (`arc::browser::ui::Window::PreHandleKeyboardEvent`).

### Allocator

`chrome_exe_main_mac.cc` calls `partition_alloc::EarlyMallocZoneRegistration()` first. Without it,
PartitionAlloc takes a late path that "is fine if the current process is not hosting multiple
threads" (`allocator_shim_override_apple_default_zone.h`, near the end of the zone setup). So Rust
`main` calls `dlopen` on the framework before anything starts a thread (gpui's executors included).

## Start-up and shutdown sequence

```
Rust main
  dlopen("../Frameworks/baa Framework.framework/baa Framework")    single-threaded, first thing
  baa_install_app(argc, argv, &callbacks)   NSApp = [BaaApplication sharedApplication]; keeps argv
  gpui Application::new().run(closure)
    MacPlatform::run: setDelegate(gpui delegate); [NSApp run]
      -[BaaApplication run] #1  -> ChromeMain(argc, argv)          (in-process, browser type)
        ContentMain -> BrowserMain -> PreMainMessageLoopRun (profile loaded; --no-startup-window)
        RunLoop::Run -> MessagePumpNSApplication::DoRun -> [NSApp run]
          -[BaaApplication run] #2 -> [super run]                    the one real AppKit loop
            didFinishLaunching -> gpui delegate -> closure: open window, baa_web_contents_create
            ... app runs; Chromium tasks fire from the pump's CFRunLoop sources ...
⌘Q -> gpui quit -> [NSApp terminate:] -> BrowserCrApplication terminate:
   -> AppController tryToTerminateApplication -> browser shutdown -> RunLoop quit -> [NSApp stop:]
          [super run] returns
        BrowserMainRunner::Shutdown, ContentMainRunner::Shutdown; ChromeMain returns code
      -[BaaApplication run] #1 stores the code and returns
    gpui's run returns
  std::process::exit(baa_exit_code())
```

- Re-entrancy: Chromium never calls Rust from inside a gpui borrow, except for callbacks triggered
  synchronously by a Rust → C call. Rule: callbacks only queue work (`cx.spawn` or a channel). They
  never update gpui state inline. The key callback is the one exception, and it calls only
  `[gpuiView performKeyEquivalent:]`, which is what CEF does today.
- gpui's `on_app_quit` and `applicationWillTerminate` do not fire, because `terminate:` is
  overridden. That is accepted for the spike. In phase 3, `baa_install_app` callbacks get
  `will_quit`.

## C ABI (spike)

`chromium/baa/baa.h`, all main-thread:

```c
typedef struct BaaWebContents BaaWebContents;
// after_page = false: before the page sees it; true: the page did not consume it.
typedef bool (*BaaKeyFn)(void* ctx, BaaWebContents*, void* ns_event, bool after_page);

void baa_install_app(int argc, const char** argv, BaaKeyFn key, void* key_ctx);
int  baa_exit_code(void);
BaaWebContents* baa_web_contents_create(const char* url);   // last-used profile
void* baa_web_contents_view(BaaWebContents*);                // NSView*, owned by the WebContents
void baa_web_contents_focus(BaaWebContents*);
void baa_web_contents_edit(BaaWebContents*, int command);    // copy/cut/paste/select-all/undo/redo
void baa_web_contents_destroy(BaaWebContents*);
```

There is no resize call. The view uses autoresizing, and `WebContentsViewCocoa` passes frame
changes to the `RenderWidgetHostView`, the same as content_shell. There is no navigation API
beyond the initial URL, because the spike does not need one. Phase 3 replaces this with the
`baa::Browser`-shaped API.

## Steps

Each step ends in something runnable. Do not run `gn gen` or `autoninja` while another siso build is
running (`pgrep -x siso`).

### 1. `//baa`: BaaApplication + start (Chromium side)

Files (in `chromium/baa/`, synced into `src/baa` by phase 1's tooling):
- `BUILD.gn`: add `app_mac.mm`, `web_contents.mm` and `baa.h` to phase 1's target. Add deps
  `//chrome/browser`, `//content/public/browser` and `//base`. If GN reports a visibility error,
  extend that target's `visibility` in the patch below.
- `app_mac.mm`: `BaaApplication : BrowserCrApplication` with the `void* platform` ivar. Its `-run`
  override: on the first call, call `ChromeMain(argc, argv)`, store the result and return. On later
  calls, `[super run]`. Also `baa_install_app` and `baa_exit_code`.
- `web_contents.mm`: `BaaWebContents` holds a `std::unique_ptr<content::WebContents>` and is its
  `WebContentsDelegate`. `PreHandleKeyboardEvent` and `HandleKeyboardEvent` call the `BaaKeyFn` with
  `event.os_event.Get()`, but only for RawKeyDown with ⌘/⌃ or Escape (the same filter as CEF's
  `on_pre_key_event`). Create with
  `WebContents::Create(CreateParams(ProfileManager::GetLastUsedProfile()))`,
  then `GetController().LoadURL`. `edit` calls `WebContents::Copy/Cut/Paste/SelectAll/Undo/Redo`.

Patch `chromium/patches/baa/embed-in-gpui.patch` (add it to `series` after `add-baa-layer.patch`):
- `chrome/app/framework.exports`: `+_baa_*`.
- `chrome/browser/app_controller_mac.mm` `+sharedController`: set `NSApp.delegate` only if it is nil.
  Drop `CHECK_EQ(NSApp.delegate, sharedController)`.
- `chrome/browser/chrome_browser_main_mac.mm` `PreCreateThreads`: drop `CHECK_EQ(nil, NSApp.delegate)`.

Build: `autoninja -C out/Default chrome`.
Test: `nm -gU "out/Default/baa Framework.framework/baa Framework"` lists the seven `_baa_` symbols.
In a component build they live in `libchrome_dll.dylib`, so run `nm -gU` there too. The stock
`out/Default/baa.app` still starts and opens a window, which shows the patch did not break Chrome.

### 2. `crates/baa-chromium`: load, install, run an empty gpui window

Files:
- `crates/baa-chromium/Cargo.toml`: deps `gpui`, `gpui_platform`, `objc2`, `core-video`,
  `core-foundation` and `raw-window-handle`, at the versions `vendor/gpui-cef` already uses.
  `dlopen` comes from `libc`, which is already in the lock file through gpui. Nothing new.
- `src/lib.rs`: `Core::load()` calls `dlopen` and `dlsym` for each `baa_*`, and keeps the function
  pointers in a struct. This is the whole FFI: no bindgen and no sys crate.
- `src/bin/spike.rs`: `main` as in the start-up sequence. It adds `--no-first-run`,
  `--no-startup-window` and `--user-data-dir=$TMPDIR/baa-spike` to argv.
- `spike.sh`: `cargo build -p baa-chromium --bin spike`; `ditto out/Default/baa.app
  "out/Default/baa Spike.app"` (only when the GN app is newer); copy the binary over
  `Contents/MacOS/baa`; `codesign --force --sign - "out/Default/baa Spike.app"`; then
  `open "out/Default/baa Spike.app" --args "$@"`. The bundle must stay in `out/Default` (rpath).
- Root `Cargo.toml`: add the workspace member. `CLAUDE.md`: add an ownership line for
  `crates/baa-chromium` (baa-web orchestrator; it becomes baa-web's backend in phase 5).

Test (risks 1–3): `./spike.sh` shows an empty gpui window. The log shows Chromium's browser process
started (`--enable-logging=stderr`; look for the profile path). `ps` shows the GPU helper and the
network service. Run `pgrep -fl "baa Spike"` for 2 minutes: nothing crashes, and
`$TMPDIR/baa-spike/Crashpad/` gets no new report. ⌘Q exits with code 0, and the log shows
`BrowserMainLoop::ShutdownThreadsAndCleanUp`.

### 3. Page in a GPUI hole

Files:
- `crates/baa-chromium/src/native.rs`: copy these from `vendor/gpui-cef/.../native.rs`:
  `Container`, `punch_buffer`, `install_mouse_forwarding`, `forward_current_event`,
  `offer_key_equivalent`, `make_first_responder`, `has_first_responder` and `gpui_view`. Mark the
  copy with `// ponytail: copied from gpui-cef; gpui-cef is deleted in phase 5, then this is the
  only copy`.
- `src/page.rs`: a gpui `Element` that, on paint, calls `container.place(view, bounds)` and
  `window.paint_surface(bounds, punch_buffer(..))`, and forwards mouse events, the way
  `vendor/gpui-cef/.../mod.rs:897-973` does. On mouse down in the page, call
  `baa_web_contents_focus`. On mouse down outside it, make gpui's view first responder.
- The spike window: a 40 px gpui bar with a text input (for the focus and ⌘ tests) above the page.

Test: the page renders under gpui with rounded corners. Live window resize keeps the page filling
the hole, and the page re-lays out within about 40 ms of the last resize (the same throttle as
CEF). Clicking a link navigates.

### 4. Input tests (the "done when" checks)

Each is a manual check on this Mac's ProMotion display, compared against the GN `baa.app` (stock
Chrome UI from the same build) as the reference:

| Criterion | Test | Pass |
|---|---|---|
| 120 Hz scroll | Load a long page with a rAF counter overlay (a `data:` URL that prints frames per second; keep it in `crates/baa-chromium/spike-fps.html`). Trackpad-scroll. Also Quartz Debug's frame meter. | ≥ 110 fps while scrolling, the same as in `baa.app`. |
| Typing | Type into a `<textarea>` with key repeat, arrows, ⌥/⌘-arrows and backspace. | Every key lands. No double keys and no beeps. |
| IME | Japanese Romaji and Pinyin in the textarea: marked text, candidate window, commit, cancel with Esc. | The candidate window sits at the caret, and the commit replaces the marked text. |
| ⌘ keys to GPUI | gpui binds ⌘L (focus the bar) and ⌘T (log "new tab"). Press each with the page focused and with the bar focused. | gpui handles both in both cases. The page never sees them (check with a `keydown` logger on the page). |
| Editing ⌘ keys | ⌘A, ⌘C, ⌘V, ⌘X, ⌘Z and ⇧⌘Z in the textarea while gpui does not bind them. | They work in the page through the `after_page` → `baa_web_contents_edit` fallback. |
| Esc | An overlay test is not needed. Check only that Esc reaches the page (`keydown` logger). | It does. |
| Focus | Click the bar, type, click the page, type. | Keys follow the click, and the caret blinks only in one place. |

### 5. Write down what phase 3 inherits

Update `PLAN-chromium.md`: correct the start-up paragraph, record the measured build times
(step 6) and the results of the risk tests, and list what carries over.

**Carries over to phase 3:** `BaaApplication` and run ownership, the `embed-in-gpui.patch`, the
bundle layout and `spike.sh` (it becomes the dev run script), `Core::load`, `native.rs` and
`page.rs`, and the key callback (its caller moves from our `WebContentsDelegate` to
`baa::ui::Window::PreHandleKeyboardEvent`).

**Replaced in phase 3:** bare `WebContents` → tabs in a `TabStripModel` owned by a `Browser` whose
window is `baa::ui::Window`; `GetLastUsedProfile` → `baa::Profile` per Space; `TabHelpers` attached
the way Chrome does it.

### 6. Build and run loop

Commands (from `/Users/Shared/hb/helium-macos`):

```sh
pgrep -x siso && echo "build running, wait"            # never two builds
source devutils/shared.sh && cd "$_src_dir"
<phase 1 sync script>                                    # //baa + chromium/patches into the tree
SISO_PATH="$_siso_path" python3 "$_depot_tools_dir/autoninja.py" -C "$_out_dir" chrome
cd ~/Documents/Projects/baa/crates/baa-chromium && ./spike.sh
```

Expected incremental times (component build, estimates; measure them in step 1 and record them):
- A `//baa` `.mm` edit: 1 compile plus a relink of `libchrome_dll.dylib`, about 1–3 min.
- A patch to `app_controller_mac.mm` or `chrome_browser_main_mac.mm`: the same.
- A `BUILD.gn` edit: add about 1 min for GN to regenerate. ninja does this itself.
- The Rust side: about 10–30 s (gpui already cached). The `ditto` copy runs only when `baa.app`
  changed.

## Risks and the cheapest early test

| # | Risk | Cheapest test | Fallback |
|---|---|---|---|
| 1 | gpui needs more of `GPUIApplication` than the `platform` ivar (now or after a gpui bump). | Step 2: the empty window comes up. Add a `debug_assert!` in `spike.rs` that NSApp's class has an ivar named `platform`. | Arc's route: keep the `nsapp.rs` retrofit on `GPUIApplication` and patch Chrome's `isKindOfClass` CHECK and `NativeEventProcessor` instead. |
| 2 | `AppController` without the delegate role misses start-up work it does in `applicationDidFinishLaunching:` and crashes later. | Step 2's 2-minute run, then open `chrome://version`, `chrome://settings` and `chrome://extensions` in the spike page. | Forward `applicationWill/DidFinishLaunching:` from `BaaApplication` (`-finishLaunching`) to `AppController`. |
| 3 | Something during start-up spins a nested `RunLoop` before the main one (first-run UI, profile picker, a dialog). That makes `[NSApp run]` the outermost loop early, and gpui's closure runs before Chromium is ready. | Start with a fresh `--user-data-dir` but without `--no-first-run`, and log the order of `didFinishLaunching` and `PreMainMessageLoopRun`. | Keep `--no-first-run`. The `-run` override counts calls and only calls `[super run]` from the main `RunLoop` (check `base::RunLoop::IsNestedOnCurrentThread()`). |
| 4 | PartitionAlloc's late zone setup races with an early thread. | `dlopen` is the first statement in `main`. Grep stderr for "Trying to load the allocator multiple times". Launch 20 times in a loop. | Export a `baa_early_zone()` from a tiny dylib that the exe links, or call `EarlyMallocZoneRegistration` from a C shim before `dlopen`. |
| 5 | Shutdown is unclean: prefs or history are lost, or a hang. | Visit 3 pages, ⌘Q, relaunch with the same `--user-data-dir`, check `chrome://history`. Exit code 0. No Crashpad report. | Call `chrome::AttemptExit()` through `baa_quit()` from gpui's quit path instead of `terminate:`. |
| 6 | 120 Hz fails in a non-Chrome NSWindow. | Step 4's rAF test. | Low risk: CEF windowed mode already gets 120 Hz with this exact layering (`native.rs` header). |
| 7 | GN visibility or `gn check` refuses `//baa` → `//chrome/browser`. | Step 1's first build. | Add `//baa/*` to the `visibility` lists in `embed-in-gpui.patch`. |
| 8 | The spike app's ad-hoc signature, or the copy of a component-build bundle, breaks helper launch. | Step 2: `ps` shows the renderer and GPU helpers. | Run from the GN `baa.app` itself, with `MacOS/baa` swapped in place. |

## Decided (user, 2026-11-06)

1. The framework is `BaaCore.framework` now (rename patch above).
2. Persisting baa-model state from a phase 3 `will_quit` callback is fine.

## Results (2026-11-06)

Implemented on branch `worktree-phase2-plan`: `chromium/baa/{baa.h,baa.cc,baa_mac.mm,BUILD.gn}`,
`chromium/patches/baa/{rename-framework-baacore,embed-in-gpui}.patch`, `crates/baa-chromium`
(`src/{main,ffi,native}.rs`, `spike.sh`, `smoke.sh`, `cdp.mjs`, `spike-test.html`).

Run: `crates/baa-chromium/spike.sh` (builds, refreshes `out/Default/baa Spike.app`, signs, runs in
the foreground). Checks without a human: `BAA_SPIKE_SELFTEST=1 BAA_SPIKE_QUIT_AFTER=14
crates/baa-chromium/smoke.sh "<out>/baa Spike.app/Contents/MacOS/baa" 20`, plus
`--remote-debugging-port=9333` and `node crates/baa-chromium/cdp.mjs 9333 '({value:
document.querySelector("textarea").value, keys})'` for the page's side.

| Criterion | Status | Evidence |
|---|---|---|
| BaaCore.framework with `//baa` | done | `BaaCore.framework` with helpers inside; 8 `_baa_*` exports in `libchrome_dll.dylib`. |
| Rust `main` starts it, CrAppProtocol NSApp | done | Main-thread stack: gpui `run` → `-[BaaApplication run]` → `ChromeMain` → `BrowserMain` → `RunLoop::Run` → `MessagePumpNSApplication::DoRun` → `-[BaaApplication run]` → `-[NSApplication run]`. gpui's `didFinishLaunching` runs after the profile is up (uBlock Origin's background page is running). |
| Quit | done | gpui `cx.quit()` (the ⌘Q path) → Chrome shutdown → `ChromeMain` returns 0 → process exits 0, no crash reports, no DCHECKs. |
| WebContents NSView in a GPUI hole | done | Screenshot: page under gpui's bar with rounded corners; page reports `visible`, 1184×745 = the hole. |
| Typing | done (synthetic NSEvents) | Click into the textarea, type "hello": page gets mousedown/mouseup and every keydown; textarea has the text. |
| ⌘ keys to GPUI | done (synthetic) | With the page focused, ⌘T and ⌘L run gpui actions and never reach the page; after ⌘L, "ab" goes to gpui's bar, not the page. |
| Editing ⌘ keys | done (synthetic) | ⌘A then "x" then ⌘Z leaves "hello": the page saw the keys unhandled, the after-page callback sent select-all / undo. Escape reaches the page. |
| 120 Hz | partly | Page rAF runs at 120 fps on the ProMotion display. **Trackpad scrolling still needs a human**: synthetic scroll events never reach the app without Accessibility permission. The page's HUD shows fps while scrolling; `scrollFps` holds the samples taken while scrolling. |
| IME | **needs a human** | Japanese/Pinyin in the textarea: marked text, candidate window at the caret, commit, Esc. |

Measured build loop: a `//baa` edit rebuilds in 14–21 s (2 steps); a patch change re-preps the tree
(about 5 min) and then builds in about 25 s; the Rust side is 2–5 s.

What changed from the plan while building it:
- `baa_install_app` calls `base::CommandLine::Init` first: `+[BrowserCrApplication initialize]` reads
  the command line before `ChromeMain` runs.
- AppController, when it is not the delegate, observes the will/did-finish-launching and
  will-terminate notifications itself. Without that it never drops its keep-alive and quit hangs.
  Chrome posts `NSApplicationWillTerminateNotification` itself, so gpui's delegate gets
  `applicationWillTerminate:` too.
- Pages are destroyed on `browser_shutdown::AddAppTerminatingCallback`. A page that outlives its
  profile trips `SiteProcessCountTracker`'s DCHECK. Later UI calls on a gone page do nothing.
- The gpui window needs `WindowBackgroundAppearance::Transparent`. An opaque Metal layer draws the
  hole black.
- Signing: ad-hoc signatures change with every build, so the Keychain asked about
  "baa Storage Key" on every launch. The prompt also took key focus away during tests. `spike.sh`
  signs with the first "Apple Development" identity (`BAA_SIGN_IDENTITY` overrides it), so "Always
  Allow" is kept across rebuilds. The stock `out/Default/baa.app` is still ad-hoc (siso owns it) and
  still asks.

Gotchas found:
- `--remote-debugging-port` plus `--no-startup-window` keeps Chrome alive until DevTools sends
  `Browser.close`, so ⌘Q does nothing in such a run (`chrome_devtools_manager_delegate.cc:197`).
- In 1 of 4 synthetic runs, the page stopped acknowledging keys after an event carried "left button
  down" (a real click elsewhere, probably on the Keychain prompt). Re-check by hand: hold the mouse
  button outside the window, release it, then type and use ⌘A in the page.

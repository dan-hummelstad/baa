# baa — plan

An Arc-like browser. Chromium engine, de-Googled, with a GPUI shell written in Rust.
Domain: baa.me. Targets: **macOS + Windows** (committed). iOS/Android later (noted, not committed, see §12).

Researched 2026-10-22. Versions quoted are the ones live that day.

---

## 1. One-paragraph architecture

The whole UI (sidebar, spaces, command bar, peek, animations) is a GPUI app. Every tab is a Chromium
page hosted by CEF (Chromium Embedded Framework) as a **native child view placed under GPUI's layer**
(macOS; decision 2026-10-22, see §2 Engine host). The page element punches a rounded transparent hole in
GPUI's frame and moves the view to its rect, so GPUI still draws chrome, popovers and the command bar over
it, while Chromium renders and scrolls natively at the display's refresh rate.
Each Space owns a CEF **request context** with its own `cache_path`, which is what gives you
per-space cookies / localStorage / IndexedDB / service workers for free. Chromium is never patched or
compiled by us: we take Spotify's official CEF binaries and strip Google at runtime (switches, prefs,
request blocking). A scheduled GitHub Action bumps the CEF pin whenever a new stable appears, with no
code changes required. No baa.me backend exists in v1.

```
┌───────────────────────────── baa (GPUI, Rust) ─────────────────────────────┐
│  sidebar · spaces · command bar · peek · little baa · animations · keymap  │
│                                                                            │
│   Tab = gpui element: rect + hole  ──►  CEF NSView below gpui's layer       │
│   mouse/keys/IME/scroll: native to Chromium  ──►  per-Space RequestContext │
└────────────────────────────────────────────────────────────────────────────┘
         libcef (Spotify binary, pinned in Cargo.toml)   ← GH Action auto-bumps
```

---

## 2. Decisions and why

| Decision | Choice | Rejected | Why |
|---|---|---|---|
| Engine host | **CEF via `cef` crate (tauri-apps/cef-rs)**, Alloy style, **native child view under GPUI's layer** (macOS) | OSR (used until 2026-10-22); wry/WKWebView | OSR caps input-driven frames at 60 Hz: viz's FrameIntervalDecider pins the interval during input to the begin-frame source's minimum, which for a display-less OSR compositor is the 16.67 ms default, and no stock-CEF switch changes it (external BeginFrames avoid it but hit cef#4311). A native view gets Chromium's own display link (120 Hz scroll, momentum, native IME). GPUI keeps drawing over it: the page element punches a hole (vendored `gpui_apple`), GPUI's view passes hit tests through where the page hitbox is not occluded, shortcuts are offered to GPUI first via `OnPreKeyEvent`. Overlays over a page must `.occlude()`. |
| UI | **GPUI from the Zed git repo** (pin a rev, as gpui-cef / gpui-component do) | `gpui` 0.2.2 on crates.io (stale, Oct 2025), gpui-ce 0.3.0 | Zed paused community GPUI publishing in Feb 2026; the crates.io version lags a year. Git pin of mainline keeps Windows (Win32 + DirectX) and macOS (Metal) backends current. gpui-ce is a fallback if mainline breaks us; its API is a superset. |
| Prior art to fork | **bokuweb/gpui-cef** (CEF 151 OSR → GPUI element, IME via `EntityInputHandler`, MIT/Apache) | start from scratch | It already solves OSR→element, resize, IME, cursor forwarding. 3 stars, one author; we vendor it into our workspace and own it. |
| Frame path | Native: Chromium presents its own layer; nothing is copied. (OSR's CPU-copy and IOSurface paths are gone on macOS.) | — | Windows still uses WebView2 child windows. |
| De-Googling | **Runtime stripping** of stock CEF (§5) | Building CEF against ungoogled-chromium / Helium patches | Building Chromium is 4–8 h on a 32-core box, not viable on hosted runners per release, and the ungoogled-on-CEF path is undocumented folklore (forum thread, "not all patches apply"). CEF Alloy already has no sign-in, sync, Chrome UI, or account machinery. What is left is switchable or blockable. Keep a source build as a documented v2 option. |
| Storage isolation | One `CefRequestContext` per Space with `cache_path = <profile>/spaces/<id>` | Chromium profiles | Context = Chromium `BrowserContext`/`StoragePartition`: cookies, localStorage, IDB, cache, SW, permissions all isolated. Exactly what Arc "profiles" do. Several Spaces may share one context (Arc allows that too). |
| Updates | **GitHub Actions cron** bumps CEF pin from `cef-builds.spotifycdn.com/index.json` and the `cef` crate, opens a PR, CI builds, auto-merge on green, release | Sparkle/own update server | No code change needed for an engine bump. App self-update is v1.1. |
| Extensions | **Chrome Web Store extensions, verified 2026-10-22.** CEF runs on the Chrome bootstrap, so every Space's request context is a real Chrome profile with the extension system. MV3 content scripts, declarativeNetRequest and service workers run inside our Alloy-style OSR pages. Chrome's own extension UI (install prompt, chrome://extensions, action popups) cannot be hosted in OSR, so it opens in a native Chrome-style window on the same profile (`WebHost::open_extensions`). Ad blocking = user installs uBO Lite. | Bundled adblock engine (removed: list licensing); custom Chromium build like Arc (rejected in de-Googling row) | Arc compiles Chromium and draws extension icons and popups in its own UI. Next step toward that: read each extension's `action` from its manifest, show icons in the sidebar, and render `default_popup` in an OSR page inside a GPUI popover. |

---

## 3. What Arc actually is (feature inventory to reproduce)

Must-haves (in the brief) are bold.

**Sidebar (vertical tabs)**
- **Vertical tab column**, collapsible (⌘S), auto-hide with hover reveal.
- **Side is configurable: left (default) or right**; all slide/reveal animations and the space-swipe direction mirror with it. Setting is global, Space override allowed.
- Four zones top→bottom: Favorites grid (cross-space pinned icons), Pinned tabs, Today (unpinned), New Tab.
- Drag between zones; drag reorder; folders inside pinned.
- Auto-archive of Today tabs after N hours (12 h default, configurable), Archive view to recover.
- Tab rows: favicon, title, loading indicator, audio indicator, close-on-hover.

**Spaces**
- **Configurable Spaces**: name, emoji/icon, accent color/gradient theme, assigned storage profile.
- Space switcher strip at sidebar bottom; ⌃1…9 and ⌃⇥ / ⌃⇧⇥; **two-finger horizontal swipe** on the sidebar.
- Each Space has its own Pinned + Today lists; Favorites are global.
- Theme tints the whole window chrome (Arc's signature look).

**Navigation / command**
- Command bar (⌘T / ⌘L): URL, search, open tabs, history, bookmarks, commands, "switch to space".
- Peek (built 2026-10-22): any link that would open a tab (target=_blank, window.open) and any shift-click opens in a floating card over a scrim; Esc, the scrim or ✕ dismiss, ⤢ or ⌘O promote it to a tab keeping the live page. Cmd/middle-click still opens a background tab.
- lil-baa (Little Arc, built 2026-10-22): links from other apps (default browser, `open`, second launch) open in a small window; "Open in <Space>" (or ⌘O) moves the live page into the browser, the chevron picks another Space (which reloads it in that Space's storage). ⌘⌥N for an empty one is not built.
- Split view (⌃⇧+ or drag tab onto page), 2–4 panes.
- Back/forward by **two-finger swipe** on the page (standard macOS, must feel native), ⌘[ ⌘].

**Animations (must)** — every state change is animated, nothing snaps:
- Sidebar collapse/expand, hover-reveal slide.
- Space switch: horizontal slide of the whole sidebar content + theme color crossfade.
- Tab open/close/reorder: height collapse + list reflow (FLIP).
- Peek: scale+fade in from the link position; interactive dismiss follows the finger.
- Command bar: spring scale-in.
- Split view: pane width spring.

**Keybinds (desktop friendly)** — Arc's defaults, remappable. Table in §8.

**Explicitly out of scope for v1**: Boosts (CSS/JS injection), Easels, Notes, Arc Max/AI, sync, iOS companion,
extensions UI, password manager, Picture-in-Picture auto-pop.

---

## 4. Workspace layout

```
baa/
├─ Cargo.toml                     workspace; pins gpui rev + cef version (single source of truth)
├─ crates/
│  ├─ baa/                        bin: app entry, window, keymap, settings
│  ├─ baa-ui/                     GPUI views: sidebar, spaces, command bar, peek, split, theme
│  ├─ baa-web/                    CEF host: runtime init, OSR browser element, input/IME bridge,
│  │                              request-context manager, per-platform texture import
│  ├─ baa-model/                  Space/Tab/Session structs, persistence (SQLite via rusqlite,
│  │                              bundled), archive policy
│  └─ baa-privacy/                switch list, pref overrides, host blocklist, tests that assert
│                                 nothing outside the allowlist is contacted
├─ xtask/                         bundle (mac .app with helpers, win dir), sign, notarize, cef bump
├─ .github/workflows/             ci.yml · cef-bump.yml · release.yml
├─ assets/                        icons, fonts, default themes
└─ PLAN.md
```

Rust edition 2021 (2024 once the pinned gpui rev compiles under it), MSRV = stable (1.99.1 on the dev box). Keep dependency count low: gpui, cef, rusqlite, serde, parking_lot,
log, anyhow, url. No async runtime; CEF and GPUI both pump their own loops (CEF via
`external_message_pump` + `do_message_loop_work` scheduled on GPUI's foreground executor, as gpui-cef does).

---

## 5. De-Googling: what Helium removes, and what we do about each

Helium = ungoogled-chromium (source patches + domain substitution) + Helium patches + bits of Brave/Bromite/Debian/Inox.
Mapping each item to what a *binary* CEF Alloy build lets us do:

| Google/external hook | Present in stock CEF Alloy? | baa action |
|---|---|---|
| Google sign-in, sync, GAIA, account consistency | No (Chrome-layer, absent in Alloy) | nothing |
| Chrome UI, Omnibox suggest, NTP, promos, "AI mode" | No | nothing (ours is GPUI) |
| UMA metrics / crash reporting (Crashpad) | Opt-in; off unless `CefSettings` enables | leave off; no `crash_reporter.cfg` |
| Field trials / variations seed + `X-Client-Data` header | Fetch disabled in CEF, header logic exists | `--disable-field-trial-config --disable-features=…`; strip header in `OnBeforeResourceLoad` as belt-and-braces |
| Safe Browsing | Not wired in Alloy | nothing; verify with network test |
| Component updater (CRLSet, PKI metadata, Widevine, etc.) | Yes, calls `update.googleapis.com` | `--disable-component-update`; CRLSet loss accepted (Chromium still does OS revocation) |
| GCM / push | Yes (Alloy includes GCM driver) | `--disable-features=GCMDriver` equivalent + host block |
| Network time, DoH autoupgrade to Google, `clients*.google.com` connectivity checks | Yes | switches + host blocklist |
| Default search / suggest | n/a (no omnibox) | baa ships DuckDuckGo default, user-settable; no suggest requests unless user enables |
| Translate, spellcheck dictionary download | Yes | translate off; spellcheck uses OS (mac) / Hunspell local (win) or off |
| Media router / Cast | Yes | `--disable-features=MediaRouter` + host block |
| Google Fonts / Google domains in web content | web content, not browser | not our business (that is the page) |
| Widevine CDM | downloaded via component updater | not shipped in v1; Netflix won't work. Documented. |

Implementation = three mechanisms, all in `baa-privacy`:
1. **Switches** appended in `OnBeforeCommandLineProcessing`.
2. **Host blocklist** in `CefRequestHandler::GetResourceRequestHandler` for browser-initiated requests
   to `*.google.com`, `*.googleapis.com`, `*.gstatic.com`, `*.doubleclick.net` that are not page-initiated
   (check `request.resource_type` + initiator). Page content is never blocked by this list. Exception:
   `clients2.google.com/service/update2/crx`, the Web Store's extension download and update endpoint.
3. **CI test**: launch headless, open `about:blank` and a local page for 60 s behind a
   logging proxy (`mitmproxy` in CI), fail if any host outside the allowlist is contacted.
   This is the only thing that proves the stripping holds across engine bumps, so it gates the auto-bump PR.

Honest limit: domain substitution (rewriting google URLs at source) is impossible on a binary. Residual
risk is a code path we didn't list. The CI network test is the control.

---

## 6. Per-Space storage isolation

- `CefSettings.root_cache_path = <app data>/baa/profile`.
- Each **storage profile** (user-visible: "Personal", "Work", …) = `CefRequestContextSettings { cache_path: root/space-<id> (must be a direct child of root; Chrome rejects deeper paths), persist_session_cookies: true }`.
- A Space references exactly one storage profile; many Spaces may share one. Default: new Space gets a new profile (true Arc behaviour is "same profile unless you choose"; make it a toggle in the Space creator).
- Incognito Space = empty `cache_path` (in-memory context).
- Downloads, permissions, zoom per context via `CefRequestContextHandler`.
- Peek and Little baa inherit the context of the originating Space.
- Deleting a profile: shut its browsers, `cef_shutdown` is not needed, just `rm -rf` the directory after all browsers close (CEF holds files open until then; queue the delete on next launch if locked).
- Known CEF constraint: one executable instance per `root_cache_path`. Single-instance lock + IPC "open URL" to the running process (mac: `application:openURLs:` via GPUI's `on_open_urls`; win: named pipe).

---

## 7. Rendering and input pipeline (baa-web)

**Phase 1 (works today in gpui-cef):**
`CefRenderHandler::OnPaint(BGRA, dirty rects)` → copy into `gpui::RenderImage` → `img()` element sized to the tab's layout bounds. `GetViewRect` returns the element's bounds × scale factor; `GetScreenInfo` the display DPI. Resize = `WasResized()` from `prepaint`.

**Phase 2 (zero-copy):**
- mac: `shared_texture_enabled`, `OnAcceleratedPaint` gives an `IOSurface` → `CVPixelBufferCreateWithIOSurface` → GPUI `surface()`. Needs a BGRA path in `metal_renderer.rs` (today NV12-only). ~1 day patch; keep in our GPUI fork, PR upstream.
- win: `OnAcceleratedPaint` gives a D3D11 shared NT handle → `OpenSharedResource1` in GPUI's DirectX renderer → draw as texture. Needs a new `surface()` equivalent on the Windows backend. Note CEF issue #4057 (null handle on CEF 143, Windows); verify on 154+ before depending on it.
- Fall back to Phase 1 automatically if the accelerated path fails.

**Input**
- Mouse: GPUI `on_mouse_*` on the element → `SendMouseClickEvent/MoveEvent/WheelEvent` with view-local coordinates.
- Keyboard: `RAWKEYDOWN`/`KEYUP` only; text via `EntityInputHandler` → `ImeCommitText` (gpui-cef's design, avoids double chars and makes IME work).
- Scroll: GPUI `ScrollEvent` (renamed from `ScrollWheelEvent` in 2026) with phase → `SendMouseWheelEvent`. Precise deltas preserved so pages scroll like Chrome.
- Cursor: `OnCursorChange` → `window.set_cursor_style`. Custom cursors → arrow (accepted).
- Focus: `SetFocus(true/false)` tied to GPUI focus handle so ⌘L steals focus from the page correctly.
- Context menus: `CefContextMenuHandler` → render our own GPUI menu (OSR has no native one).
- Popups (`window.open`, select dropdowns): OSR delivers `<select>` popups via `OnPopupShow/OnPopupSize/OnPaint(PET_POPUP)`; composite as a second image layered above. `window.open` → new tab in same Space (or Peek when from pinned).
- Drag & drop: deferred (gpui-cef has none). Files dropped on page = v1.1.
- Find in page, zoom, print, dev tools (`ShowDevTools` into a separate CEF window, windowed mode is fine there), downloads via `CefDownloadHandler` with a GPUI download tray.

**Process model**
- mac: `baa.app/Contents/Frameworks/Chromium Embedded Framework.framework` + 5 helper bundles (`baa Helper`, `(GPU)`, `(Renderer)`, `(Plugin)`, `(Alerts)`). `cef-rs`'s `bundle-cef-app` + `Cargo.toml [package.metadata.cef.bundle]` generates these. Hardened runtime entitlements: `allow-jit`, `allow-unsigned-executable-memory`, `disable-library-validation`. Sign inside-out, notarize with `notarytool`.
- win: single exe + `libcef.dll` + resources next to it; subprocess = same exe (`CefExecuteProcess` early in `main`). Code-sign with an OV/EV cert (Azure Trusted Signing is the cheap GH-Actions-friendly option).

---

## 8. Keymap (defaults, all remappable in `keymap.json`, GPUI's native keymap format)

| Action | macOS | Windows |
|---|---|---|
| Command bar / new tab | ⌘T | Ctrl+T |
| Focus address (same bar) | ⌘L | Ctrl+L |
| Toggle sidebar | ⌘S | Ctrl+S |
| Close tab | ⌘W | Ctrl+W |
| Reopen closed | ⌘⇧T | Ctrl+Shift+T |
| Next / prev tab | ⌘⌥→ / ← , ⌃⇥ | Ctrl+Tab / Ctrl+Shift+Tab |
| Go to tab 1–8, last | ⌘1–8, ⌘9 | Ctrl+1–8, Ctrl+9 |
| Next / prev space | ⌃⌥→ / ← | Ctrl+Alt+→ / ← |
| Go to space n | ⌃1–9 | Ctrl+Alt+1–9 |
| Pin / unpin | ⌘D | Ctrl+D |
| Peek current link | ⇧-click | Shift-click |
| Little baa | ⌘⌥N | Ctrl+Alt+N |
| Split view | ⌃⇧+ | Ctrl+Shift+= |
| Back / forward | ⌘[ / ⌘] , swipe | Alt+← / → , swipe |
| Find | ⌘F | Ctrl+F |
| Dev tools | ⌘⌥I | F12 |
| Archive view | ⌘⇧A | Ctrl+Shift+A |

Browser-standard keys (⌘R, ⌘+/-/0, ⌘P, ⌘⇧N incognito space) included. Keys the page should get first
(text fields) are dispatched to CEF first; unhandled keys bubble to GPUI actions, using CEF's
`OnPreKeyEvent`/`OnKeyEvent` return values.

---

## 9. Gestures

- **Two-finger swipe left/right over the page** → back/forward with the rubber-band arrow, driven by `ScrollEvent` phases (began/changed/ended/momentum) on macOS. GPUI already handles NSEvent swipes (PR #23332 merged Jan 2025) for mice emulating swipes; we extend to trackpad phases. Only triggers when the page's `scrollLeft` is at the edge, checked via a cheap `eval` cache or by letting CEF consume the wheel event first and acting on the unconsumed delta.
- **Two-finger swipe over the sidebar** → previous/next Space, interactive (content tracks finger, springs to nearest).
- **Peek dismiss** → vertical swipe/drag on the Peek sheet.
- Pinch-to-zoom → page zoom (`SetZoomLevel`) with a GPUI zoom badge.
- Windows: precision touchpads deliver `WM_POINTER`/horizontal `WM_MOUSEHWHEEL` with phases via GPUI's Win32 backend; same handlers, verified on a Surface + a Dell XPS in CI's manual checklist.

---

## 10. Animation system

GPUI has `with_animation` + `Animation` (duration + easing, re-renders each frame). That covers
fades/slides/scales. Arc's feel is **spring physics with interruptible, finger-driven progress**, so:

- `baa-ui/motion.rs`: a tiny spring integrator (mass/stiffness/damping, ~40 lines), stored per-animated
  value in the model; `cx.request_animation_frame`-style re-render while any spring is moving.
- FLIP for list reorders: capture bounds in `prepaint` of frame N, animate offset from old→new in frame N+1.
- Theme colour crossfade: interpolate in OKLCH, not RGB.
- Page content stays live during animations: the native view moves with its element each frame. (Transforms other than move/clip, e.g. scale, need a snapshot; not done.)
- Respect "reduce motion" OS setting → durations ×0, springs become snaps.

---

## 11. Milestones

Each milestone ends with something runnable. Estimates are solo-dev, full-time.

| # | Milestone | Done when | ~Weeks |
|---|---|---|---|
| 0 | **Skeleton** | Workspace builds on mac+win in CI; GPUI window with empty sidebar; CEF 154 initialised, one OSR tab renders example.com as a rounded element with a GPUI badge over it (gpui-cef demo, vendored). | 1 |
| 1 | **Tabs + input** | Multiple tabs, switch, close, title/favicon/loading from CEF handlers; keyboard, IME, scroll, cursor, context menu, `<select>` popups, downloads, find. Sidebar today/pinned zones with drag. Session persisted to SQLite. | 3 |
| 2 | **Spaces + isolation** | Spaces CRUD, theme tint, per-space request context, incognito space, switcher + keybinds, single-instance + open-URL. Login to two Google accounts in two Spaces proves isolation. | 2 |
| 3 | **Privacy + extensions** | `baa-privacy` switches/blocklist; CI proxy test green; default search DDG; settings UI; Web Store installs via the Chrome-style window; extension icons + popups in the sidebar. | 2 |
| 4 | **Arc feel** | Spring/FLIP motion, sidebar collapse + hover reveal, space swipe, page back/forward swipe, Peek, Little baa, command bar with fuzzy search over tabs/history, auto-archive. | 4 |
| 5 | **Ship** | mac bundle signed+notarized, win signed; `release.yml`; `cef-bump.yml` live; split view; crash-free 1-week dogfood. v0.1 on baa.me. | 2 |
| 6 | **Zero-copy** | IOSurface path on mac, D3D11 shared-texture path on win, GPUI BGRA patch upstreamed. | 2 |

Total ≈ 15 weeks to a shippable v0.1 with zero-copy. Milestones 0–3 are the de-risking spine; if 0 fails on Windows OSR (unlikely, cef-rs supports it) the fallback is windowed CEF child HWNDs on Windows only, accepting that overlays/clip break there, as gpui-cef does with WebView2.

---

## 12. Mobile (noted, not committed)

You said "macOS & Android" for later; I read that as **iOS & Android**, since macOS is already in v1.
- **iOS**: App Store rules require WebKit; the page host becomes `WKWebView`. GPUI has an iOS backend in progress (zed PR #63068). The shell (`baa-ui`, `baa-model`) ports; `baa-web` gets a `wk` backend behind the same `WebHost` trait. No per-space storage isolation beyond `WKWebsiteDataStore(forIdentifier:)` (iOS 17+), which does map onto our profile model.
- **Android**: system WebView is Chromium, so a `WebHost` backend on `android.webkit.WebView` (or CEF's experimental Android support if it matures). GPUI has no Android backend; that is the real blocker.
- **What to do now**: keep all CEF types behind `baa-web::WebHost` (one trait, two methods groups: lifecycle/navigation and frame/input). Do not let `cef::*` leak into `baa-ui`. That is the only concession v1 makes; it costs nothing.

---

## 13. GitHub Actions

### `ci.yml` (push/PR)
- matrix: `macos-15` (arm64), `windows-2025`; `cargo build --release`, `cargo test`, `cargo clippy -D warnings`.
- cache: `~/.local/share/cef` (download-cef output, ~300 MB per platform) keyed on CEF version; `sccache` for Rust.
- privacy proxy test (§5.3) runs on both.
- uploads unsigned bundles as artifacts.

### `cef-bump.yml` (the "always current Chromium" requirement)
```yaml
on:
  schedule: [{cron: "17 6 * * *"}]   # daily
  workflow_dispatch:
jobs:
  bump:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - name: Find newest stable CEF common to all targets
        run: |
          curl -s https://cef-builds.spotifycdn.com/index.json -o cef.json
          # newest 'stable' cef_version present for macosarm64, macosx64, windows64, windowsarm64
          NEW=$(python3 xtask/cef_latest.py cef.json macosarm64 macosx64 windows64 windowsarm64)
          CUR=$(python3 xtask/cef_current.py Cargo.toml)
          echo "new=$NEW cur=$CUR" >> $GITHUB_OUTPUT
      - name: Resolve matching `cef` crate version      # crate is versioned MAJOR.x.y+CEF (e.g. 154.5.0+154.0.34)
        run: python3 xtask/cef_crate_for.py "$NEW"      # crates.io API; fail if no crate for that major yet
      - name: Update pins
        run: python3 xtask/cef_bump.py "$NEW" "$CRATE"  # edits workspace Cargo.toml + lock, Info.plist CFBundleVersion suffix
      - uses: peter-evans/create-pull-request@v7
        with:
          branch: bot/cef-${{ steps.ver.outputs.new }}
          title: "cef: bump to ${{ steps.ver.outputs.new }}"
          labels: cef-bump
      - run: gh pr merge --auto --squash   # merges when ci.yml (incl. privacy test) is green
```
- Rule: bump only when **all four** target platforms have the same stable `cef_version` in `index.json` (on 2026-10-22 mac is on 154.0.34 / Chromium 154.0.8037.98 while Windows is still 152.0.14, so the bump would wait). Option flag to allow per-platform skew if Windows lags more than 14 days.
- The `cef` crate tracks CEF majors within days (154.5.0+154.0.34 shipped 2026-10-07). If the crate lags, the PR is opened as draft with a note; `cef-dll-sys` can also be pointed at the raw binary via `CEF_PATH` as an escape hatch.
- After auto-merge, `release.yml` runs.

### `release.yml` (on merge to `main` touching `Cargo.toml` CEF pin, or tag)
- build both platforms, sign (mac: Developer ID + notarize; win: Azure Trusted Signing), produce `baa-<ver>-macos-arm64.dmg`, `baa-<ver>-windows-x64.msix`/zip, GitHub Release, upload to `https://baa.me/dl/` (Cloudflare R2 or GH Pages redirect).
- Version = `<app semver>+cef<cef_version>` so an engine-only release is visible.

---

## 14. Risks (ordered) and the control for each

1. **GPUI git churn** breaks the build between pins. Control: pin rev, bump monthly in a separate bot PR, keep a `gpui-ce` adapter branch alive.
2. **Windows OSR accelerated path flaky** (CEF #4057). Control: Phase 1 copy path is always available; Windows ships on it if needed.
3. **Residual Google traffic** after an engine bump. Control: proxy test gates every bump PR.
4. **cef-rs API breaks on major bumps** (bindings are generated per CEF major). Control: `baa-web` is the only crate touching `cef::*`; bump PRs that fail CI stay open for a human.
5. **IME / dead keys** unverified on real hardware in gpui-cef. Control: milestone 1 manual checklist (JP, DE, US-intl layouts).
6. **No Widevine** → no DRM video. Control: documented; v2 could fetch the CDM via a user-initiated download.
7. **Binary size**: CEF ≈ 200 MB installed per platform. Accepted; same as every CEF app.
8. **Licensing**: CEF BSD-3, Chromium BSD + third-party notices → ship `CREDITS.html`; GPUI Apache-2.0 (Zed's gpui crate); gpui-cef MIT/Apache.

---

## 15. First commands (milestone 0, day 1)

```sh
cargo new --bin baa && cd baa
cargo install export-cef-dir && export-cef-dir --force "$HOME/.local/share/cef"   # pulls CEF for host
git subtree add --prefix crates/gpui-cef https://github.com/bokuweb/gpui-cef main --squash
# Cargo.toml: gpui = { git = "https://github.com/zed-industries/zed", rev = "<pin>" }, cef = "154"
cargo run -p browser   # gpui-cef demo renders; then start replacing its toolbar with the sidebar
```

---

## Sources

- cef-rs / `cef` crate: https://github.com/tauri-apps/cef-rs , https://crates.io/crates/cef (154.5.0+154.0.34, 2026-10-07)
- CEF builds index: https://cef-builds.spotifycdn.com/index.json (stable: mac 154.0.34 / Chromium 154.0.8037.98; win 152.0.14 / 152.0.7977.167 on 2026-10-22)
- gpui-cef (CEF OSR inside GPUI, IME, IOSurface notes): https://github.com/bokuweb/gpui-cef
- GPUI window composition PR (native-view layering, still open): https://github.com/zed-industries/zed/pull/62379
- GPUI swipe events: https://github.com/zed-industries/zed/pull/23332 ; ScrollEvent rename: https://github.com/zed-industries/zed/pull/65805 ; iOS backend: https://github.com/zed-industries/zed/pull/63068
- gpui crates.io 0.2.2 (stale) https://crates.io/crates/gpui ; gpui-ce 0.3.0 https://crates.io/crates/gpui-ce ; gpui-kit webview limits https://gpui-kit.com/docs/webview/
- Helium / ungoogled-chromium: https://github.com/imputnet/helium-chromium , https://deepwiki.com/imputnet/helium/4.1-de-googling-and-privacy-modifications
- CEF on ungoogled-chromium (folklore, not reproducible): https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=19804
- CEF request context settings: https://cef-builds.spotifycdn.com/docs/113.3/structcef__request__context__settings__t.html
- CEF accelerated OSR issues: https://github.com/chromiumembedded/cef/issues/4057
- CEF mac signing/notarization: https://github.com/chromiumembedded/cef/issues/2744
- Arc feature references: https://keyshortcuts.net/arc-shortcuts , https://resources.arc.net/hc/en-us/articles/20498377604887-Arc-for-macOS-2023-Release-Notes , https://supasidebar.com/blog/what-is-arc-browser-sidebar-2026

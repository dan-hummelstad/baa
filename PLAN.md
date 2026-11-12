# baa — plan

An Arc-like browser. Chromium engine, de-Googled, with a GPUI shell written in Rust.
Domain: baa.me. Targets: **macOS** (Windows later, via the same `//baa`). iOS/Android later (noted, not committed, see §12).

Researched 2026-10-22. Versions quoted are the ones live that day.

The engine is specified in `PLAN-chromium.md`. Sections marked *superseded* are kept for history.

---

## 1. One-paragraph architecture

The whole UI (sidebar, spaces, command bar, peek, animations) is a GPUI app. The engine is baa's own
Chromium build: Helium's patch set (de-Googled) plus `//baa`, a layer that exposes real Chrome
browsers, profiles and tabs through a C ABI (`BaaCore.framework`, loaded by the Rust binary with
`dlopen`). Every tab is a Chrome tab whose page `WebContents` NSView sits **under GPUI's layer**. The
page element punches a rounded transparent hole in GPUI's frame and moves the view to its rect, so GPUI
still draws chrome, popovers and the command bar over it, while Chromium renders and scrolls natively at
the display's refresh rate. Each Space is one Chrome `Profile` under `--user-data-dir`, which gives
per-space cookies / localStorage / IndexedDB / service workers and a real extension system. A weekly
bot PR takes new Helium releases; a release chain builds the engine in CI. No baa.me backend exists in v1.

```
┌───────────────────────────── baa (GPUI, Rust) ─────────────────────────────┐
│  sidebar · spaces · command bar · peek · little baa · animations · keymap  │
│                                                                            │
│   Tab = gpui element: rect + hole  ──►  page WebContents NSView under gpui │
│   mouse/keys/IME/scroll: native to Chromium  ──►  one Chrome Profile/Space │
└────────────────────────────────────────────────────────────────────────────┘
  baa.app/Contents/MacOS/baa  +  Frameworks/BaaCore.framework (Helium + //baa)
                              ← Helium releases: weekly bump PR
```

---

## 2. Decisions and why

| Decision | Choice | Rejected | Why |
|---|---|---|---|
| Engine host | **Own Chromium build (BaaCore)**, native page view under GPUI's layer | CEF; WKWebView | See `PLAN-chromium.md`. |
| UI | **GPUI from the Zed git repo** (pin a rev, as gpui-component does) | `gpui` 0.2.2 on crates.io (stale, Oct 2025), gpui-ce 0.3.0 | Zed paused community GPUI publishing in Feb 2026; the crates.io version lags a year. Git pin of mainline keeps Windows (Win32 + DirectX) and macOS (Metal) backends current. gpui-ce is a fallback if mainline breaks us; its API is a superset. |
| Frame path | Native: Chromium presents its own layer; nothing is copied. | — | See `PLAN-chromium.md`. |
| De-Googling | Helium's patch set in the build, plus baa's switches | Runtime stripping of stock CEF | See `PLAN-chromium.md`. |
| Storage isolation | One Chrome `Profile` per Space (`space-<id>`) | — | See `PLAN-chromium.md` and §6. |
| Updates | Weekly Helium bump PR, release chain builds the engine, `release.yml` ships a signed DMG; no in-app updater yet | Sparkle / own update server | See `PLAN-chromium.md`, `PLAN-chromium-phase5.md`. |
| Extensions | Chrome Web Store extensions on real Chrome profiles; baa draws the toolbar and popups | — | See `PLAN-chromium.md`. Per Space by default; "Add to All Spaces" (Settings → Extensions, or the install prompt's Space picker) installs one into every Space's profile through `baa_profile_copy_extension` (ABI 8, no second prompt). Each Space keeps its own copy's data and on/off; removing a shared one removes it everywhere. Deleting a Space names the extensions that go with it. |

---

## 3. What Arc actually is (feature inventory to reproduce)

Must-haves (in the brief) are bold.

**Sidebar (vertical tabs)**
- **Vertical tab column**, collapsible (⌘S), auto-hide with hover reveal.
- **Side is configurable: left (default) or right**; all slide/reveal animations and the space-swipe direction mirror with it. Setting is global, Space override allowed.
- Four zones top→bottom: Favorites grid (cross-space pinned icons), Pinned tabs, Today (unpinned), New Tab.
- Drag between zones; drag reorder; folders inside pinned, nested to any depth (folder menu "New Subfolder"; the Arc import keeps Arc's nesting).
- Live folders (built 2026-11-07): a folder whose tabs mirror GitHub pull requests or issues, or an RSS/Atom feed (or the feed a page links to). Made with one item, "New Live Folder from This Page": any github.com page picks its own kind (a repo's pull requests or issues, a list page's query, else your own). The folder menu's submenu (as in Arc) shows the last fetch, Refresh, filters (Created by Me / Review Requests / Team Review Requests; for issues Assigned to Me / Mentioning Me) and Repositories to hide. Refreshed every 10 min and from the folder menu, through the system `curl`; GitHub signs in with `GH_TOKEN`/`GITHUB_TOKEN` or the `gh` CLI's login. A tab that drops out of the source stays while it is open. Live folders take no dragged tabs and hold no subfolders.
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
extensions UI, password manager.
Not out of scope: ad blocking. uBlock Origin (Helium's) is built in, one per Space, with bundled filter lists.

---

## 4. Workspace layout

```
baa/
├─ Cargo.toml                     workspace; pins the gpui rev (single source of truth)
├─ crates/
│  ├─ baa/                        bin: app entry, window, keymap, settings
│  ├─ baa-ui/                     GPUI views: sidebar, spaces, command bar, peek, split, theme
│  ├─ baa-web/                    WebHost contract + BaaCore host: dlopen of BaaCore, page views,
│  │                              input/IME bridge, profile manager
│  ├─ baa-model/                  Space/Tab/Session structs, persistence (SQLite via rusqlite,
│  │                              bundled), archive policy
│  └─ baa-privacy/                switches, pref overrides, proxy allowlist; the CI test asserts
│                                 nothing outside the allowlist is contacted
├─ chromium/                      vendored Helium + helium-macos, baa's patches, //baa, local-build.sh
├─ vendor/gpui_apple/             gpui's macOS backend with the punch-hole surface
├─ xtask/                         cargo bundle, sign the DMG, notarize, privacy_test.py
├─ .github/workflows/             ci.yml · chromium.yml · chromium-phase.yml · helium-bump.yml · release.yml
├─ assets/                        icons, fonts, default themes
└─ PLAN.md
```

Rust edition 2021 (2024 once the pinned gpui rev compiles under it), MSRV = stable (1.99.1 on the dev box). Keep dependency count low: gpui, rusqlite, serde, parking_lot,
log, anyhow, url. No async runtime; Chromium runs its main loop inside gpui's `[NSApp run]`.

---

---

## 5. De-Googling: what Helium removes, and what we do about each (*superseded*)

Kept for history. Today: Helium's patches, `chromium/patches/baa/*`, `baa_privacy::CORE_SWITCHES`, checked by the privacy test.

Ad blocking: uBlock Origin (Helium's) is built in, one per Space, and runs in ephemeral Spaces too. It uses
its bundled default filter lists (`remove-helium-services.patch` turns off Helium's list proxy), so they
change with each engine bump. Optional lists the user turns on are fetched from their own sites, unproxied.

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

- One Chrome `Profile` per Space under `--user-data-dir=<app data>/baa/chromium` (`space-<id>`; Chrome rejects deeper paths). Ephemeral (incognito) Space = off-the-record profile `baa::<id>`.
- A Space references exactly one storage profile; many Spaces may share one. Default: new Space gets a new profile (true Arc behaviour is "same profile unless you choose"; make it a toggle in the Space creator).
- Downloads, permissions, zoom per profile.
- Shared accounts (Google, Microsoft, GitHub): never by default. A fresh sign-in in a Space shows "Use this account in all Spaces?" (Just This Space / Use in All Spaces; buttons only, Share ignored for 500 ms). Shared, the account's HTTP cookies on its own domains are kept alike in every non-incognito profile by `chromium/baa/accounts.mm` (`baa_accounts_share`, ABI 14): sign-ins, refreshes and sign-outs in one are made in all, and a Space opened later catches up (newest wins). Settings → Spaces lists them with Stop Sharing; the answers live in `Settings::shared_accounts` / `kept_accounts`. Incognito profiles are never watched, asked about or stored.
- Peek and Little baa inherit the profile of the originating Space.
- Deleting a profile: close its pages, then `rm -rf` the directory (queued for the next launch, before Chromium opens anything).
- Single-instance lock + IPC "open URL" to the running process (mac: `application:openURLs:` via GPUI's `on_open_urls`).

---

## 7. Rendering and input pipeline (baa-web) (*superseded* except Input)

The OSR phases below are history: pages are native views (§1). Input still describes the UI side.

**Phase 1 (works today in gpui-cef):**
`CefRenderHandler::OnPaint(BGRA, dirty rects)` → copy into `gpui::RenderImage` → `img()` element sized to the tab's layout bounds. `GetViewRect` returns the element's bounds × scale factor; `GetScreenInfo` the display DPI. Resize = `WasResized()` from `prepaint`.

**Phase 2 (zero-copy):**
- mac: `shared_texture_enabled`, `OnAcceleratedPaint` gives an `IOSurface` → `CVPixelBufferCreateWithIOSurface` → GPUI `surface()`. Needs a BGRA path in `metal_renderer.rs` (today NV12-only). ~1 day patch; keep in our GPUI fork, PR upstream.
- win: `OnAcceleratedPaint` gives a D3D11 shared NT handle → `OpenSharedResource1` in GPUI's DirectX renderer → draw as texture. Needs a new `surface()` equivalent on the Windows backend. Note CEF issue #4057 (null handle on CEF 143, Windows); verify on 154+ before depending on it.
- Fall back to Phase 1 automatically if the accelerated path fails.

**Input** (the page is a native NSView under gpui's layer)
- Keyboard and IME go to Chromium natively: the page view is first responder. Shortcuts are offered to GPUI first; unhandled keys reach the page.
- Mouse: gpui keeps the mouse where the page hitbox is not occluded and hands the original NSEvent on; clicks and moves reach the page as native events.
- Scroll: native to Chromium (display link, momentum, precise deltas).
- Cursor and focus: Chromium sets its own cursor; focus follows the GPUI focus handle, so ⌘L takes it from the page.
- Context menus: `//baa` reports the menu; baa draws it as a native NSMenu (`WebHost::show_menu`).
- Popups (`window.open`, select dropdowns): `window.open` becomes a tab of the same Space; Chrome's own popups are native.
- Drag & drop: not yet. Files dropped on a page = v1.1.
- Find in page, zoom, print, DevTools (docked beside the page or in a window) and downloads (a GPUI tray) go through `PageHandle` / `WebHost`.

**Process model**
- mac: `baa.app/Contents/MacOS/baa` (the Rust binary) + `Contents/Frameworks/BaaCore.framework` (`BaaCore`, `Helpers/` renderer, GPU and plain helper apps, `Libraries/*.dylib`). Chromium's own signer (`sign_chrome.py`) signs inside-out with the entitlements in `PLAN-chromium-phase5.md`; the DMG is notarised with `notarytool`.
- Windows: later, with the `//baa` HWND port.

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
| Library, last section | ⌘⇧L | Ctrl+Shift+L |

Browser-standard keys (⌘R, ⌘+/-/0, ⌘P, ⌘⇧N incognito space) included. Keys the page should get first
(text fields) are dispatched to Chromium first; unhandled keys bubble to GPUI actions, using Chromium's
`OnPreKeyEvent`/`OnKeyEvent` return values.

---

## 9. Gestures

- **Two-finger swipe left/right over the page** → back/forward with the rubber-band arrow, driven by `ScrollEvent` phases (began/changed/ended/momentum) on macOS. GPUI already handles NSEvent swipes (PR #23332 merged Jan 2025) for mice emulating swipes; we extend to trackpad phases. Only triggers when the page's `scrollLeft` is at the edge, checked via a cheap `eval` cache or by letting Chromium consume the wheel event first and acting on the unconsumed delta.
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
| 5 | **Ship** | Engine move and release: see `PLAN-chromium.md` phases 5–6 (`PLAN-chromium-phase5.md`). Then split view; crash-free 1-week dogfood. v0.1 on baa.me. | 2 |
| 6 | **Zero-copy** | IOSurface path on mac, D3D11 shared-texture path on win, GPUI BGRA patch upstreamed. | 2 |

Total ≈ 15 weeks to a shippable v0.1 with zero-copy. Milestones 0–3 are the de-risking spine; if 0 fails on Windows OSR (unlikely, cef-rs supports it) the fallback is windowed CEF child HWNDs on Windows only, accepting that overlays/clip break there, as gpui-cef does with WebView2.

---

## 12. Mobile (noted, not committed)

You said "macOS & Android" for later; I read that as **iOS & Android**, since macOS is already in v1.
- **iOS**: App Store rules require WebKit; the page host becomes `WKWebView`. GPUI has an iOS backend in progress (zed PR #63068). The shell (`baa-ui`, `baa-model`) ports; `baa-web` gets a `wk` backend behind the same `WebHost` trait. No per-space storage isolation beyond `WKWebsiteDataStore(forIdentifier:)` (iOS 17+), which does map onto our profile model.
- **Android**: system WebView is Chromium, so a `WebHost` backend on `android.webkit.WebView` . GPUI has no Android backend; that is the real blocker.
- **What to do now**: keep all engine types behind `baa-web::WebHost` (one trait, two methods groups: lifecycle/navigation and frame/input). Do not let `cef::*` leak into `baa-ui`. That is the only concession v1 makes; it costs nothing.

---

## 13. GitHub Actions

Details: `PLAN-chromium-phase5.md` §6.
- `ci.yml` (push/PR, manual): `scripts` (python tests) and `build` on `macos-15`: `cargo test --release --workspace`, `cargo clippy ... -D warnings`, then the privacy test (§5.3) on the release engine for this commit's `chromium/` tree, if one exists (else a notice). No Chromium tree is needed: `baa-web` loads BaaCore at run time.
- `chromium.yml` (manual; `mode=dev` component build with the tree kept for 7 days, `mode=release` official build): chained phases (`chromium-phase.yml`), each compiling up to about 4.75 h. A release run uploads `baa-engine-<tree>` (`baa.app`, `baa Packaging`, `engine.json`, 90 days) and runs the `privacy` job on it.
- `helium-bump.yml` (weekly): a newer `imputnet/helium-macos` release is subtree-pulled onto a bot branch, a PR is opened, and `chromium.yml` (release) and `ci.yml` start on it. No auto-merge.
- `release.yml` (tag `vX.Y.Z` = `Cargo.toml` version): takes `baa-engine-<tree>` of the tagged commit (fails with the command to run if none), adds the Rust binary, signs with Chromium's signer, runs the privacy test on the signed app, builds and notarises `baa-<ver>-macos-arm64.dmg`, writes `rust-credits.html` with `cargo about`, publishes a GitHub Release. Engine versions go in the release notes, not in the version.

---

## 14. Risks (ordered) and the control for each

1. **GPUI git churn** breaks the build between pins. Control: pin rev, bump monthly in a separate bot PR, keep a `gpui-ce` adapter branch alive.
2. (removed with the Windows build)
3. **Residual Google traffic** after an engine bump. Control: the privacy test gates every release engine and every release.
4. (removed with the CEF bindings)
5. **IME / dead keys** unverified on real hardware. Control: manual checklist (JP, DE, US-intl layouts).
6. **No Widevine** → no DRM video. Control: documented; v2 could fetch the CDM via a user-initiated download.
7. **Binary size**: the app is about 350 MB (an official build; Helium is 342 MB, Arc 871 MB). Accepted.
8. **Licensing**: GPL-3.0-only (Helium is GPL-3.0). Chromium and Helium notices ship as `chrome://credits`, Rust crates' as `rust-credits.html` (`cargo about`); GPUI is Apache-2.0.
9. **Release chain length**: an official build on the free runners takes days across chained phases; it is not measured yet. Control: record it per run; paid runners if it matters.
10. **Security-fix latency**: Helium lag + weekly bump + one to two days of build. Control: dispatch `chromium.yml` on the bump branch at once.

---

## 15. First commands (milestone 0, day 1) (*superseded*)

Milestone 0's day-1 CEF commands, kept for history. Today: README "Build".

```sh
cargo new --bin baa && cd baa
cargo install export-cef-dir && export-cef-dir --force "$HOME/.local/share/cef"   # pulls CEF for host
git subtree add --prefix crates/gpui-cef https://github.com/bokuweb/gpui-cef main --squash
# Cargo.toml: gpui = { git = "https://github.com/zed-industries/zed", rev = "<pin>" }, cef = "154"
cargo run -p browser   # gpui-cef demo renders; then start replacing its toolbar with the sidebar
```

---

## Sources

- GPUI window composition PR (native-view layering, still open): https://github.com/zed-industries/zed/pull/62379
- GPUI swipe events: https://github.com/zed-industries/zed/pull/23332 ; ScrollEvent rename: https://github.com/zed-industries/zed/pull/65805 ; iOS backend: https://github.com/zed-industries/zed/pull/63068
- gpui crates.io 0.2.2 (stale) https://crates.io/crates/gpui ; gpui-ce 0.3.0 https://crates.io/crates/gpui-ce ; gpui-kit webview limits https://gpui-kit.com/docs/webview/
- Helium / ungoogled-chromium: https://github.com/imputnet/helium-chromium , https://deepwiki.com/imputnet/helium/4.1-de-googling-and-privacy-modifications
- Arc feature references: https://keyshortcuts.net/arc-shortcuts , https://resources.arc.net/hc/en-us/articles/20498377604887-Arc-for-macOS-2023-Release-Notes , https://supasidebar.com/blog/what-is-arc-browser-sidebar-2026

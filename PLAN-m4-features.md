# baa — M4 features plan

Written 2026-11-08 against `main` at `f6c21edd` (ABI 8, Helium 0.20.3.1, Chromium 156.0.8078.79).
Spec level: where each change goes and its outline, not exact diffs. Paths are repo-relative;
line numbers are as of that commit.

Features: **A** split view · **B** uBlock Origin · **C** media mini-player + auto PiP ·
**D** Claude (MCP server + in-browser chat with the user's own Claude Code) ·
**E** developer tools (environments, responsive view, dev servers, console badge) ·
**F** tab search + Clear Today.

---

## 0. Shared engine work: ABI 10

Several features need the same thing from //baa. They all land in **one** ABI bump so `ci.sh`
and the release chain run once. Bump steps follow commit `daaf3b6f`: `BAA_ABI_VERSION` in
`chromium/baa/baa.h:36`, `ABI_VERSION` in `crates/baa-web/src/core_host/ffi.rs:6`, symbols in
`Core` (`ffi.rs:214`) and the `core!` list (`ffi.rs:306+`), callbacks in `Callbacks`
(`ffi.rs:175`) and `dispatch.rs`, test `header_matches` (`ffi.rs:425`), mock
(`crates/baa-ui/src/mock.rs`), and the harness (`crates/baa-chromium`).

| New in ABI 10 | Used by | //baa outline |
|---|---|---|
| `baa_tab_cdp(tab, id, json)` + callback `cdp(ctx, tab, id, json)` (id 0 = event) | D, E | New `chromium/baa/cdp.mm`: a `content::DevToolsAgentClient` attached in-process with `DevToolsAgentHost::GetOrCreateFor(wc)->AttachClient`, `DispatchProtocolMessage` in, `DispatchProtocolMessage` back out. **No `--remote-debugging-port`, no socket.** Detach on tab close. Chrome supports several clients, so the user's DevTools keeps working. |
| `BaaTabState.muted` | C | `tab.mm:126`, next to `audible`: `wc->IsAudioMuted()`, re-sent from `OnAudioStateChanged` (`tab.mm:162`). |
| callback `media(ctx, tab, const BaaMedia*)` + `baa_tab_media_action(tab, action)` | C | `content::MediaSessionObserver` on `MediaSession::Get(wc)`: title, artist, artwork URL, playing, and a bitmask of the actions the page supports. Actions: play, pause, next, previous, enter PiP (`MediaSession::Resume/Suspend/NextTrack/PreviousTrack/EnterPictureInPicture`). |
| uBO visible (behaviour, no symbol) | B | `extensions.mm:263`, `:300`, `:435`, see B. |

Why one generic CDP hook instead of a function per need: screenshot (`Page.captureScreenshot`),
read page (`Runtime.evaluate` with `returnByValue`), real input (`Input.dispatchMouseEvent` and
`Input.insertText`), console (`Runtime.consoleAPICalled` and `Log.entryAdded`), and device
emulation (`Emulation.*`, `Network.setUserAgentOverride`) all come from that one pipe.
`baa_tab_eval` has no result today (`tab.mm:669`, `NullCallback`), and none of the other
capabilities exist (`baa.h` has no capture, console, input, or UA calls).

**Security rule for the hook:** CDP is all-powerful, so only Rust decides what passes. baa-web
exposes typed `PageHandle` methods (below), never raw CDP to baa-ui or the agent. The method
allowlist lives in one place, `crates/baa-web/src/core_host/cdp.rs`.

New `PageHandle` methods (`crates/baa-web/src/lib.rs:163`; implementation in
`core_host/page.rs`; mock in `crates/baa-ui/src/mock.rs:70`):
`eval_value(script, done)`, `screenshot(done)`, `click(x, y)`, `insert_text(s)`,
`press_key(k)`, `set_emulation(Option<Device>)`, `set_media_action(a)`, plus
`PageEvent::{Console{level, text}, Media(Media), Muted(bool)}` (`lib.rs:105`).
Console events are only subscribed (`Runtime.enable`) on tabs that ask for them: localhost tabs
(E) and agent tabs (D). That keeps the cost off normal browsing.

---

## A. Split view

PLAN.md §3:74, §8:214 (⌃⇧=), §10:83 (pane width spring). README calls it M4, PLAN's milestone
table says M5. The placeholder note is at `crates/baa-ui/src/actions.rs:50`.

**What already works:** more than one native page can be visible in one window. Docked DevTools
is a second `CorePage` beside the active one (`crates/baa-ui/src/browser/page_area.rs:21-65`).
Holes, hitboxes and click-to-focus are per page (`baa-web/src/core_host/slot.rs`,
`native.rs:233`). No engine change is needed.

**Model** (`crates/baa-model/src/lib.rs`)
- `Space` (`:68`) gets `#[serde(default)] split: Option<Split { tabs: Vec<TabId>, widths: Vec<f32> }>`.
  It persists for free in the session JSON (`store.rs:22`), and incognito is stripped because it
  lives on `Space`.
- Invariant: `active_tab` is always one of `split.tabs` while a split exists.
- Repair the split everywhere `active_tab` is repaired: `close_tab` (`:521`), `move_tab` (`:459`),
  `drop_tabs` (`:615`), `pick_neighbour` (`:501`). A split with one tab left dissolves.
- Protect pane tabs wherever only `active_tab` is protected today: `archive_today_where` keep-list
  (`:591`), `set_live_items` shown list (`:334`).
- Favourites in a split: a favourite pane belongs to its own Space (`activate_tab`, `:486`).
  Rule: a split only holds tabs of the active Space, and a favourite dropped in is opened as a
  copy in that Space. ponytail: no cross-Space panes.
- Unit tests next to the existing ones (`lib.rs:745`).

**UI** (`crates/baa-ui`)
- `page_area.rs:6` `render_page`: when the Space has a split, lay out N panes in a flex row with
  seams. Copy the DevTools seam drag (`page_area.rs:66-94`: a canvas that measures, plus a
  full-area `.occlude()` overlay while dragging). Docked DevTools sits beside the focused pane only.
- `browser.rs:451-453` visibility loop: `set_visible(true)` also **activates** the tab in Chromium
  (`baa-web/src/core_host/page.rs:289-295` → `tab.mm:725`). The loop runs in HashMap order, so
  Chromium's active tab would become arbitrary. Fix in baa-web: `set_visible` only shows, and
  `activate()` is a separate `PageHandle` call made by `sync` for the focused pane, **last**.
  Chromium's single active tab then always equals the focused pane. That keeps permissions
  (`baa.h:214`, held for background tabs), JS dialogs (`window.mm:44`), sheets (`asks.mm:365`)
  and extension action clicks (`extensions.mm:361`) correct.
- Focus follows clicks: a click already focuses a pane in gpui and Chromium (`slot.rs:93`), but
  `Space.active_tab` does not follow. Add `PageEvent::Focused` (emitted from `page.rs:148` focus-in)
  → baa-ui sets `active_tab`. Then the header URL, back/forward, find, asks and crash overlay
  (all keyed on `active_tab`) follow the focused pane with no further change. Asks, crash and find
  overlays move from card-wide to the focused pane's rect (`page_area.rs:95-98`, `:160`).
- Entry points:
  - action `SplitView` (⌃⇧=) in `actions.rs` and `keymaps/{macos,windows}.json`, splitting with
    the most recent other tab;
  - drag a sidebar tab onto the page: `DraggedTab` (`sidebar.rs:31`, make `id` `pub(crate)`),
    with an `.occlude()` drop overlay over the page while a `DraggedTab` drag is active (left or
    right half), otherwise `slot.rs:101` forwards the hover to Chromium;
  - tab context menu "Add to Split" and "Remove from Split";
  - a command-bar `Command`.
- Sidebar: split tabs render as one grouped row, as in Arc (`sidebar.rs:550`, `:664` selection).
- Motion: pane widths are owned `Spring`s ticked in `Browser::tick_motion` (`page_area.rs:300`).
  Don't use keyed `Motion`: it drops springs that settle at 0. Native resizes are throttled to
  40 ms (`native.rs:63`); that's acceptable, and the ponytail note stays.
- Test: `tests.rs:50-55` asserts exactly one visible page; update it and add a split case.

---

## B. uBlock Origin (verified: still built in, but hidden)

**Verdict: yes, baa ships uBO by default in every regular Space.**
- Helium's uBO 1.75.0 is downloaded via `chromium/helium/deps.ini:19-25`.
- It is installed as a component (`helium/patches/series:180-184`, `ublock-install-as-component.patch`
  → `ComponentLoader::AddUBlock`, id `blockjmkbacgjkknlgpkjjiijinjdanf`).
- MV2 stays alive through `ungoogled-chromium/extensions-manifestv2.patch` (`series:31`).
- Nothing in `chromium/patches` or the build scripts drops it. The prepared tree has it
  (`chrome/browser/extensions/component_loader.cc:618,715`).
- `ComponentLoader` is per profile, so there is one uBO per Space with its own settings.

**Problems found**
1. **Invisible.** `chromium/baa/extensions.mm:263` skips every component extension, so uBO has no
   toolbar icon or popup and can't be paused per site. `:300` and `:435` refuse changes and opening.
2. **Frozen filter lists.** `chromium/patches/baa/remove-helium-services.patch` makes
   `ShouldAccessServices()` false, so `ShouldAccessUBlockAssets()` is false and uBO uses only its
   bundled lists. Those refresh only when the Helium bump rebuilds the engine.
3. **Off in ephemeral Spaces.** Helium excludes uBO from "component extensions run in incognito"
   (`ublock-reconfigure-defaults.patch:116`).
4. **Docs contradict this.** `README.md:15-16` tells users to install uBO Lite, which gives
   two blockers. PLAN.md §3:87 and the CEF-era notes say no bundled blocker.

**Changes**
- `extensions.mm`: allow the one uBO component id through `baa_profile_extensions`, change and
  open (a single named constant, not all components). It then shows in the extensions toolbar and
  Settings → Extensions like any extension. Existing popup path: `baa_extension_popup`.
- Filter lists, **user decision**:
  - (a) keep the bundled lists (no network; stale for at most a week given the weekly bump);
  - (b) let uBO update from its upstream list URLs directly (unproxied; a small patch to
    `ublock-helium-services` behaviour, or a pref in `crates/baa-privacy/src/chromium.rs`).
  - Recommend (a) now. It matches "no external services", and (b) is one switch later.
- Ephemeral Spaces: recommend enabling uBO in OTR by a small `chromium/patches/baa/` patch that
  reverts that one exclusion. Blocking matters most in private windows.
- Docs: README "Privacy" bullet, PLAN.md §3 out-of-scope line and §5 note: "uBO (Helium's) is
  built in, one per Space."
- Harness: a `tests/ublock.expect` that loads a known ad URL and asserts it is blocked, run by
  `ci.sh` in CI.

---

## C. Media mini-player + auto Picture in Picture

**Today:** `audible` exists end to end (`tab.mm:126` → `PageEvent::AudibleChanged`
`page.rs:114` → `browser.rs:161` sets). Muted state lives only in the UI
(`prompts.rs:223 toggle_mute`) and is never read back. PiP is manual JS (`tools.rs:52 PIP_SCRIPT`).
There is no media metadata or control in the ABI.

**Engine:** `muted`, `media`, `baa_tab_media_action` from §0.

**UI**
- `browser.rs:161` keeps `muted` from `PageEvent::Muted` (engine truth) instead of its own set.
- Mini-player: a new `crates/baa-ui/src/browser/media.rs` rendered in `sidebar.rs:221` between
  `lists` and `render_bottom_bar` (`sidebar.rs:790`).
  - Shows the most recently playing tab that is **not on screen**, in any Space: artwork, title,
    play/pause, next, mute.
  - Clicking the title activates that tab (and its Space).
  - Height and opacity use a `Feel::Glide` spring.
- Tab row speaker (`sidebar.rs:513`) reads engine `muted`.

**Auto PiP**
- Chromium 156 already creates `AutoPictureInPictureTabHelper` for every tab, triggered by an
  active-tab change in the TabStripModel. `baa_tab_activate` calls `ActivateTabAt`, so switching
  tabs **inside a Space** may already work for sites that register the `enterpictureinpicture`
  media-session action.
- For any video, as Arc does: add `BrowserInitiatedAutomaticPictureInPicture` to an enabled-features
  list next to `CORE_DISABLED_FEATURES` (`crates/baa-privacy/src/chromium.rs:8`).
- Content setting `AUTO_PICTURE_IN_PICTURE` defaults to ASK. Set it to ALLOW from a Settings toggle
  "Picture in Picture when switching away" (`baa-model/src/settings.rs:204`, settings tab
  `settings/tabs.rs:550` area), via the existing pref and site-rule plumbing.
- A Space switch changes `Browser`, so the helper won't fire. In `browser/spaces.rs:6`
  `activate_space`, if the outgoing Space's visible tab is playing video, call
  `set_media_action(EnterPip)`.
- **Verify on the engine first:** the helper's safe-URL check (`auto_picture_in_picture_tab_helper.cc:816-848`)
  may refuse while `safebrowsing.enabled` is off (`PREFS_OFF`). If so, a one-line
  `chromium/patches/baa` patch skips that check when Safe Browsing is disabled.
- PLAN.md §3:88 lists "Picture-in-Picture auto-pop" as out of scope; remove that.

---

## D. Claude: MCP server + in-browser chat (Claude Code only)

Model: the way Pen (pen.dev) does it, checked on this machine. Pen ships a stdio MCP shim
(`Pen.app/.../mcp-server-darwin-arm64 --app desktop --agent claudeCodeCLI`) that Claude Code runs.
The shim talks to the running app over a unix socket (`~/.pencil/socket/pencil-desktop.sock`), and
Pen's in-app chat runs Claude through the Agent SDK on the user's own Claude login. baa does the
same **without Node**: the shim is the baa binary itself, and the chat spawns the user's installed
`claude` CLI. Flecha's alternative (HTTP on 127.0.0.1:7390 + bearer token) is rejected. A
localhost HTTP port can be reached by any local process and, through DNS rebinding and CSRF, by
web pages. A user-only unix socket can't.

### D1. Two ways in, one tool server
1. **External Claude Code:**
   `claude mcp add baa -- /Applications/baa.app/Contents/MacOS/baa mcp`.
   Settings → Claude shows the command with a copy button.
2. **In-browser chat (Dia/Arc Max-style, feature 11):** a panel (⌘J, Arc-like) that spawns
   `claude -p --input-format stream-json --output-format stream-json`, with `--mcp-config`
   pointing at `baa mcp`. Uses: "ask about this page", "summarise these tabs", "tidy my Today
   tabs into folders". Tidy, rename and summarise are prompt presets over the same tools, not
   separate features.

Both reach the same tool server inside the app. The chat is just one more MCP client.

### D2. Where the code goes
- `crates/baa/src/main.rs` (integration owner):
  - `baa mcp` subcommand (`args` already parsed at `:32`): a stdio↔socket bridge, a few dozen
    lines, with no gpui start;
  - a second listener `agent.sock` beside `ipc.sock` (`listen`, `:179`). The open-URL socket stays
    as it is.
- `crates/baa-ui/src/agent.rs` (new, baa-ui owner):
  - MCP JSON-RPC (initialize, tools/list, tools/call) on `serde_json`;
  - tool dispatch against `Browser` (`session`, `pages`, `with_page`);
  - the approval state and the activity log.
  - ponytail: a hand-rolled MCP subset of 3 methods, with no SDK crate. Upgrade if the spec's
    other capabilities are needed.
- `crates/baa-ui/src/agent_panel.rs` (new): the chat panel. It parses `stream-json` messages
  (assistant text, tool_use, result), with a transcript, input box and a Stop button.
- Child process: spawned from baa-ui via `std::process::Command`, on the same off-thread pattern
  as live folders' `curl` (`crates/baa-model/src/live.rs`).
- Settings: `baa-model/src/settings.rs:204` gets a `claude` block (enabled, allowed Spaces,
  `claude` path override, evaluate allowed). Settings UI: a new `Tab::Claude` (`settings/rows.rs:10`,
  body in `settings/tabs.rs`, ROWS entries).

### D3. Tools (all via the typed `PageHandle` methods of §0)
- **Read:** `list_tabs` (id, Space, title, URL; allowed Spaces only), `read_page` (text and links
  via `eval_value`, truncated), `screenshot`, `console_logs` (buffered from `PageEvent::Console`),
  `list_spaces`.
- **Act:** `open_tab`, `activate_tab`, `navigate`, `click`, `type` (CDP `Input.*`, so events are
  real, not `isTrusted:false`), `press_key`, `wait_for`, `close_tab`, `move_to_folder`,
  `rename_tab`. `move_to_folder` and `rename_tab` power "tidy"; they act only on the model, using
  `Session` methods that already exist.
- **Gated:** `evaluate` (arbitrary JS). Off unless enabled in settings, and every call asks.

### D4. Security (required, not optional)
- **Off by default.** Enabling it is explicit (Settings → Claude). Disabling it closes `agent.sock`,
  drops connections and kills the child.
- **Transport:**
  - `agent.sock` is created inside the app data dir with mode 0600.
  - Each accepted connection is checked with `getpeereid` (via `libc`, already in the graph
    through gpui) to match our uid.
  - There is no TCP listener and no `--remote-debugging-port`. The CDP hook is in-process only (§0).
- **Scope:**
  - Only Spaces ticked in settings are visible. **Ephemeral Spaces and lil-baa are never visible.**
  - `baa://`, `chrome://`, `chrome-extension://`, `devtools://` and `file:` pages are never readable
    or controllable.
  - There is no tool for cookies, storage, passwords, downloads or settings.
- **Approvals for actions:**
  - Read tools run freely inside allowed Spaces.
  - The first act tool on an origin raises a sheet in baa: "Claude wants to act on github.com —
    Allow once / For this session / Deny". The sheet uses the existing GPUI asks sheet,
    `crates/baa-ui/src/prompts.rs`.
  - Approvals are per origin and in memory only. They are cleared on disable and on quit.
  - `navigate` to a new origin needs that origin's approval before the next act.
- **No typing into secrets:** `type` refuses when the focused element is `input[type=password]` or
  has `autocomplete` containing `cc-` or `one-time-code` (checked through `eval_value` before
  `insertText`). ponytail: heuristic; the ceiling is fields that hide their purpose.
- **Prompt injection:** page text is untrusted. The in-browser chat runs `claude` with:
  - only baa's tools: `--strict-mcp-config`, `--allowedTools "mcp__baa__*"`, built-in tools off
    (`--tools ""`), so no Bash, Edit or file access;
  - `--permission-prompt-tool mcp__baa__approve`, so any remaining permission prompt appears in
    baa's UI;
  - a working directory of an empty `<app data>/agent/`, so no project `CLAUDE.md` or settings load.
  - Worst case, an injected page can act only on origins the user has approved.
  - External Claude Code is the user's own session with its own permissions; baa still enforces
    scope and approvals on its side.
- **Visible at all times:**
  - a sidebar strip "Claude · reading github.com" with Stop, while any client is connected;
  - a tinted outline on tabs being driven;
  - an activity log (tool, tab, time) in the panel.
- **Credentials:** baa never reads, stores or passes Claude credentials or `ANTHROPIC_API_KEY`.
  `claude` uses its own login. The environment is inherited, plus `PATH` from
  `$SHELL -lc 'command -v claude'`, because GUI apps don't get the login PATH; the result is cached
  in settings and can be overridden.
- **Not found / not logged in:** the panel shows "Install Claude Code" or "run `claude` once to log
  in" instead of failing silently.
- **Check before building:** the `claude` flags above (`--tools`, `--strict-mcp-config`,
  `--permission-prompt-tool`, `--setting-sources`) against `claude --help` on the installed version.
- **Tests:**
  - unit tests in baa-ui on the mock host: scope filter, approval state machine, password refusal,
    excluded schemes;
  - one `ci.sh` expect test for `screenshot` and `read_page` through CDP.

---

## E. Developer tools

### E1. Environments
- Model: `Settings.environments: Vec<Project { name, envs: Vec<Env { label, origin }> }>`
  (`settings.rs:204`; `#[serde(default)]`, no migration).
- Header badge: `crates/baa-ui/src/browser/header.rs:93-105`. The host is already computed at
  `:10`. Add a `LOCAL` / `STAGING` / `PROD` chip; PROD is tinted red.
- Action `CycleEnvironment` (⌥⌘E): same path and query on the next env's origin.
  - Files: `actions.rs`, `browser/actions.rs:6`, `keymaps/*.json`.
  - Command-bar `env <label>`: `command_bar.rs` `Command`/`COMMANDS` (`:47`, `:82`).
- Settings UI: a "Developer" tab (`settings/rows.rs:10`). It also absorbs the existing developer
  rows (`:144-145`, `:152`) and holds E2–E4 toggles.

### E2. Responsive view (⌥⌘M)
- Builds on **A**: a split whose second pane is the same URL in a new tab with
  `set_emulation(Some(Device::Phone))`. Emulation is CDP `Emulation.setDeviceMetricsOverride`
  393×852 @3x mobile, `setTouchEmulationEnabled`, and `Network.setUserAgentOverride` (§0).
- The phone pane has a fixed width (not springy) and its own reload. Closing it ends the split.
- ponytail: one device preset; a picker when asked.

### E3. Dev servers on localhost
- `crates/baa-model/src/live.rs:15` `LiveSource` gets `DevServers`. The fetcher runs
  `lsof -nP -iTCP -sTCP:LISTEN` (system tool, same off-thread pattern as `curl`), keeps the
  current user's processes on loopback, and names them by process (node, vite, cargo…).
- Refresh every 5 s while the window is key; the GitHub and feed sources keep 10 min.
- Shown as a live folder "Dev servers" (opt-in from Settings → Developer, or a sidebar item).
  Also on the start page: `start.rs:23 render` gets the list as a parameter.

### E4. Console error badge
- For localhost tabs only, baa-web enables console events (§0, `PageEvent::Console`). baa-ui keeps
  an error count per tab.
- A red badge on the tab row (`sidebar.rs:488`) and the dev-server row opens docked DevTools on the
  Console (`open_devtools`, existing).

---

## F. Tab search + Clear Today

- **Tab search already exists** in ⌘T: fuzzy over every Space's open tabs (`command_bar.rs:268-316`),
  with the Space in the detail. Add **archived tabs** as a result group, ranked below open tabs.
  Selecting one restores via `session.restore_archived` (`library.rs:526` does it today).
- **Clear Today (⇧⌘K, unbound today):**
  - Model: `archive_today` (`baa-model/src/lib.rs:586`) runs across all Spaces and keeps each
    Space's active tab. Add a `clear_today(space)` that archives the current Space's Today tabs
    **including** the active one, except split panes (A) and tabs being driven by Claude (D).
  - UI: action `ClearToday` in `actions.rs`, binding in `keymaps/*.json`, a `Command` in the bar,
    and the Today zone header menu (`sidebar.rs:208`).
  - A toast with **Undo** for a few seconds restores the batch (keep the archived ids).
- **Archive view ⇧⌘A** (PLAN.md §8:218, never bound): action → `open_library(Section::Archive)`
  (`library.rs:125`). Removes the matching half of the ponytail note at `actions.rs:50`.

---

## Order and owners

| # | Work | Owner | Depends on |
|---|---|---|---|
| 1 | F: Clear Today, ⇧⌘A, archived tabs in ⌘T | baa-model, baa-ui | — |
| 2 | B: docs fix, uBO visible, OTR patch, `ublock.expect` | engine, docs | — |
| 3 | ABI 10 (§0): CDP hook, `muted`, media | engine + baa-web | — |
| 4 | A: split view (incl. show/activate split in baa-web) | baa-model, baa-web, baa-ui | — |
| 5 | C: mini-player, auto PiP | baa-ui, baa-privacy | 3 |
| 6 | E1, E3 | baa-model, baa-ui | — |
| 7 | E2, E4 | baa-ui | 3, 4 |
| 8 | D: socket + shim, tools, approvals, panel | baa, baa-ui | 3 |

Items 1, 2, 3, 4 and 6 can run in parallel; there is no file overlap except
`actions.rs`/keymaps (small, rebase-friendly).

## Open decisions for the user
1. uBO filter lists: bundled-only (recommended) or fetched from upstream (B).
2. uBO in ephemeral Spaces: on (recommended)?
3. Claude default scope when enabled: current Space only (recommended), or all non-ephemeral Spaces.
4. Claude `evaluate` tool: off by default with ask-every-time (recommended), or absent.
5. Mini-player across Spaces (recommended), or current Space only.

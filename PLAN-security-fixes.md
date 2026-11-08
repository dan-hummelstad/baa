# Fix plan: security audit run-1

Covers all 14 leads in `~/audits/my-project/NEEDS-VALIDATION.md` (outside the repo) (audited at `22c00248`), plus the unreviewed `daaf3b6f` extension code.

Line numbers are from the audit. Before you edit, find the code again by function name. Since the audit, `live.rs`, `browser.rs`, `prompts/dialog.rs`, `peek.rs` and `extensions.mm` have changed, and HEAD is now `f6c21edd`. Lead 3 is still present at HEAD: `live.rs:274-282`.

The fixes are grouped by owner, following the CLAUDE.md ownership rules. Each item lands as its own small commit on `main` that passes `cargo test` for its crate. `//baa` changes are checked by build only; `ci.sh` is run by the owner or by CI, not by agents.

---

## Part A: changes to the code

### A1. CI/release owner (`.github/`, `xtask/`), first priority

**1. The release job must take the engine only from a trusted build** (lead 1)
- *Status (2026-11-08):* fixed: 02b5acdc (engine action trusts only a chromium.yml workflow_dispatch from this repo with a recorded sha256; signer from chromium/packaging). Test `.github/actions/engine/trusted_test.sh`. Owner: dispatch chromium.yml once, then a tag run.
- `.github/actions/engine/action.yml`: change the `find` step so it accepts an artifact only when the run that made it matches all three of these:
  - `path == ".github/workflows/chromium.yml"`
  - `event == "workflow_dispatch"`
  - `head_repository.full_name == $GITHUB_REPOSITORY`

  Do this by looking up `actions/runs/<workflow_run.id>` for each candidate. Fail if nothing matches. Never fall back to the first match.
- `chromium-phase.yml` (the producer): after it packs `engine.tar.zst`, write its `sha256sum` into the run summary. Also upload it as a small `baa-engine-<tree>.sha256` artifact in the same run.
- `action.yml` and `release.yml`: before unpacking, compare the downloaded tarball with the checksum from the *same trusted run*.
- `release.yml:80-105`: stop running `sign_chrome.py` from the artifact. Commit the packaging scripts under `chromium/packaging/`, or copy them from the Helium checkout at the pinned tag, and run that copy from the tagged checkout. The artifact supplies only `baa.app` and the framework.
- Check: a shell `test` in the action that fails when the producer's `event` is not `workflow_dispatch`. You can exercise it with `act`, or by dispatching the workflow once against a dummy artifact.

**2. The dev chain must take its base tree only from a trusted run** (lead 2)
- *Status (2026-11-08):* fixed: 39b112cd (usable() runs trusted.sh; a base without a checksum is rejected instead of kept with out/ wiped; guards untouched). First dev chain after this builds from scratch.
- `chromium-phase.yml:91` `usable()`: add `.event == "workflow_dispatch" and .head_repository.full_name == env.GITHUB_REPOSITORY and .path == ".github/workflows/chromium.yml"`.
- `chromium-phase.yml:153`: after restoring a base tree, delete the `out/Default` host tools and binaries that are not in siso's state, or wipe `out/` when the base run's checksum is not recorded.

  **Do not change the hand-off guards** (the `.siso_fs_state`/mtime checks at :129-132). CLAUDE.md requires that they stay, and the memory note `baa-chromium-ci-handoff` explains why: they protect against tar mtime precision problems, not against attackers. Put the identity check *in front of* them.
- `README.md:65`: tell developers to download only a `workflow_dispatch` run from `dan-hummelstad/baa`, and give them the one-line `gh api … -q '.event'` command to check it.

**3. Lock down workflows that fork PRs can trigger** (helps leads 1 and 2)
- *Status (2026-11-08):* fixed: f96a7212 (ci.yml `permissions: {}`, per-job grants). GitHub setting "Require approval for all outside collaborators" is the user's to switch on.
- In repo settings, set Actions → "Require approval for all outside collaborators". This is a setting, not code; record it in the README "CI" section.
- `ci.yml`: add `permissions: {}` at the top level, and grant only what each job needs.

**4. `cargo bundle` must not use an engine another account controls** (lead 14)
- *Status (2026-11-08):* fixed: d12ad81e (xtask `only_mine` on every path component, local-build.sh `mine()`). Test `xtask tests::only_mine_rejects_group_or_world_writable_and_symlinked_trees`.
- `xtask/src/main.rs:79`: remove the silent `/Users/Shared/hb` fallback. Use it only when `BAA_ENGINE` points there explicitly, or when every directory from `/Users/Shared/hb` down to `baa.app` passes both checks:
  - its `metadata().uid()` equals `getuid()`
  - `mode & 0o022 == 0`
- Add the same check to `chromium/local-build.sh`: `stat -f %u` against `id -u`, and fail if group or others can write.
- Check: a unit test in xtask with a temp dir set to `chmod 777` must be rejected.

### A2. `crates/baa-model`

**5. Live folders: http(s) only, all the way to curl** (lead 3)
- *Status (2026-11-08):* fixed: 44bdd972. Tests `live::tests::{feed_links_resolve_to_http_only, curl_is_held_to_http}`.
- `live.rs:275`: add `.filter(|u| is_web(u.as_str()))` to `next`, and return Err with "Feed link is not http(s)" when it fails.
- `live.rs:282`: add `"--proto", "=http,https", "--proto-redir", "=http,https"` to the curl argv. Put them before `--`.
- Check: a unit test of `feed_link` + resolve that feeds `gopher://`, `file:///`, `dict://` and `javascript:` hrefs and expects Err. It needs no network: split the resolve step into a pure function if needed.

### A3. `crates/baa-web` (the baa-web orchestrator; it also owns `vendor/gpui_apple`)

**6. The drag target pointer must not outlive its view** (lead 11)
- *Status (2026-11-08):* fixed: 611373a1 (Retained<AnyObject>, isDescendantOf check, cleared on view swap). Test `core_host::tests::pages::a_drag_target_is_dropped_with_its_view`. Owner: ASan/NSZombie run per NEEDS-VALIDATION lead 11.
- `core_host.rs:205`, `slot.rs:95`: store the drag target as `Retained<AnyObject>` (objc2), not a raw pointer.
- Before each use in `native.rs:487-518`, check that `isDescendantOf: container.page()` still holds. If not, clear the drag target and hit-test again.
- Clear the drag target in `swap_page` / on tab view changes.
- Check: build, plus a unit test that the drag target is cleared on swap, if the fake core can express that. Otherwise leave a `ponytail:` note and an owner ASan check.

**7. Downloads carry the incognito flag and the danger state** (leads 12 and 13; the ABI part needs the engine owner, see A5)
- *Status (2026-11-08):* fixed: af2f7244 (ABI **9**, not 8: baa.h was already 8 at HEAD). Tests `core_host::tests::menus::a_held_download_carries_incognito_and_danger_and_waits_for_the_user`, `m3::page_callbacks_become_events`.
- `lib.rs:84-94` `Download`: add `ephemeral: bool` and `danger: Option<DangerKind>`.

**8. Page-opened URLs are never replayed as typed loads** (lead 4, baa-web half)
- *Status (2026-11-08):* fixed: e8dd376e (`create_page(.., user_initiated, ..)`; a page URL that misses adoption opens only http(s), else about:blank; ADOPTION_EXPIRY 10 min). Tests `adoption::{a_page_supplied_url_that_misses_adoption_is_never_loaded, the_user_never_gets_a_tab_a_page_opened}`.
- `host.rs:207-209`, the fallback when adoption fails: when the URL is not `http(s)`/`about:blank`, open `about:blank` and do **not** load the URL. That covers `data:`, `baa://`, `chrome://`, `chrome-extension://`, `file:` and so on. Leave a `ponytail:` comment explaining that Chrome's original verdict was lost, so baa takes no risk.
- `skin.rs:78`: run `prepare_tab` → `to_chrome` only for URLs the user started (command bar or settings), never for an `OpenRequested` URL. The simplest way is a `user_initiated: bool` on `create_page`.
- Raise `ADOPTION_EXPIRY` (`dispatch.rs:29`) from 60 s, or keep the pending tab alive until a background tab is activated, so the fallback path becomes rare.

### A4. `crates/baa-ui`

**9. Peek and lil-baa adopt the tab Chrome opened, and never reload its URL** (lead 4, UI half)
- *Status (2026-11-08):* fixed: 1f4c27ab (Peek swaps to the adopted page, lil-baa opens an adopted tab; no `load_url` left in peek.rs/lil_baa.rs). Tests `tests::{links_a_peek_opens_adopt_chromes_tab_and_are_never_loaded, links_lil_baa_opens_become_adopted_tabs_and_are_never_loaded}`.
- `peek.rs:119`, `lil_baa.rs:96`: instead of `load_url(url)` on the requesting page, open the URL in a new tab through `create_page` (`take_pending`), which adopts Chrome's tab, or swap the card to the adopted page. If adoption fails, drop the request unless the URL is `http(s)`.
- Check: a test in `tests_*` that sends `OpenRequested{url:"data:text/html,x"}` and `OpenRequested{url:"baa://flags"}` on a Peek page and on a lil-baa page, and asserts that the fake core gets no `tab_load_url`.

**10. Dialogs need a delay before they accept input, and Enter must not accept risky dialogs** (lead 5)
- *Status (2026-11-08):* fixed: 8646c300. Tests `tests_dialogs::{enter_meant_for_an_alert_does_not_open_the_app_that_replaces_it, ok_waits_out_the_activation_delay, a_press_on_one_dialog_does_not_complete_on_its_replacement, a_held_enter_does_not_answer, the_checkbox_answers_with_its_state}` (the last one reversed).
- `DialogAsk`/`PermissionAsk`: add `shown_at: Instant`. In `answer_dialog` and `answer_permission`, ignore OK, Allow or Submit sent within 500 ms of `shown_at`.
- `on_dialog_key`: ignore events where `is_held` is set. For `ExternalProtocol` and `ExtensionInstall`, Enter does nothing and the user must click. Escape still cancels.
- `dialog.rs:169,223,236`: build the element ids from the request id, for example `("dialog-ok", ask.id)`. Then a mouse press that started on one dialog cannot complete on its replacement.
- `tests_dialogs.rs:57-66` currently *asserts* that Enter accepts an ExternalProtocol dialog. Reverse it: Enter must do nothing there. Add a test that sends Dialog A, then ExternalProtocol B, then an immediate `enter`, and checks that B is not answered.

**11. Focus never goes to the page while a dialog is still on screen** (lead 6)
- *Status (2026-11-08):* fixed: f36ca465. Test `tests_dialogs::closing_a_peek_leaves_the_keyboard_on_the_base_tabs_dialog`.
- `browser.rs:focus_page`: if `active_dialog().is_some()`, focus `input_focus` and return. This puts the guard in one place, so `close_peek` (`peek.rs:167`) and DevTools (`page_area.rs:173,181`) inherit it.
- Check: a test that opens a Peek, sends an Auth dialog for the base tab, sends `PageEvent::Closed` from the Peek, and asserts that `input_focus` has focus.

**12. Show the real host, without userinfo, and never cut off the site's own name** (leads 8 and 7, UI half)
- *Status (2026-11-08):* fixed: afa7ee09 (`host_of` via url::Url, `cut_left` with text_ellipsis_start, Chrome's dialog title first). Tests `browser::host_of_shows_the_real_host`, `tests_dialogs::a_js_dialog_shows_the_title_chrome_sent_not_the_tabs_host`. No 250-char render test: gpui's test API cannot read ellipsized text.
- `browser.rs:897` `host_of`: parse with `url::Url` (already a dependency), use `host_str()` only, and drop userinfo and the port unless it is non-default. Remove the hand-written split.
- `header.rs`, Peek, lil-baa, dialog titles: replace `.truncate()` with gpui's `text_ellipsis_start` (pinned `styled.rs:97`), so any cut happens on the left and the right end (the site's real domain) stays visible.
- JS dialog titles (`dialog.rs:103,114-115`): show `d.title` whenever the engine sends a non-empty title (see A5 item 13). Use `site_name` only as a fallback.
- Check: `host_of("https://a.com@b.com/") == "b.com"`. Also a render test with a 250-character host where the end of the host is visible.

**13. Fullscreen shows a notice with the origin and how to exit** (lead 9, UI half)
- *Status (2026-11-08):* fixed: 1b103ef5. Test `browser::tests_m3_page::fullscreen_and_pointer_lock_name_the_site_and_the_way_out`.
- `fullscreen_view`: on `PageEvent::Fullscreen(true, origin, kind)`, show an overlay for 3 s that names the origin and says "Press Esc to exit" ("Press and hold Esc" under keyboard lock). Show it again on `ExclusiveAccessUserInput`.
- Check: after the event, the view contains the origin text, following the `tests_m3_page.rs` pattern.

**14. Incognito leaves nothing in `baa.db`** (lead 12)
- *Status (2026-11-08):* fixed: 8a18b96f (favicons), fcc1c508 (downloads). Tests `tests::{incognito_favicons_stay_out_of_the_store, incognito_downloads_leave_no_record}`.
- `browser.rs:497-499`: put `save_favicon` behind `!private`, as the history writes beside it already are.
- `library.rs:153-165`: skip `record_download` when `download.ephemeral`.
- Fix the comment at `lib.rs:77` if anything else turns out to be written.
- Check: a FaviconImage event and a finished download from an ephemeral Space leave both tables empty, using the in-memory store.

**15. Warn before keeping a dangerous download** (lead 13, UI half)
- *Status (2026-11-08):* fixed: 4ce81a01. Test `tests::a_dangerous_download_waits_for_keep_or_discard`.
- In the download tray, show a row for a download with `danger.is_some()` with Keep and Discard buttons and the reason. Keep calls `download_validate`. Discard calls `download_remove`.

### A5. Engine owner (`chromium/baa`, `chromium/patches`). Check by build only; the owner runs `ci.sh`

**16. Pass the engine's own dialog title through** (lead 7)
- *Status (2026-11-08):* fixed in source: a9c02441. Needs engine build + ci.sh by owner (dialogs.expect: assert `title=".* says"`).
- `dialogs.mm:112-114`: name the `title` parameter of `CreateNewDialog` and set `BaaDialog.title` from it (`dialogs.mm:34`). It already holds Chrome's text, "X says" or "An embedded page at X says". Do the same for app-modal `AppModalDialogController::title()`.

**17. Fullscreen and pointer lock notice** (lead 9, engine half)
- *Status (2026-11-08):* fixed in source: 0a031a30 (new `exclusive_access` callback, not an extended `fullscreen`). Needs engine build + ci.sh (fullscreen.expect: add exclusive_access lines).
- `window.mm`: in `UpdateExclusiveAccessBubble`, forward the origin and bubble type through a new `fullscreen` callback field, or extend the existing one.
- `IsExclusiveAccessBubbleDisplayed`: return the real displayed state.
- `EnterFullscreen`: keep the `url::Origin` instead of discarding it.

**18. Downloads: hold dangerous items until the user decides** (lead 13, engine half)
- *Status (2026-11-08):* fixed in source: 2a5ed96a (header, ABI 9), 6b88b708. Needs engine build + ci.sh (downloads.expect: a dangerous/insecure case).
- `downloads.mm:45-51`: stop calling `ValidateDangerousDownload`/`ValidateInsecureDownload` on sight. Instead put `danger_type` and `insecure` in `BaaDownload`, and add `baa_download_validate(id)` / `baa_download_discard(id)`.
- Also add `ephemeral` from `BaaProfile::ephemeral` (lead 12).
- This changes the ABI: bump `baa.h` to **ABI 9** (it was already 8 at HEAD; the audit saw 7) and update `ffi.rs`. The baa-web owner reviews the Rust side.

**19. Tab-modal Chrome dialogs open over the tab that asked** (lead 10)
- *Status (2026-11-08):* fixed in source: 582d60f7 (full per-WebContents host, not the deferral version). Needs engine build + ci.sh. Ceiling: dialogs are still loose windows, so a base tab's dialog floats over an open Peek.
- `window.mm:582-610`: `GetWebContentsModalDialogHostFor(wc)` returns a host bound to *that* WebContents. Compute `GetHostView` and the dialog position from `wc->GetNativeView()`, not from `ActiveView()`.
- Send `OnPositionRequiresUpdate` to observers when the view moves or is resized.
- Smallest version: while a Peek owns focus, defer or hide dialogs whose WebContents is not the active tab.

**20. Review `daaf3b6f` (extensions copied to other Spaces)** (not reviewed by the audit)
- *Status (2026-11-08):* reviewed: (a) not real, the picker defaults to Here; (b) fixed ea020e51 (developer mode required, blocklisted copies not spread), needs engine build; (c) fixed 4a5044b5 in baa-ui (match the prompt's named extension, none when ambiguous). Test `browser::tests_shared_extensions::only_the_extension_the_prompt_named_is_moved`.

These three concerns came from a reviewer looking at the commit and are not confirmed. Check each against the code before changing anything:
- Copying an extension to another Space must go through the same consent step as installing it. At minimum, the install prompt's Space picker must be ticked by the user, not on by default.
- Unpacked extensions are copied only when the target Space has developer mode on.
- Copy by extension id, the one the user approved, not "the newest extension that appeared".

### Order of work

| Step | Items | Why this order |
|---|---|---|
| 1 | 1, 2, 3 | Release integrity. Low effort, and the biggest impact on users. |
| 2 | 5, 11, 12 | Small, can be tested offline, and they close the web-reachable issues. |
| 3 | 9 + 8, 10, 14 | Changes the UI; the existing tests need updating. |
| 4 | 16, 17 + 13 | Engine plus UI pairs; one engine build. |
| 5 | 18 + 7 + 15, 19 | Needs ABI 8. Bundle these into one engine build. |
| 6 | 4, 6, 20 | Local or lower priority. 20 can run in parallel at any time. |

After step 5, the owner runs `crates/baa-chromium/ci.sh` once. Then rerun the audit's local resolution plans for leads 1, 3, 4, 5 and 6 to turn "needs validation" into "fixed".

---

## Part B: preventing these problems from coming back

### B1. Additions to CLAUDE.md
- *Status (2026-11-08):* done, ce56ff2a.

Add a new section after "Style". Every rule below is one the audit found broken:

```markdown
## Security invariants (from audit run-1, 2026-11; see ~/audits/my-project)
baa replaces Chrome's own UI. Anywhere baa redraws or re-issues something Chrome
does, it must keep Chrome's check or be stricter. If unsure, ask the engine owner.

- **Page-supplied URLs are never re-issued as browser loads.** A URL from
  `OpenRequested`, a page, a feed, or an extension is adopted (`take_pending`) or
  dropped. Never pass it to `load_url`/`tab_create`/`to_chrome`; those are for
  URLs the user typed or picked.
- **Network fetches outside Chromium are http(s) only.** Anything that runs curl
  or another fetcher checks `is_web` on *every* URL, including discovered and
  redirected ones, and passes `--proto =http,https --proto-redir =http,https`.
- **Security prompts (permission, dialog, auth, external protocol, extension
  install) must:** show the origin the engine supplies (never the tab URL),
  ignore input for 500 ms after they appear or change, ignore key-repeat, never
  accept a risky prompt (external protocol, install) on Enter, and use element
  ids derived from the request id.
- **Focus never leaves a visible prompt.** Code that moves focus to a page goes
  through `focus_page`, which keeps the prompt focused while `active_dialog()`
  is some.
- **Origins on screen come from `url::Url::host_str()`** through the one shared
  helper. No string splitting. Long hosts are cut on the left
  (`text_ellipsis_start`), never on the right.
- **Incognito (`private`/`ephemeral`) never writes to `baa.db`.** Every new
  `store.*` write states how it handles private mode, and has a test.
- **Engine shortcuts that remove a Chrome safety check** (auto-validating
  downloads, stubbing a bubble, dropping a dialog's title or origin) need the
  user's approval and a `// security:` comment naming the check that was
  removed. `ponytail:` is not enough on its own.
- **Raw ObjC/NSView pointers are not kept across run-loop turns.** Use
  `Retained<_>`, or check the pointer again before each use.
- **CI artifacts are trusted by who produced them, never by name.** Any job that
  downloads an artifact checks the run's `path`, `event == workflow_dispatch` and
  `head_repository`, and checks its sha256. Jobs that hold secrets never run
  scripts that came from an artifact.
- **Never trust paths in `/Users/Shared` (or any location other accounts can
  write)** unless the owner is you and group/others cannot write.
```

Also change these existing CLAUDE.md lines:
- Ownership: add *"Security-relevant changes in a crate you do not own (a prompt, a URL path, an artifact) are handed to that owner with the matching invariant cited, not left in place."*
- Build/tests: add *"A change to a security prompt, URL handling, downloads, or private mode needs an offline regression test in the owning crate's `tests_*`. ci.sh is not a replacement for it."*

### B2. Checks that run automatically (CI owner)
- *Status (2026-11-08):* done, 0b0943bb (actionlint + zizmor job, actions pinned to SHAs) and afa99452 (`.github/security-lint.sh`, run by ci.yml). Also 03ef15b5 fixed the `spike` harness for ABI 9.

Keep these small and cheap: grep or `cargo test`, no new dependencies.
- **`cargo test` in `ci.yml`** already runs. The new regression tests from Part A protect the behaviours from then on. Make sure `baa-ui` and `baa-model` tests run on PRs.
- **A grep step in `ci.yml`** (in `xtask lint` or a 10-line shell step) that fails on these patterns:
  - `Command::new("curl")` without `--proto` on the same or next line.
  - `load_url(` inside `peek.rs` / `lil_baa.rs` / in an `OpenRequested` match arm.
  - `.truncate()` in files that render origins (`header.rs`, `dialog.rs`, `peek.rs`, `lil_baa.rs`).
  - `Validate(Dangerous|Insecure)Download` anywhere other than the user-decision path.
  - `upload-artifact`/`download-artifact` of `baa-engine-*` outside `chromium-phase.yml`/`release.yml`.
- **actionlint + zizmor** (a GitHub Actions security linter) as a one-step job on changes to `.github/**`. zizmor flags artifact poisoning and dangerous triggers. This is a CI tool, not a runtime dependency, but list it in the report as required.

### B3. Process
- *Status (2026-11-08):* memory line added; sandbox and the next audit run are the user's.

- **Audit `daaf3b6f` and later commits.** The next audit run starts from ref `22c00248..HEAD`, with item 20 as its first unit. `coverage-ledger.json` lets it skip the units already marked covered.
- **Set up a sandbox once**, for example a disposable macOS VM via `tart`, so a later run can *confirm* fixes instead of leaving them as needs-validation. Until then, after each fix the owner runs the matching local plan once by hand.
- **Memory:** after B1 lands, add one line to `MEMORY.md` saying that security invariants are in CLAUDE.md and each one traces back to an audit lead. Then future sessions treat them as deliberate, not as clutter to clean up.

### B4. Mapping of leads to their protections

| Lead | Direct fix (A) | Protection (B) |
|---|---|---|
| 1 artifact → signing | 1, 3 | CI-artifact invariant, zizmor, grep for artifact use |
| 2 dev base tree | 2, 3 | CI-artifact invariant, zizmor |
| 3 curl SSRF | 5 | fetch invariant, grep for curl `--proto`, unit test |
| 4 Peek/lil-baa replay | 8, 9 | page-URL invariant, grep for `load_url` |
| 5 Enter / mouse press swap | 10 | prompt invariant, reversed test |
| 6 focus_page | 11 | focus invariant, test |
| 7 dialog origin | 16, 12 | prompt-origin invariant |
| 8 host_of | 12 | origin-display invariant, grep for `.truncate()` |
| 9 fullscreen notice | 17, 13 | engine-shortcut invariant |
| 10 modal host | 19 | engine-shortcut invariant |
| 11 drag UAF | 6 | raw-pointer invariant |
| 12 incognito data saved | 7, 14, 18 | private-mode invariant, test |
| 13 dangerous downloads | 18, 7, 15 | engine-shortcut invariant, grep for Validate |
| 14 /Users/Shared | 4 | shared-path invariant, xtask test |

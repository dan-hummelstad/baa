# CI and automated testing (deferred, 2026-11-06)

The user put CI integration and automated testing on hold to get baa off the ground first. This plan
collects everything known about that work so it can be picked up later without re-reviewing. It covers
the //baa harness runner, the CI jobs, the release engine, and the checks that were never run.

## Where it stands

- **Harness runner exists:** `crates/baa-chromium/ci.sh [--engine DIR] [--only NAME] [--keep]` builds
  the harness against an engine, and `ci.py` runs `tests/*.txt` and checks them against `tests/*.expect`
  (log patterns, exit code, crash dumps, files, pasteboard, CDP probes). It passed 17/17 once against
  the local component tree (`/Users/Shared/hb`). It has never run on a CI runner.
- **CI wiring exists, untested:** `.github/actions/harness`, plus a `harness` job in `ci.yml` and in
  `chromium.yml`. Both run only when an engine artifact `baa-engine-<git rev-parse HEAD:chromium>`
  exists. No such engine exists for the current tree, so today they only print a notice.
- **Release engine:** no release-mode chain has ever finished. Run 42147459520 (started on 653f8884)
  is measuring how long one takes. Its engine is for an older `chromium/` tree and will not be used.
- **Privacy test:** wired as the `privacy` action (ci.yml, chromium.yml, release.yml). It has not run
  on BaaCore yet.
- **Agents do not run GUI tests.** The user's rule: checks are automated, not run by hand by agents.

## Order of work

1. **Get an engine for the current tree.** Dispatch `chromium.yml` with `mode=release`. Record the
   phase count and total time, then decide whether free runners are enough (PLAN-chromium-phase5.md
   §9 lists the fallbacks: no PGO, a paid runner, or this Mac as a self-hosted runner).
2. **Make the runner CI-proof** (the "Runner" list below). Then run `ci.sh` locally once to confirm.
3. **First CI harness run** on the engine from step 1. Fix whatever only fails on a runner. The likely
   spots are activation/key window, geolocation, crash dumps and timing (below).
4. **Turn on the gates:** the release lookup takes only engines whose harness and privacy jobs passed.
   The harness runs on PRs that change `chromium/` (see "Workflows").
5. **Parity checklist** (PLAN-chromium-phase5.md §5.0) as automated tests: the integration flows
   below, driven in-process, not through System Events.

## Runner (from the Opus review of c3b4fa37, 2026-11-06)

Must do before trusting a CI run:
- **perm2 on CI:** runners have no camera or microphone, so `getUserMedia` fails before the prompt.
  Add `--use-fake-device-for-media-stream` to the base args. Do not add `--use-fake-ui-…`, which would
  skip the real prompt.
- **ci.py:131/245:** a CDP `TimeoutExpired` aborts the whole run and deletes the logs. Catch it,
  record each test's exceptions as problems, delete a temp dir only when its test passed, and always
  print the summary.
- **Job time limit:** set `timeout-minutes: 45` on both `harness` jobs.
- **Screenshots:** the `screenshot` step (src/main.rs:540) can raise a Screen Recording (TCC) prompt
  that takes focus. Make it a no-op unless `BAA_HARNESS_SCREENSHOTS` is set.
- **Timing:** fixed waits leave only a few seconds of slack (`windows` `cdp-after: 48`, `keys`
  `cdp-after: 12`). Wait for a log marker instead (`spike: self-test done`, or a `script: done` line
  printed at the end of every script), with a time limit.

Should do:
- Log `NSApp.isActive` and whether the window is key at script start. If the runner does not activate
  the app, about 8 click/key tests fail at once and the cause is not obvious.
- Geolocation may be blocked by the OS on a fresh VM. Base perm1/all/dialogs' "ask once, then
  remembered" checks on notifications, and keep geolocation optional.
- `chrome://crash` in `close`/`misc`: add an `allow-crash-dump` expect key. An official build may
  write dumps where the component build did not.
- Point the fetch tests at the local `download-server.py` and keep at most one test that needs the
  public internet.
- Require Node 22 or newer (cdp.mjs uses the global `WebSocket`): `actions/setup-node`
  `node-version: 22` in the harness action, and a major-version check in ci.py.
- ci.sh: fail on a bad explicit `--engine` and print the engine chosen; check staleness on the
  `BaaCore.framework/BaaCore` binary, as xtask does; `rm -f` before copying over a signed binary.
- Give the download tests their own download directory inside the test's temp dir, so cleanup never
  touches `~/Downloads`.
- Stronger expectations. These tests only check "no crash" today:
  - `devtools-window`
  - `misc`: the download, file chooser, right-click and onbeforeunload close are not checked
  - `windows`: the `history` output is not checked
  Add checks for those, plus a `tab_favicon` line.
- ci.py: read each log once; clear the `BAA_SPIKE_*` environment variables as well as `BAA_HARNESS_*`.
- Not automatable as is: print, because its modal panel blocks quit (`menu_print.txt` was deleted).
  Options: a `--kiosk-printing`-style switch, or cancelling the panel through the harness.

## Workflows

- Upload `harness-logs` and `privacy-netlog` with `overwrite: true`. Otherwise re-running a failed job
  fails with a 409.
- Skip the sccache/rustup setup in the harness jobs when there is no engine.
- release.yml's engine lookup ignores the run conclusion. Prefer, or require, an engine whose
  harness and privacy jobs passed.
- chromium.yml can only be dispatched by hand (a user decision), so a PR that changes `chromium/` gets
  no harness or privacy run until someone dispatches a release chain. Consider a cheaper PR job: a
  dev-tree build of `//baa` plus the harness on the component engine.
- The engine key is the whole `chromium/` tree hash, so a docs-only or `local-build.sh`-only change
  forces a new multi-hour chain. Move files that don't affect the engine out of `chromium/`, or key on
  `chromium/{baa,patches,helium,helium-macos}`.
- helium-bump.yml uses `actions/checkout@v5`; the other workflows use `@v7`.
- helium-bump also dispatches `ci.yml` on the bot branch. That works now, because ci.yml has
  `workflow_dispatch`.
- shared.sh dev mode prints `enable_precompiled_headers=false` after the symbol lines, while the
  downloaded tree's args.gn has it first. The values are the same and nothing rebuilds. Swap the order
  for byte-identical output if it ever matters.

## Behaviour never exercised (needs a test or a decision)

From the C++ implementer's reports and the reviews:
- The combined `all` run with an extension popup open at quit. A fix (`CloseBareTabsForQuit` plus the
  popup registry) is built but has not been run.
- ⌘Q with the print panel open: hung for 30 s in M1.
- ⌘Q with a beforeunload dialog in a background tab. Rust auto-answers OK while quitting.
- HTTP auth, external-protocol, extension install and `getDisplayMedia` dialogs. Chrome draws these
  as loose windows (PLAN-chromium-phase4.md, M3 candidates).
- Memory-saver discard and the page view swap.
- Keys inside docked DevTools.
- Geolocation after "allow" gave `geo-err:1`, probably the helper's macOS location permission.
- 1Password sign-in and seeing tabs (manual, with the native messaging host copied).
- The privacy proxy test on BaaCore. Its fallback is a URL-loader patch (PLAN-chromium-phase4.md).
- Integration flows in the real app: Space switch = profile switch, deleting a Space, Peek, lil-baa,
  the settings sheet, extension popups from the toolbar, the downloads tray. `crates/baa/ui-check.sh`
  drives these with System Events. That needs Accessibility, which CI runners lack, so these need an
  in-process driver like the harness's.

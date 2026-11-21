# CI: the pipeline, its rules, and what is still open (2026-11-22)

The pipeline was reorganised on 2026-11-22 at the user's request: checks run quickly, full builds
are deliberate, releases are gated, and nothing that can fail without a fault gates anything. This
file is the plan for it. `PLAN.md` §13 lists the workflows; the workflow files say why each step is
there.

## The shape

| Tier | Workflow | Starts on | Takes | What a red run stops |
|---|---|---|---|---|
| Checks | `ci.yml`: `lint`, `test`, `privacy` side by side | every push to `main`, every PR, by hand, and from `release.yml` | about 5 min (was 10) | a release of that commit; the landing of a Chromium bump |
| Engine tests | `harness.yml` | by hand, from `chromium.yml` when an engine is finished, and on a push that changes what the tests can see | 25 min | nothing (user, 2026-11-21) |
| Engine build | `chromium.yml` → `chromium-phase.yml` × up to 12 | by hand, or `chromium-bump.yml` (daily, only for a newer build of the vendored milestone) | 1.5 h on a base tree, about a day from scratch | the landing of that engine |
| Release | `release.yml` | a `v*` tag (pushed by hand or by the `land` job) | an hour, most of it waiting for approval | the release |

## Rules

1. **A check is deterministic or it is not a check.** `ci.yml` holds only what gives the same answer
   twice: unit tests, clippy, greps, the scripts' own offline tests, and the privacy test (which
   fails only when the app contacts a host outside the allowlist). Anything that depends on timing
   or a window goes to `harness.yml`, which nothing waits for. No automatic retries anywhere: a
   retry hides the flake that `harness.yml` exists to show.
2. **Checks are quick.** Jobs run side by side, never one after the other; tests build the dev
   profile; Cargo's downloads are cached. The lint job is seconds and includes the workflow files
   themselves (actionlint, zizmor): a mistake in `chromium.yml` otherwise shows hours into a build.
3. **A full build is deliberate.** Nothing starts `chromium.yml` on a push. A person starts it, or
   the daily bump does, and only for a new build inside the milestone the patches were written for.
4. **Before a full build, the cheap checks.** In order of cost: `xtask/check-patches.sh` (a minute,
   no tree: do the patches apply?), `chromium/local-build.sh` on a dev tree (minutes: does `//baa`
   compile?), a `mode=dev` chain on a base tree (an hour or two), a `mode=release` chain.
5. **Compiled hours are never thrown away.** See the next section.
6. **A release is gated three times:** the checks pass on the tagged commit, a trusted engine exists
   for its `chromium/` tree, and the owner approves the job that holds the signing secrets. That job
   takes nothing from a cache and runs no script from an artifact.
7. **Workflow file names are part of the pipeline.** `trusted.sh` trusts an engine by the path
   `chromium.yml`; the bump and `land` jobs start and query `ci.yml`, `harness.yml` and `release.yml`
   by name. Renaming one orphans every engine built so far or stops a landing.
8. **An engine is its sources.** The engine's key (`.github/engine-key.sh`) is a hash of the five
   `chromium/` directories the build takes, so a workflow change, a doc or a script never costs a
   rebuild. While engines named by the old key (the tree hash of all of `chromium/`) are in use,
   which includes the 158 build that was running on 2026-11-22, a change elsewhere under
   `chromium/` still orphans those.

## Chromium trees: where hours were lost, and what closes each

A from-scratch release chain is about 21 hours of compiling in 5 phases. Each row is a way that
work was, or could be, thrown away. "Unrun" means the fix is on `main` since 2026-11-22 with its
offline tests passing, but no chain has run it yet: it landed without a test chain (the user's
choice), so the first chain after it is the test (see "Next").

| Loss | Seen | Closed by | State |
|---|---|---|---|
| tar's whole-second mtimes made siso recompile everything, four times | 2026-11-04 | pax format, and three guards that fail the phase | on `main` |
| `Pack tree` failed after the archive was complete (tar could not remove a directory), so the upload was skipped: a 4.75 h phase gone | run 43888560258 | `.github/pack-tree.sh`: when tar fails, the archive decides (it must read to its end and hold siso's state); `pack-tree_test.sh` reproduces that run | unrun |
| A step between Build and `Pack tree` fails (the summary's API call, the engine tarball) and vetoes the pack | not yet | the tree steps run on `!cancelled()` and the Build step's status alone; the summary cannot fail the job | unrun |
| The upload replaces the previous phase's tree before the new one exists (`overwrite: true` deletes first), so a failed upload leaves the chain with nothing | not yet; Helium and ungoogled-chromium have the same exposure | every phase uploads under its own name (`chromium-tree-xcode<v>-phase<N>`); only a finished tree takes the plain name. Trees are 5 to 7 GB, so keeping all of them costs nothing | unrun |
| A dev chain that fails to compile uploads nothing | by design until now | every ended Build step (running, finished, failed) packs, in both modes | unrun |
| A hand-off guard fires and the phase's tree is dropped with it | by design until now | the guard still ends the chain red, after the tree is uploaded | unrun |
| An unfinished tree expires after 2 days, a finished one after 14 | a failure on a Friday | 7 days and 30 days (a milestone is about four weeks) | unrun |
| A run is cancelled mid-phase | when someone does it | nothing can: a cancelled job is killed after five minutes and packing takes eight. The phase before it is kept, so the cost is at most one phase. Do not cancel a chain to push a fix; let the phase end | documented |
| The runner dies, or the job hits six hours | not yet | "Re-run failed jobs": the phase starts again from the tree before it, which is no longer replaced | unrun |
| A patch does not apply, found half an hour into phase 1 | run 43876197908 and others | `xtask/check-patches.sh` before the build | on `main` |
| A release chain starts from scratch although a finished release tree of the same milestone exists, because nobody passed `base_run` (the 157.0.8103.81 bump compiled for 24 h; its source differed from .79 in the `VERSION` file) | 2026-11-19 | every chain picks the newest finished tree of its mode and milestone; `base_run=none` is the from-scratch build | unrun |
| A change under `chromium/` that is not a build input (`local-build.sh`, `packaging/`) orphans the engine | not recently | the engine key is a hash of the five directories the build takes (`.github/engine-key.sh`); the lookup also takes the old name until the last old engine expires (February 2027) | unrun |

## Next

1. **Read the first chain after 2026-11-22.** In its first phase: `Pack tree` ends with a
   `name=chromium-tree-xcode<v>-phase1` line and `Upload tree` uploads that name. In its second:
   `Restore tree` downloads `-phase1`, and the run's artifacts then hold both `-phase1` and
   `-phase2`. At the end: the plain name, 30 days. If a phase fails in `Pack tree` or an upload, the
   phases before it are still there: start again with `base_run=<that run>`. To see all of it
   before a real build needs it: `gh workflow run chromium.yml -f mode=dev -f budget_minutes=15
   -f base_run=none`, three phases, then cancel (a test chain's title is not "chromium dev", so
   nothing picks its tree as a base).
2. **Decided by the user on 2026-11-22, all three done:**
   1. Every chain, release included, builds on the newest finished tree of its mode and Chromium
      milestone unless `base_run=none`. What it costs: a release binary then comes from an
      incremental build, and a file that a newer Chromium no longer produces could stay in
      `baa.app` from the older tree; `base_run=none` is the release that owes nothing to an
      earlier tree. Not measured yet: how much a new Chromium build of the milestone recompiles
      (its PGO profile may change, and with it every compile). A same-version resume takes 1.5 to
      3.5 h. Read the first bump's `base tree:` summary line and its phase count.
   2. The engine key is a hash of the build inputs (`chromium/{helium,helium-macos,patches,baa,
      vendor}`), not of all of `chromium/`. `engine-key_test.sh` fails when that list and the
      directories `chromium-phase.yml` copies into the build differ.
   3. The `release` environment takes only `v*` tags, and releases are immutable once published
      (both repository settings; `release.yml` fills a draft and publishes it last).
3. **Not planned:** a merge queue or required status checks (one committer, trunk-based: the gate
   is at the release, not at the push); nextest (the tests run for two seconds; the time is the
   compile); a second cache tool (sccache works and is shared by every job).

## Compared with others (read 2026-11-22)

- **Helium, ungoogled-chromium-macos:** the same chain of phases with an artifact hand-off, which is
  where baa's came from. Neither keeps a failed phase's work, neither has a resume from another
  run, both overwrite the one tree artifact, and both sign inside the build jobs. baa already
  differed by trusting artifacts by their producer, checking checksums and signing in a separate
  gated job.
- **Zed, uv, cargo, rust-analyzer:** tests and clippy are separate parallel jobs in the dev (or a
  lighter) profile; none tests with `--release`. Zed's and uv's release workflows run the tests
  again and gate publishing on an environment; uv turns caches off for releases. None runs
  actionlint or zizmor as a workflow of its own: they are a job or a step of the main CI.
- **Kubernetes, Chromium, Ladybird on flaky tests:** quarantine, do not retry. A flaky test is taken
  out of what gates and run where its failure rate can be read.

## Harness backlog (from the 2026-11-06 review; not re-checked on 2026-11-22)

### Runner (from the Opus review of c3b4fa37, 2026-11-06)

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
- Done 2026-11-21: no test needs the public internet. Every site a test loads is
  `tests/test-server.py` (`ci.py` `start_server`), and Chrome is given no other host.
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

### Behaviour never exercised (needs a test or a decision)

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

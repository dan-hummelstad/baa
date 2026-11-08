# baa — agent rules

Read PLAN.md first; it is the spec. Sections are referenced as §N.

## Build
- `. ~/.cargo/env` if cargo is missing from PATH.
- `cargo check -p <crate>` for your crate. Full `cargo build -p baa` is slow (gpui); run it once at the end, not per edit.
- macOS app only starts bundled: `cargo bundle --run` (needs an engine, README "Build").
- Never run two Chromium builds (`pgrep -x siso`); never remove the hand-off guards in `chromium-phase.yml`.
- Tests: `cargo test -p <crate>`. `//baa` changes are tested by `crates/baa-chromium/ci.sh` (CI runs it; it opens real windows, so agents do not run it unasked).
- A change to a security prompt, URL handling, downloads, or private mode needs an offline regression test in the owning crate's `tests_*`. ci.sh is not a replacement for it.

## Git: trunk-based
- Everything lands on `main`. No feature branches that live past the task, no PRs, no "merge this?" hand-offs.
- Commit small and often, each commit building and passing `cargo test` for the crates it touches.
- An agent isolated in a worktree still commits there, then lands on main: `git fetch origin && git rebase origin/main && git push origin HEAD:main` (fast-forward only; never force-push). Delete the worktree's branch afterwards.
- The user's checkout of `main` catches up with `git pull --ff-only`.

## Ownership (one crate per orchestrator; do not edit outside yours)
- `crates/baa-model`   data + SQLite + settings
- `crates/baa-privacy` switches, prefs, proxy allowlist
- `crates/baa-web`     BaaCore host (`WebHost` in lib.rs is the contract; UI codes against it)
- `crates/baa-chromium` the `//baa` test harness: a gpui shell (`spike`) plus `tests/*.expect`, run by `ci.sh` against an engine; engine orchestrator
- `chromium/baa`, `chromium/patches`: engine orchestrator; `chromium/helium*`: changes only through a Helium bump or the engine orchestrator
- `vendor/gpui_apple`  baa-web orchestrator
- `crates/baa-ui`      GPUI views, theme, motion
- `crates/baa`         app shell wiring (integration orchestrator only)
- `.github/`, `xtask/` CI/release orchestrator only
- Root `Cargo.toml` `[workspace.dependencies]`: add a dep there only if two crates need it; otherwise in your crate.
- Security-relevant changes in a crate you do not own (a prompt, a URL path, an artifact) are handed to that owner with the matching invariant cited, not left in place.

## Style
- Ponytail applies: smallest working code, no speculative abstractions, no single-impl traits except `WebHost`.
- `// ponytail:` marks a deliberate shortcut with its ceiling.
- Comments explain why, never what. Doc comments on pub items only when the name is not enough.
- No new runtime dependencies beyond those in root `Cargo.toml` without listing it in your report.
- gpui API: it is the pinned Zed rev (see root Cargo.toml). When unsure, read the source in `~/.cargo/git/checkouts/zed-*/<rev>/crates/gpui/`.

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

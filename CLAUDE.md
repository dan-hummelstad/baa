# baa — agent rules

Read PLAN.md first; it is the spec. Sections are referenced as §N.

## Build
- `. ~/.cargo/env` if cargo is missing from PATH.
- `cargo check -p <crate>` for your crate. Full `cargo build -p baa` is slow (gpui); run it once at the end, not per edit.
- macOS app only starts bundled: `cargo bundle --run` (needs an engine, README "Build").
- Never run two Chromium builds (`pgrep -x siso`); never remove the hand-off guards in `chromium-phase.yml`.
- Tests: `cargo test -p <crate>`. `//baa` changes are tested by `crates/baa-chromium/ci.sh` (CI runs it; it opens real windows, so agents do not run it unasked).

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

## Style
- Ponytail applies: smallest working code, no speculative abstractions, no single-impl traits except `WebHost`.
- `// ponytail:` marks a deliberate shortcut with its ceiling.
- Comments explain why, never what. Doc comments on pub items only when the name is not enough.
- No new runtime dependencies beyond those in root `Cargo.toml` without listing it in your report.
- gpui API: it is the pinned Zed rev (see root Cargo.toml). When unsure, read the source in `~/.cargo/git/checkouts/zed-*/<rev>/crates/gpui/`.

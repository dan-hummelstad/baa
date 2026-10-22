# baa — agent rules

Read PLAN.md first; it is the spec. Sections are referenced as §N.

## Build
- `. ~/.cargo/env` if cargo is missing from PATH.
- `cargo check -p <crate>` for your crate. Full `cargo build -p baa` is slow (CEF wrapper + gpui); run it once at the end, not per edit.
- macOS app only starts bundled: `cargo bundle` then `open target/bundle/baa.app`.
- CEF binaries: `CEF_PATH` is set in `.cargo/config.toml` (populated by `export-cef-dir`). Never commit them.
- Tests: `cargo test -p <crate>`.

## Ownership (one crate per orchestrator; do not edit outside yours)
- `crates/baa-model`   data + SQLite + settings
- `crates/baa-privacy` switches, host blocklist, adblock engine (uBO lists)
- `crates/baa-web`     CEF host (`WebHost` trait in lib.rs is the contract; UI codes against it) + `vendor/gpui-cef`
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

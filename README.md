# baa

An Arc-like browser: vertical tabs, Spaces with isolated storage, a command bar, and spring-driven
motion, on top of a stock Chromium engine that is stripped of Google services at runtime.

- Shell: Rust + [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) (Zed's UI framework).
- Engine: [CEF](https://bitbucket.org/chromiumembedded/cef). On macOS each page is a native Chromium
  view under GPUI's layer, so it scrolls and animates at the display's refresh rate, while GPUI still
  draws the chrome, popovers and command bar over it.
- Privacy: Chromium switches and a host blocklist strip Google services. No baa.me backend, no
  telemetry.
- Icons: [Lucide](https://lucide.dev) (ISC), embedded from `crates/baa-ui/assets/icons`.
- Extensions: Chrome Web Store extensions install per Space profile and run in every tab. For ad
  blocking, install uBlock Origin Lite. Open "Chrome Web Store" or "Extensions" from the command
  bar (⌘T); Chrome's own install and management UI opens in a separate native window.
- Targets: macOS and Windows. The engine pin is bumped automatically by a daily GitHub Action.

`PLAN.md` is the spec. `CLAUDE.md` holds the rules for agents working in this repo.

## Status

Early. Milestones 0–3 of `PLAN.md` are implemented: tabs, Spaces, per-Space storage, keymap,
command bar, de-Googling, plus Peek (links that open a tab, or shift-click, preview in a card) and
lil-baa (links from other apps open in a small window you can move into the browser), favourites
(shared by every Space) and folders. Browser basics: downloads (header button and list), history
(in ⌘T, and ⌘Y), permission prompts and per-site settings, page dialogs drawn over the page,
print, view source, reader mode, Picture in Picture, translate, tab mute, Settings (⌘,), and import
of Arc's Spaces and other browsers' history. Password managers work through their Chrome extensions;
Apple's iCloud Passwords cannot (macOS only lets Chrome and Edge start its helper). The UI follows
the pen.dev mocks. Still to come from milestone 4:
split view. Windows currently runs on WebView2 through the vendored gpui-cef fallback, not CEF.

## Build

```sh
rustup toolchain install stable
cargo install export-cef-dir
export-cef-dir --force "$HOME/.local/share/cef"   # CEF binaries; CEF_PATH is set in .cargo/config.toml
cargo bundle                                        # macOS: the app only runs bundled
open target/bundle/baa.app
```

Judge smoothness only on `cargo bundle --release`. A debug build lays out GPUI too slowly to
present more than ~60 fps. macOS asks for Documents access on every rebuild launched from inside
`~/Documents` (each build is a new ad-hoc signature); keep the checkout elsewhere to avoid it.

Development loop: `cargo check -p <crate>` and `cargo test -p <crate>`. A full `cargo build -p baa`
is slow; run it once at the end. `cargo test --workspace` and
`cargo clippy --workspace --all-targets -- -D warnings` are what CI runs.

Useful environment variables when running the bundle:

| Variable | Effect |
|---|---|
| `BAA_CEF_LOG=<file>` | CEF's own log, verbose |
| `RUST_LOG=info` | shell log on stderr |

App data lives in `~/Library/Application Support/baa` (`%APPDATA%\baa` on Windows): `baa.db`
(session + settings), `profile/space-<id>/` (per-Space Chromium profile), `keymap.json`
(overrides `crates/baa-ui/keymaps/macos.json` or `windows.json`).

## Layout

| Crate | Owns |
|---|---|
| `crates/baa` | app entry, window, single-instance lock, open-URL forwarding |
| `crates/baa-ui` | GPUI views: sidebar, command bar, theme, motion, keymap actions |
| `crates/baa-web` | the `WebHost` / `PageHandle` contract and its CEF implementation |
| `crates/baa-model` | Spaces, tabs, settings, SQLite persistence |
| `crates/baa-privacy` | Chromium switches, host blocklist |
| `vendor/gpui-cef` | vendored CEF-in-GPUI host (native page views, focus, keyboard routing) |
| `xtask` | bundle, sign, notarize, CEF bump helpers |

## Keys (macOS; Ctrl on Windows)

⌘T command bar / new tab · ⌘L address · ⌘S sidebar · ⌘W close · ⌘⇧T reopen · ⌃⇥ next tab ·
⌘1–8 tab n · ⌘9 last · ⌃⌥→/← next/prev Space · ⌘D pin · ⌘F find · ⌘⇧N incognito Space ·
⌘Y history · ⌘⇧J downloads · ⌘P print · ⌥⌘U view source · ⌘⇧R reader mode · ⌘, settings.
Everything is remappable in `keymap.json` (GPUI's keymap format). Two-finger swipe across the
sidebar or address bar switches Spaces; hold (or right-click) Back/Forward for history; double-click
the top bar to zoom the window; double-click a Space's name to rename it. Right-click the sidebar for
New Folder; right-click a tab to favourite, pin or file it; drag tabs onto a folder or the strip above
the Space name.

## Third-party notices

CEF and Chromium are BSD-3; GPUI is Apache-2.0; gpui-cef is MIT OR Apache-2.0. A `CREDITS.html`
with all notices is part of the release milestone.

## Licence

MIT OR Apache-2.0, at your option.

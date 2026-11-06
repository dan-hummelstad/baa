# Phases 5–6: delete CEF, build the release, ship

Status: implemented 2026-11-06 (code, workflows, docs). Not yet run: the parity checklist (5.0), a release chain, a tag, notarisation. The Windows build is removed, the push trigger on `chromium/**` is not added (release engines start by hand and from `helium-bump.yml`), the licence is GPL-3.0-only, Helium's codec setting is kept; the `cef-final` tag is made by the manager.

Plan for phases 5 and 6 of `PLAN-chromium.md`, written 2026-11-06. Phase 5 starts when M1
(`PLAN-chromium-phase3.md`) and M2 (the delegate surface) are done and the core build
(`cargo build -p baa --features core`) has every feature the CEF build has (the parity list in step 5.0).

**Phase 5 is done when** the repo has no CEF: no `vendor/gpui-cef`, no `cef` crate, no `baa_helper`,
no CEF steps in CI, no `core` feature (BaaCore is the only backend), and one documented command builds
and runs baa on BaaCore. **Phase 6 is done when** a `v*` tag produces a Developer ID signed,
notarised DMG on GitHub Releases from CI, `helium-bump.yml` opens engine bump PRs, and the privacy
test passes on the BaaCore build in CI.

## Facts this plan rests on (checked 2026-11-06)

| Fact | Source |
|---|---|
| A `chromium.yml` release build has never finished. The old split runs (phases 2–10, Oct 31–Nov 2) lost their work at each hand-off (the tar mtime bug); phase 9 ended at 26,188 done / 52,496 remaining. The real time of an official build on the free runners is not known. | `gh run list --workflow chromium.yml`, run 41389352363 log |
| A dev (component) chain from nothing: 92,972 steps, 4 phases, about 16.6 h on `xcode-27` (image `xcode-27-arm64`). The tree artifact is 4.5 GB (`chromium-tree`, newer runs `chromium-tree-xcode27.2`); the local `out/Default` is 7.5 GB. | run 41838189804 |
| When a release chain finishes, `chromium-phase.yml` uploads **nothing**: "Pack tree" and "Upload tree" run only for `mode == 'dev'`. A release run leaves no output. | `.github/workflows/chromium-phase.yml` |
| The repo is public, so standard runners cost nothing. An artifact is kept for at most 90 days. | `gh api repos/{owner}/{repo}` |
| No repo secrets are set (`total_count: 0`), `release.yml` has never run, and the repo has no GitHub Releases. The dev Mac has `Developer ID Application: DANIEL JOHN HUMMELSTAD (LJNQV2DKTV)` and an Apple Development identity. | `gh api .../actions/secrets`, `security find-identity` |
| Windows runs WebView2 through `wry` in `vendor/gpui-cef/.../platform/wry` (441 lines), not CEF. No Windows build has been released. | README "Status", gpui-cef `Cargo.toml` |
| The CEF build starts Chromium with `--use-mock-keychain` (`handlers.rs:46`), so the cookies in `<data>/profile/space-*` are encrypted with the mock key, not a Keychain key. | `vendor/gpui-cef/.../cef/handlers.rs` |
| Chromium has its own signer. Target `chrome/installer/mac` copies `sign_chrome.py`, `signing/`, `pkg-dmg` and the entitlement plists to `out/Default/baa Packaging/`. `rename-framework-baacore.patch` already points `config.framework_dir` at `BaaCore.framework`. Helium's `chromium-signing.patch` adds the Sparkle parts and exports notarised apps when `--disable-packaging` is set. Helium's `build.sh` builds `chrome/installer/mac` and then runs `sign_and_package_app.sh`. | `chromium/helium-macos/`, `chrome/installer/mac/signing/parts.py` in `/Users/Shared/hb` |
| The GN-built `baa.app` has `CFBundleIdentifier me.baa.browser`, version `0.20.3.1` (Helium's), `LSMinimumSystemVersion 13.0`, and the camera/microphone/location usage strings in localised `InfoPlist.strings`. It is 216 MB (component). Helium.app is 342 MB, Arc.app 871 MB. | `plutil -p out/Default/baa.app/Contents/Info.plist` |
| Vendored Helium is 0.20.3.1 (Chromium 156.0.8078.79). Upstream released 0.21.1.1 on 2026-11-05. | `chromium/helium/*.txt`, `gh api repos/imputnet/helium-macos/releases` |

Entitlements as signed today (`codesign -d --entitlements -`):

| Part | Chromium signer (`parts.py`) | Helium 0.21 (shipped) | Arc |
|---|---|---|---|
| outer app | `app-entitlements.plist`: audio-input, bluetooth, camera, print, usb, location, photos; full hardened runtime (kill, restrict, library-validation, runtime) | same, plus application-identifier, `web-browser.public-key-credential`, keychain-access-groups (needs Helium's provisioning profile) | `web-browser.public-key-credential`, keychain-access-groups |
| Helper (Renderer) | `allow-jit`; runtime + kill + restrict, **no** library-validation | allow-jit | allow-jit |
| Helper (GPU) | `allow-jit`; same flags as Renderer | allow-jit | allow-jit |
| Helper (plain), Alerts, crashpad, app_mode_loader, web_app_shortcut_copier | none; full hardened runtime | plain Helper: `disable-library-validation` | plain Helper: `disable-library-validation` |
| BaaCore.framework, `Libraries/*.dylib` | signed, no options | — | — |

baa takes Chromium's signer as it is. It already knows every nested part, including the new Aperitif
helpers. The plain Helper's `disable-library-validation` in Helium and Arc is for code loaded at
runtime (for example a CDM). baa loads no such code, so baa does not add it. If a utility process
fails to load something when signed, add it to that part only.

## Decisions

| # | Question | Decision |
|---|---|---|
| 1 | Windows | **Remove it in phase 5.** It runs WebView2, not Chromium, and has no extensions. It was never released, and keeping it means keeping a second `WebHost` implementation. Before the deletion commit, tag the last commit `cef-final` so the wry code can be found again. `crates/baa-ui/keymaps/windows.json` and the platform-neutral code stay for the later `//baa` port to Windows. README and PLAN say macOS only until then. (Open question 1.) |
| 2 | Architectures | arm64 only. An x86_64 build would double a multi-day chain. Add it when someone asks. |
| 3 | Release engine | `chromium.yml mode=release` builds the targets `chrome chrome/installer/mac` with Helium's official args, `symbol_level=0` and no `cc_wrapper`. It uploads `baa-engine-<T>`, where `<T>` = `git rev-parse HEAD:chromium` (the tree hash of everything that goes into the engine: Helium, patches and `//baa`, including `baa.h`). The artifact holds `baa.app` (unsigned, Rust-less), `baa Packaging/` and `engine.json`, and is kept for 90 days. |
| 4 | Which engine a release uses | Exactly `baa-engine-<T>` for the tagged commit's `<T>`. Equal tree hashes mean the same `baa.h` ABI and the same patches, so no version matching is needed. If no such artifact exists, the release fails and the message says which `chromium.yml` run to start. |
| 5 | When release engines build | On `push` to `main` with paths `chromium/**` (plus manual dispatch and `helium-bump.yml`). `concurrency: chromium-release`, `cancel-in-progress: false`: one run at a time, and only the newest waiting run is kept. (Open question 3.) |
| 6 | Signing | `baa Packaging/sign_chrome.py --identity "$MACOS_SIGN_IDENTITY" --disable-packaging`, after the Rust binary replaces `Contents/MacOS/baa`. It signs inside-out (dylibs, crashpad, helpers with their entitlements, app_mode_loader, framework, then the app with `app-entitlements.plist`) with `--timestamp` and hardened runtime, then runs `codesign --verify --deep --strict`. The outer app's library validation holds because BaaCore has the same Team ID. `xtask/entitlements.plist` (CEF's allow-unsigned-executable-memory and disable-library-validation on everything) is deleted. |
| 7 | Notarisation, DMG | Keep today's steps: `hdiutil` DMG (format ULMO, as Helium), `codesign` on the DMG, `xtask notarize` (notarytool `--wait`, status gate, staple). The app inside relies on the ticket of the stapled DMG (today's ponytail stays). |
| 8 | Version | `CFBundleShortVersionString` = `CFBundleVersion` = the workspace version in `Cargo.toml`. `xtask bundle` writes it. `release.yml` fails if the tag (`v0.3.0`) does not equal it. The engine is shown in the release notes ("Chromium 156.0.8078.79, Helium 0.20.3.1, engine `<T:8>`", from `engine.json`), not in the version. That replaces §13's `+cef<ver>`. The updater is off (`enable_updater=false` → `updater_no_updater.cc`), so Chromium never reads the outer app's version. |
| 9 | Dev build | `cargo bundle` absorbs `crates/baa/core.sh` (from M1). It needs an engine directory. A component engine (the dev tree) bundles in place as `<out>/baa Dev.app` (rpath). An official engine (a downloaded `baa-engine-*`) bundles to `target/bundle/baa.app`. So Rust-only work needs no Chromium tree. |
| 10 | Profile data | One move at start-up: `<data>/profile/space-*` → `<data>/chromium/space-*`. Cookies do not survive: they were encrypted with the mock key, and Chromium drops cookies it cannot decrypt, so users sign in to sites once more. Extensions and their storage, localStorage, IndexedDB and history do move. |
| 11 | Licence | GPL-3.0 (PLAN-chromium.md: Helium is GPL-3.0, "baa is open source under GPL-3.0"). Root `LICENSE`, workspace `license`, README. (Open question 4: `-only` or `-or-later`.) |
| 12 | In-app updates | None in phase 6 (no Sparkle, no updater). Users download the next DMG. Helium's `enable_sparkle` path exists for later. |
| 13 | Passkeys entitlement | Not in phase 6. `web-browser.public-key-credential` needs an Apple provisioning profile (`include_branded_entitlements`, as Helium does). This is follow-up work. |

## Phase 5: delete CEF

### 5.0 Parity gate (before any deletion)

Run the core build (`cargo bundle` per 5.4, a component engine is fine) through the README's feature
list and mark each one working: tabs, Spaces with isolated storage (cookie check across two Spaces), keymap, command
bar, de-Googling switches, Peek, lil-baa (link from another app), favourites, folders, Library
(downloads kept across launches, archived tabs), downloads button, history (⌘T, ⌘Y), permission prompts
and per-site settings, page dialogs, print, view source, reader mode, Picture in Picture, translate,
mute, Settings (⌘,), Arc and history import, extensions (install uBlock Origin Lite and 1Password
from the Web Store; 1Password signs in and fills), extension popups and toolbar, chrome://settings
sheet, DevTools docked, context menus, ⌘C/⌘V/⌘A in pages, IME (Japanese), 120 Hz trackpad scroll,
⌘Q with no hang. Write the result as a checklist in the PR that deletes CEF. Anything that fails
stops phase 5.

### 5.1 Deletion list

| Item | Action |
|---|---|
| `vendor/gpui-cef/` (whole directory, incl. its README, wry/WebView2 backend, `nsapp.rs`, `pump.rs`, `chrome_window.rs`, the tabs shim, `ChromeTabClient`, hidden tab windows, Origin-null fix, hidden `chrome://extensions` page, ⌘C routing) | delete |
| `crates/baa-web/src/cef_host.rs` | delete, after moving its non-CEF functions (5.2) |
| `crates/baa/src/bin/helper.rs`, `[[bin]] baa_helper`, `[package.metadata.cef.bundle]` in `crates/baa/Cargo.toml` | delete |
| `cef` dependency in `crates/baa/Cargo.toml` (the `[target.'cfg(any(macos, windows))']` section), `xtask/Cargo.toml`; `semver` in xtask (only `app_version` used it) | delete |
| Root `Cargo.toml`: `cef = "154"`, `gpui-cef = {…}`, member `vendor/gpui-cef/crates/gpui-cef`, the comment "This rev is the one vendor/gpui-cef CI'd against", "CEF: major == Chromium major" | delete; the gpui pin comment says only "one rev for the workspace" |
| Member `crates/baa-chromium` | already gone at the end of M1; check that it is |
| `crates/baa-web` feature `core` and every `#[cfg(feature = "core")]`; `crates/baa` feature `core`; the CEF `main` in `crates/baa/src/main.rs` | delete; the core `main` becomes the only one. `crates/baa/src/main.rs` and `crates/baa-web` are `#[cfg(target_os = "macos")]` only; on other OSes `main` is `compile_error!("baa runs on macOS only until the //baa Windows port")` |
| `crates/baa/core.sh` (from M1) | delete; `cargo bundle` does its job (5.4) |
| `xtask/cef_current.py` and its test in `xtask/test_scripts.py` | delete (the test file keeps the privacy tests) |
| `xtask/src/main.rs`: `app_version`, both `platform_bundle`s, `WIN_MANIFEST`, `mac::sign`'s CEF walk | replace (5.4); `sign <file>` stays as plain `codesign --timestamp` for the DMG; `notarize` stays |
| `xtask/entitlements.plist` | delete (Chromium's signer has its own) |
| `.cargo/config.toml` `[env] CEF_PATH` and its comment | delete; keep `[alias] bundle` |
| `~/.local/share/cef` on the dev Mac, `cargo install export-cef-dir` | stop using them; say so in the PR (no repo change) |
| `.github/actions/setup/action.yml`: setup-python + ninja (cef-dll-sys's CMake), `msvc-dev-cmd`, the `link.exe` removal, the CEF version step and cache | delete; keep sccache + `rustup toolchain install` |
| `.github/workflows/ci.yml` Windows matrix entries, `xtask bundle` + unsigned bundle upload | replace (6.4) |
| `.github/workflows/release.yml` `windows` job, the six `AZURE_*` secret names in the header | delete |
| `cef-bump.yml` | already gone; delete the PLAN.md §13 text that describes it |
| `baa-privacy`: `ResourceType`, `block_browser_request` + tests, `PREFS_OFF` | delete **if** M2 dropped the request filter and prefs-off (PLAN-chromium-phase3.md M2 "Request filter / prefs off": "drop it if the CI proxy test passes on Helium's patches alone"); else keep only what core_host calls. The crate docs stop saying "stock CEF Alloy". `SWITCHES`, `merge_disabled_features`, `PROXY_ALLOWLIST` stay. |
| `WebHost` methods that only existed for CEF's workarounds (candidates: `show_chrome_page`'s hidden page, `create_popup` if M2 popups are `ExtensionViewHost`s) | delete where M2 left them as stubs the UI no longer needs; the baa-ui orchestrator removes the calls |
| CEF wording in code comments: `crates/baa-ui/src/{mock,sidebar,browser,extensions,prompts,command_bar,tests}.rs`, `crates/baa-model/src/lib.rs`, `crates/baa-privacy/src/*.rs`, `vendor/gpui_apple/Cargo.toml` ("CEF's native page view") | each crate's owner rewrites it to "the page's native view" / "Chromium"; no code change |
| `Cargo.lock` | regenerate; `cef`, `cef-dll-sys`, `wry`, `webview2-com*`, `windows*` (if nothing else needs them) drop out |

`vendor/gpui_apple` stays: the punch-hole surface is how BaaCore pages show too.

### 5.2 Moves into baa-web

M1 copied most of these. Phase 5 makes sure nothing still comes from gpui-cef or `cef_host.rs`:
- Into `crates/baa-web/src/lib.rs` as plain types: `DialogKind`, `Download`, `DownloadState`,
  `MenuItem`, `Permission`, `PermissionState`, `Error` (copied from gpui-cef's `lib.rs`; derive the same
  traits; keep the variant names so baa-ui does not change). Replace the `pub use gpui_cef::{…}` line.
- Into `crates/baa-web/src/core_host/profiles.rs` (one file): `profile_dir`, `queue_delete`,
  `purge_deleted_profiles`, `share_native_messaging_hosts`, `installed_extensions`, and their tests
  from `cef_host.rs` (`tests` module at `cef_host.rs:864-946`).
- `core_host/menu.rs` and `native.rs` are already copies. Remove any `// copied from gpui-cef`
  "keep in sync" notes.
- `crates/baa-web/src/lib.rs` crate doc: "BaaCore host. The UI codes against `WebHost`/`PageHandle`
  only; no C ABI types cross this crate's boundary."

### 5.3 Data migration

In `profiles.rs`, `pub fn migrate_cef_profiles(data: &Path)`, called by `main` before
`purge_deleted_profiles(<data>/chromium)`:
1. If `<data>/profile` does not exist, return.
2. Run `purge_deleted_profiles(<data>/profile)`, so Spaces deleted under CEF stay deleted.
3. For each `<data>/profile/space-*` directory whose name does not exist in `<data>/chromium/`,
   `fs::rename` it there (same volume). Skip names that exist (a dev's M1 test data wins).
4. `fs::remove_dir_all(<data>/profile)`. Log one line per moved Space.

CEF's `cache_path` per Space is already a Chrome profile directory (`Preferences`, `Cookies`,
`Extensions/`, `Local Extension Settings/`, …). Phase 3 #9 opens `space-<id>` under
`--user-data-dir`, so the layouts match. CEF 154 data moving forward to Chromium 156 is a supported
profile upgrade. CEF's root files (`Local State`, caches) are dropped. Chromium writes a new
`Local State`.

`// ponytail:` cookies encrypted with the mock key are lost (decision 10). Re-encrypting them
would need the AES/PBKDF2 code of OSCrypt in Rust. That is not worth it: no release has
shipped, so only builds from source have CEF data. Delete the function in the first release after
v0.2.

Tests (tempdir, as `cef_host.rs` tests do): a Space moves with its files; an existing
`chromium/space-1` is kept; a queued delete under `profile/` is not migrated; a second call does nothing.

### 5.4 Dev build: one command path

`xtask bundle` becomes:

```
cargo bundle [--release] [--engine DIR] [--run [chromium args…]]
```

- Engine `DIR`: `--engine`, else `$BAA_ENGINE`, else `/Users/Shared/hb/helium-macos/build/src/out/Default`
  if it has `baa.app`, else `target/engine`. If none exists, fail with the two ways to get one (below).
- `cargo build -p baa [--release]`.
- Component engine (`DIR/args.gn` contains `is_component_build = true`): the app is
  `DIR/baa Dev.app`. Refresh it with `ditto` from `DIR/baa.app` when that is newer (the rpath to
  `out/Default` needs it there). Official engine: the app is `target/bundle/baa.app`, a fresh
  `ditto` of `DIR/baa.app` (old helpers and signatures must not stay).
- Copy the binary to `Contents/MacOS/baa`. Set `CFBundleShortVersionString` and `CFBundleVersion`
  to `CARGO_PKG_VERSION` with `plutil -replace`.
- Unless `--no-sign`: `codesign --force --sign <id> --identifier me.baa.browser <app>` (outer app only,
  as `spike.sh`). `<id>` is `$BAA_SIGN_IDENTITY`, else the first "Apple Development" identity, else
  `-`. A stable identity keeps the Keychain's "Always Allow" for "baa Storage Key".
- Print the app path. `--run` runs `Contents/MacOS/baa` in the foreground with the rest of the args,
  so the logs show in the terminal.

There are two ways to get an engine (README, 7.1):
- **C++ / `//baa` work:** download the newest `chromium-tree-xcode<ver>` from a successful
  `chromium dev` run, unpack it at `/Users/Shared/hb` (same Xcode as the runner, see
  `chromium.yml`'s `xcode` input), then `chromium/local-build.sh`.
- **Rust-only work:** `gh run download <run> -n baa-engine-<T> -D target/engine` from the newest
  successful `chromium` release run whose `<T>` matches `git rev-parse HEAD:chromium`. Then
  `zstd -dc target/engine/engine.tar.zst | tar -xf - -C target/engine`. This is about 350 MB, needs
  no Xcode match and gives a relocatable non-component app.

## Phase 6: release build, signing, CI

### 6.1 `chromium.yml` / `chromium-phase.yml`

Keep every hand-off guard in `chromium-phase.yml`: `.siso_fs_state`, sub-second `.siso_deps`
mtime, and the remaining-steps progress check. Changes:

1. `chromium.yml` `on:` add `push: { branches: [main], paths: ["chromium/**"] }`. Every
   `with:` uses `${{ inputs.mode || 'release' }}`, `${{ inputs.runner || 'xcode-27' }}` and
   `${{ inputs.xcode || '27.2' }}` (a push has no inputs). Add
   `concurrency: { group: "chromium-${{ inputs.mode || 'release' }}", cancel-in-progress: false }`.
   `run-name` shows `release` for pushes.
2. GN args in one place. Move the CI-only edits now in "Prepare Helium sources" into
   `chromium/helium-macos/devutils/shared.sh` `write_gn_args`, under a `# baa:` comment. That means
   dropping `cc_wrapper` (always: no cache backend), and for `dev` `enable_precompiled_headers=false`
   and symbol levels 0. Add `symbol_level=0` for `release` too: no dSYMs, so less disk and no
   dsymutil step. `// ponytail:` set `symbol_level=1` and upload the dSYMs as a second artifact
   when crash reports from users matter. Then CI and `local-build.sh` produce the same `args.gn`.
3. Build step: targets `chrome` for dev, `chrome chrome/installer/mac` for release (the packaging dir,
   `dmg_tool`, `hfs_tool`).
4. New steps when `status == 'finished' && mode == 'release'`:
   ```
   - name: Pack engine
     working-directory: /Users/Shared/hb/helium-macos/build/src/out/Default
     run: |
       T=$(git -C "$GITHUB_WORKSPACE/baa" rev-parse HEAD:chromium)
       echo "ENGINE=baa-engine-$T" >> "$GITHUB_ENV"
       jq -n --arg t "$T" --arg sha "$GITHUB_SHA" \
         --arg chromium "$(cat /Users/Shared/hb/helium/chromium_version.txt)" \
         --arg helium "$(cat "$GITHUB_WORKSPACE/baa/chromium/HELIUM_RELEASE")" \
         --arg abi "$(sed -n 's/^#define BAA_ABI_VERSION //p' /Users/Shared/hb/baa/baa.h)" \
         '{tree:$t, commit:$sha, chromium:$chromium, helium:$helium, abi:$abi}' > engine.json
       tar -cf - baa.app "baa Packaging" engine.json | zstd -T0 -10 -o /Users/Shared/engine.tar.zst
   - uses: actions/upload-artifact@v4
     with: { name: "${{ env.ENGINE }}", path: /Users/Shared/engine.tar.zst, retention-days: 90, compression-level: 0 }
   ```
   (`tar` keeps the framework's `Versions/Current` symlinks. `upload-artifact` on a directory would not.)
   `chromium/HELIUM_RELEASE` is a new one-line file holding the vendored Helium release tag
   (`0.20.3.1`), written by hand now and by `helium-bump.yml` later.
5. A final job `privacy` in `chromium.yml`: `needs: [phase_01 … phase_12]`,
   `if: always() && inputs.mode != 'dev' && contains(needs.*.outputs.status, 'finished')`,
   `runs-on: macos-15`. It uses the `privacy` composite action (6.4) with this run's id. A release
   engine that sends traffic to a host outside the allowlist fails its run.
6. Release base trees (`base_run` for release) are deferred. Turn them on if measured release
   chains are longer than about 2 days. A `//baa`-only change would then need a re-link instead of a
   full rebuild.

### 6.2 Assembly, signing, notarisation (what `release.yml` runs)

```
engine/                          unpacked baa-engine-<T>
  baa.app/Contents/MacOS/baa     ← Rust binary (cargo bundle --release --engine engine --no-sign)
  baa.app/Contents/Frameworks/BaaCore.framework/{BaaCore, Helpers/…, Libraries/…}
  baa Packaging/sign_chrome.py
```

`cargo bundle --release --engine engine --no-sign` writes the binary and versions into
`target/bundle/baa.app`. Copy `engine/baa Packaging` next to it, because the signer's `--input` must
hold the app and the Packaging directory. Then:

```
python3 "target/bundle/baa Packaging/sign_chrome.py" --input target/bundle \
  --output target/bundle/signed --identity "$MACOS_SIGN_IDENTITY" --disable-packaging
APP=target/bundle/signed/stable/baa.app    # path per Helium's sign_and_package_app.sh; check on the first run
python3 xtask/privacy_test.py "$APP" proxy-allowlist.txt --seconds 60   # the signed app, as shipped
mkdir dmg && ditto "$APP" dmg/baa.app && ln -s /Applications dmg/Applications
hdiutil create -volname baa -srcfolder dmg -format ULMO "baa-$VER-macos-arm64.dmg"
cargo xtask sign  "baa-$VER-macos-arm64.dmg"
cargo xtask notarize "baa-$VER-macos-arm64.dmg"
```

The signing order inside the signer is nested parts first, then `BaaCore.framework`, then the app.
After signing, check that `codesign -d --entitlements - <part>` matches the "Chromium signer"
column of the entitlements table, and that `codesign -dv` on the app shows
`flags=…(kill,restrict,library-validation,runtime)` and `TeamIdentifier=LJNQV2DKTV`.

### 6.3 `release.yml`

```
on: push: tags: ["v*"]
permissions: { contents: read, actions: read }
jobs:
  version (ubuntu): version=${GITHUB_REF_NAME#v}; fail unless it equals
      `cargo metadata --no-deps --format-version 1 | jq -r '.packages[]|select(.name=="baa").version'`.
  engine (ubuntu): T=$(git rev-parse HEAD:chromium);
      id=$(gh api "repos/$R/actions/artifacts?name=baa-engine-$T" -q '[.artifacts[]|select(.expired|not)][0].workflow_run.id');
      empty → ::error::"no engine for chromium/ tree $T; run: gh workflow run chromium.yml -f mode=release --ref $GITHUB_REF_NAME, then re-run this workflow".
      outputs: run=$id, name=baa-engine-$T
  macos (macos-15, needs version+engine):
      checkout; ./.github/actions/setup; download-artifact (name, run-id, github-token) → engine/;
      unpack; cargo run -p baa-privacy --example proxy_allowlist > proxy-allowlist.txt;
      setup-python 3.13 + pip install mitmproxy;
      apple-actions/import-codesign-certs (MACOS_CERT_P12_BASE64, MACOS_CERT_PASSWORD);
      6.2 with MACOS_SIGN_IDENTITY, APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD;
      upload the dmg + engine.json.
  publish (ubuntu, needs version+macos): gh release create … --title "baa $VER"
      --notes "Engine: Chromium <chromium>, Helium <helium>, engine <tree:8>. Source: <repo>/tree/$TAG" --generate-notes
```

Secrets the user adds once (names unchanged from today's header): `MACOS_CERT_P12_BASE64`,
`MACOS_CERT_PASSWORD`, `MACOS_SIGN_IDENTITY` (`Developer ID Application: DANIEL JOHN HUMMELSTAD
(LJNQV2DKTV)`), `APPLE_ID`, `APPLE_TEAM_ID` (`LJNQV2DKTV`), `APPLE_APP_PASSWORD`.

**Tag → release, end to end:** merge to `main` → if `chromium/` changed, a release chain starts (one
to two days, not measured yet) → `privacy` passes in that run → tag `vX.Y.Z` (the `Cargo.toml`
version) → `release.yml` takes `baa-engine-<T>`, adds the Rust binary, signs, runs the privacy test,
notarises and publishes (about 30 min). A tag pushed while the engine is still building fails fast.
Re-run it when the chain is done.

### 6.4 `ci.yml` and the `privacy` action

`.github/actions/privacy/action.yml` (composite; inputs `run-id`, `artifact`):
download the engine artifact, unpack it to `target/engine`, `cargo bundle --release --engine target/engine`
(ad-hoc or no identity on runners → `-`), run `proxy_allowlist`, then setup-python 3.13 and
`pip install mitmproxy`. Then run `python3 xtask/privacy_test.py target/bundle/baa.app proxy-allowlist.txt --seconds 60 --netlog netlog.json`
and upload `netlog.json` on failure.

`xtask/privacy_test.py` changes: start the app with `BAA_DATA_DIR=<tmp>` (a clean profile, M1's
variable) and add `--use-mock-keychain`. A Keychain prompt on a runner would hang the test, and
Keychain use is not network traffic. The rest stays: the app still takes `-`-prefixed args as
Chromium switches and the page URL as a tab.

`ci.yml`:
```
jobs:
  scripts (ubuntu): python3 -m unittest discover -s xtask -p 'test_*.py'
  build (macos-15):
    setup; cargo test --release --workspace; cargo clippy --release --workspace --all-targets -- -D warnings
    - id: engine   # same lookup as release.yml's engine job; empty is fine here
    - if: steps.engine.outputs.run != ''
      uses: ./.github/actions/privacy
    - if: steps.engine.outputs.run == ''
      run: echo "::notice::no release engine for chromium/ tree $T yet; the privacy test runs in that chromium.yml run"
```
`workflow_dispatch:` is added so `helium-bump.yml` can start it on its PR branch. No Chromium tree
is needed: `baa-web` loads BaaCore with `dlopen` at run time, so `cargo test`/`clippy` compile and
link without it. `core_host` tests use the fake `Core` from M1. Pages are not tested in `ci.yml`.

The engine lookup is about 6 lines in both `ci.yml` and `release.yml`. Put it in
`.github/actions/engine/action.yml` (output `run`, `name`) so it exists once.

### 6.5 `helium-bump.yml`

```
on: { schedule: [{cron: "17 6 * * 1"}], workflow_dispatch: }   # weekly
permissions: { contents: write, pull-requests: write, actions: write }
job (ubuntu, fetch-depth: 0):
  NEW=$(gh api repos/imputnet/helium-macos/releases/latest -q .tag_name)       # e.g. 0.21.1.1
  [ "$NEW" = "$(cat chromium/HELIUM_RELEASE)" ] && exit 0
  B=bot/helium-$NEW; exit 0 if origin/$B exists (a PR is open already)
  HELIUM=$(gh api "repos/imputnet/helium-macos/contents/helium-chromium?ref=$NEW" -q .sha)
  git subtree pull --prefix chromium/helium https://github.com/imputnet/helium.git "$HELIUM" --squash
  git subtree pull --prefix chromium/helium-macos https://github.com/imputnet/helium-macos.git "$NEW" --squash
  git rm -q --cached chromium/helium-macos/helium-chromium chromium/helium-macos/.gitmodules 2>/dev/null || true
  echo "$NEW" > chromium/HELIUM_RELEASE; commit; push $B
  gh pr create --title "chromium: Helium $NEW" --body "<release notes link>; merge when chromium.yml (release) and ci.yml are green"
  gh workflow run chromium.yml --ref "$B" -f mode=release
  gh workflow run ci.yml --ref "$B"
```
- A subtree merge conflict (with baa's branding edits in Helium's patches) fails the job. Its log
  links to the steps for taking a Helium release by hand in PLAN-chromium.md. A baa patch that no
  longer applies fails the chromium run's phase 1 within about 10 minutes (`prepare_sources`).
  Either way a human takes over on the branch.
- PRs opened by `GITHUB_TOKEN` do not trigger workflows, so the job dispatches both workflows
  itself (`workflow_dispatch` is allowed for `GITHUB_TOKEN`). The runs report on the branch's head
  commit, so the PR shows them.
- No auto-merge. Engine bumps get a human look, and the chain takes days anyway.
- Dev trees: a bump changes the Chromium version, so the next `chromium dev` run builds from
  nothing (the base picker matches `chromium_version.txt`). Start one after merging.

### 6.6 Licence and notices

- Root `LICENSE`: GPL-3.0 text. Set `[workspace.package] license` to `"GPL-3.0-only"` (open question 4).
- Chromium's third-party notices ship inside the engine as `chrome://credits` (built from
  `about_credits.html`).
- Rust crates' notices: `cargo about generate` in `release.yml` (a CI tool, not a runtime
  dependency) writes `Contents/Resources/rust-credits.html` before signing. `// ponytail:` this file
  is not linked from the UI. Add a Settings link if someone asks.
- Release notes link the source at the tag (GPL offer).

## 7. Docs

### 7.1 `README.md`
- Intro bullets. **Engine:** "baa's own Chromium build: Helium's patch set (de-Googled, GPL-3.0)
  plus `//baa`, a layer that lets the Rust UI drive real Chrome browsers and tabs
  (`BaaCore.framework`)". Keep the native-view sentence. **Privacy:** Helium patches out
  Google services; baa's switches cover the rest; CI checks it. **Targets:** "macOS (Apple
  silicon). Windows later." Delete "The engine pin is bumped automatically by a daily GitHub Action"
  and replace it with "Helium releases are picked up by a weekly bot PR."
- **Status:** delete "Windows currently runs on WebView2 …". Add the M2 items that are now real.
- **Build:** rewrite as two paths (5.4): the Rust-only engine download and the Chromium tree +
  `chromium/local-build.sh`. Then `cargo bundle --run`. Prerequisites:
  `brew install coreutils zstd gnu-tar rsync`, `pip3 install --user pillow requests` (tree path
  only), Xcode `27.2`. Keep the `--release` smoothness note and the Documents-access note (now
  solved by the stable identity).
- **Environment variables table:** delete `BAA_CEF_LOG`. Add `BAA_DATA_DIR` (separate data dir),
  `BAA_ENGINE` (engine dir for `cargo bundle`), `BAA_SIGN_IDENTITY`, and Chromium's own
  `--enable-logging=stderr --v=1` as args to `cargo bundle --run`.
- **App data:** `chromium/space-<id>/` (per-Space Chrome profile) replaces `profile/space-<id>/`,
  plus `chromium/Crashpad/` (local crash dumps, never uploaded). Delete the `%APPDATA%` and
  `windows.json` mentions. One line on the CEF migration (sign in to sites once).
- **Layout table:** `crates/baa-web`: "the `WebHost` contract and its BaaCore implementation (C ABI
  in `chromium/baa/baa.h`)". Delete the `vendor/gpui-cef` row. Add `vendor/gpui_apple` (gpui's macOS
  backend with the punch-hole surface), `chromium/` (vendored Helium + helium-macos, baa's patches,
  `//baa`, `local-build.sh`) and `xtask` (bundle, sign the DMG, notarise).
- New short **Release** section: tag = `Cargo.toml` version; which engine is used; the secrets.
- **Keys:** delete "Ctrl on Windows".
- **Third-party notices / Licence:** GPL-3.0; Chromium/Helium notices at `baa://credits` (the
  scheme name after substitution; check it), Rust crates in `rust-credits.html`. Delete the CEF
  and gpui-cef licences.

### 7.2 `CLAUDE.md`
- Build: delete the `CEF binaries: CEF_PATH …` bullet. Change "(CEF wrapper + gpui)" to "(gpui)".
  Change "`cargo bundle` then `open target/bundle/baa.app`" to "`cargo bundle --run` (needs an
  engine, README "Build")". Add: "Never run two Chromium builds (`pgrep -x siso`); never remove the
  hand-off guards in `chromium-phase.yml`."
- Ownership: `crates/baa-web`: "BaaCore host (`WebHost` in lib.rs is the contract)". Delete
  "+ `vendor/gpui-cef`" and the `crates/baa-chromium` line. Add `chromium/baa`, `chromium/patches`:
  engine orchestrator; `chromium/helium*`: changes only through a Helium bump or the engine
  orchestrator. Add `vendor/gpui_apple`: baa-web orchestrator.

### 7.3 `PLAN.md` (the UI spec stays; the engine parts point to PLAN-chromium.md)
- Header line 4: "Targets: macOS (Windows later, via the same `//baa`)".
- Add a note under the title: "The engine is specified in `PLAN-chromium.md`. Sections marked
  *superseded* are kept for history."
- §1: rewrite the paragraph and diagram (`MacOS/baa` + `BaaCore.framework`, page `WebContents`
  NSView under gpui's layer, Helium patch set, weekly bump).
- §2: rows Engine host, De-Googling, Storage isolation, Updates, Extensions → one line each pointing
  to PLAN-chromium.md. Delete the "Prior art to fork: gpui-cef" row and the Windows half of the
  "Frame path" row.
- §4 layout tree: delete `gpui-cef` and the cef pin; add `chromium/`; xtask without the cef bump;
  workflows `ci.yml · chromium.yml · chromium-phase.yml · helium-bump.yml · release.yml`. In the
  text below it, delete "cef" from the dependency list and "CEF and GPUI both pump their own loops".
- §5: mark *superseded*; keep the table for history. Add one line: "Today: Helium's patches,
  `chromium/patches/baa/*`, `baa_privacy::SWITCHES`, checked by the privacy test."
- §6: replace `CefRequestContext`/`root_cache_path` with "one Chrome `Profile` per Space under
  `--user-data-dir=<data>/chromium` (`space-<id>`), ephemeral = OTR profile `baa::<id>`".
- §7: mark *superseded* except Input. Rewrite "Process model" for the BaaCore bundle (6.2 layout).
- §11 milestone 5 row: point to PLAN-chromium phases 5–6.
- §12: "keep all engine types behind `baa-web::WebHost`" (drop "CEF").
- §13: replace the whole section with a summary of 6.1, 6.3, 6.4 and 6.5 (`ci.yml`, `chromium.yml`,
  `helium-bump.yml`, `release.yml`).
- §14: delete risks 2 (Windows OSR) and 4 (cef-rs), rewrite 7 (size: about 350 MB) and 8 (licence:
  GPL-3.0, credits). Add "release chain length" and "security-fix latency" (from PLAN-chromium's
  cost line).
- §15: mark *superseded* (it is milestone 0's day-1 CEF commands).
- Sources: delete the CEF-only links (cef-rs, CEF index, forum, request-context docs, #4057, #2744).

### 7.4 `PLAN-chromium.md`
- Line 3: "Replaces PLAN.md §1, §2 …" is now done; say so with the date. Delete "Until then CEF
  stays the shipping engine."
- Build and CI: add the push trigger, the `baa-engine-<T>` artifact, `release.yml`'s exact-tree
  rule and the specified `helium-bump.yml`. Delete "`cef-bump.yml` is gone; the app keeps its pinned
  CEF until phase 5".
- Phases table: mark 5 and 6 done with dates and link this file. Windows row: "WebView2 was removed
  in phase 5 (tag `cef-final`); Windows returns with the `//baa` HWND port."
- "With CEF gone, runtime stripping … shrinks": state what is left in `baa-privacy`.
- Open questions: 1 is answered by the measured release chain (record the hours). 2 points to this
  file's open question 2.

### 7.5 Others
- `PLAN-chromium-phase2.md`, `PLAN-chromium-phase3.md`: add a status line under the title ("Done
  <date>; the CEF deletions in 'Later deletions' happened in phase 5, `PLAN-chromium-phase5.md`").
  Do not rewrite them.
- `vendor/gpui-cef/README.md`: deleted with the directory.
- `chromium/helium/**/*.md`, `chromium/helium-macos/README.md`, `docs/`: upstream files, unchanged.
- No new README files.

## 8. Steps for implementers

Each step is one PR (or one commit on the phase branch) with its acceptance checks. The owners
follow CLAUDE.md: steps 1–4 belong to the baa-web and integration orchestrators, 5–9 to the
CI/release orchestrator, 10 is shared.

| # | Step | Acceptance |
|---|---|---|
| 1 | **Measure the release chain now** (it does not wait for parity): fix the missing release output (6.1 items 2–4), dispatch `chromium.yml -f mode=release` on `main`. | The chain finishes. `remaining_phaseN` falls each phase. `baa-engine-<T>` is uploaded. Record the wall clock, phases and artifact size in PLAN-chromium.md. Locally: unpack, `cargo bundle --engine target/engine` (with M1's `core` feature until step 3), and the app starts. |
| 2 | Parity gate 5.0 | Checklist in the PR, every item working. |
| 3 | Tag `cef-final`; delete CEF (5.1), move types (5.2), migration (5.3) | `cargo tree -i cef` and `cargo tree -i wry` find nothing. `rg -i 'gpui.cef\|cef_host\|CEF_PATH\|baa_helper' --glob '!PLAN*'` is empty. `cargo test --workspace`, `cargo clippy --workspace --all-targets -D warnings` pass on macOS. Migration tests pass. On the dev Mac, a copy of the real `<data>/profile` migrates: Spaces, extensions and history are there; sites ask to sign in again. |
| 4 | `cargo bundle` (5.4); delete `core.sh` | With the dev tree, `cargo bundle --run` opens baa from `out/Default/baa Dev.app`. With a downloaded engine, it bundles `target/bundle/baa.app`, which starts after it is moved to `/tmp` (relocatable). Run twice: the Keychain asks only the first time. |
| 5 | Setup action, `ci.yml`, `privacy` + `engine` actions, `privacy_test.py` changes (6.4) | On a PR: `build` green. The privacy test runs when an engine for `<T>` exists (no other host than localhost). A PR that edits `chromium/baa` shows the notice instead. Prove that a failure fails the job: on a scratch branch, add `--enable-features=` with something that makes Chromium call home (or delete one switch from `SWITCHES` that the old test caught). The job goes red and uploads `netlog.json`. |
| 6 | `chromium.yml` push trigger, concurrency and `privacy` job (6.1 items 1, 5) | A push touching `chromium/` starts a release chain. A second push while it runs waits, and a third replaces the second. The chain's `privacy` job is green. |
| 7 | Signing and notarisation, run by hand on the dev Mac with the step 1 engine (6.2), before `release.yml` | `codesign --verify --deep --strict` passes. The entitlements match the table. `spctl -a -vvv -t exec` says "Notarized Developer ID". Copy the DMG to another Mac or user account: the first launch shows no Gatekeeper warning, a camera prompt works on a WebRTC test page, and 1Password works. |
| 8 | `release.yml` (6.3); user adds the six secrets | Push tag `v0.2.0-rc.1` (version set to match): a release appears with the DMG and engine notes. Repeat the step 7 checks on the downloaded DMG. A tag whose `<T>` has no engine fails with the message from 6.3. |
| 9 | `helium-bump.yml` (6.5), `chromium/HELIUM_RELEASE` | `workflow_dispatch` with the current pin does nothing. Against the real upstream (0.21.1.1 > 0.20.3.1) it opens a PR and starts both runs. Either its chromium chain is green, or a conflict stops it with a clear log. |
| 10 | Docs (section 7), `LICENSE`, `cargo about` | A reviewer who follows only README "Build" on a clean macOS user account gets baa running (Rust-only path). `rg -n -i '\bcef\b' README.md CLAUDE.md` is empty. Every PLAN.md hit is in a section marked *superseded*. |

Steps 1, 5 and 9 do not depend on parity and can start now. Steps 3–4 wait for step 2. Steps 7–8 need
step 1's engine with the step 3 code (no `core` feature).

## 9. Risks

| # | Risk | Cheapest test | Fallback |
|---|---|---|---|
| 1 | An official (ThinLTO + PGO) chain on the free runner runs out of disk or memory in the final framework link, or takes longer than 12 phases. | Step 1. | `symbol_level=0` is already set; drop PGO (`chrome_pgo_phase=0`); else a paid larger runner or this Mac self-hosted (PLAN-chromium open question 1). |
| 2 | Engine artifacts expire after 90 days without a `chromium/` change, so a tag finds no engine. | — | The release error says what to run. A scheduled monthly release chain if this happens often. |
| 3 | The signer rejects the bundle: Info.plist checks, or the Rust binary's version keys. | Step 7. | `_sanity_check_version_keys` returns early without `KSVersion`, which baa does not set. Fix any other check in `chromium-signing.patch`'s baa twin (`patches/baa/signing.patch`). |
| 4 | Hardened runtime + library validation on the outer app blocks something the Rust process loads (Metal shader compile, a system plugin). | Step 7 launch. | Add only the entitlement that the crash log names, on the outer app, through a `patches/baa` edit to `app-entitlements.plist`. |
| 5 | The privacy test on a runner hangs on a Keychain or TCC prompt, or `--use-mock-keychain` hides a real call. | Step 5. | Mock keychain is local-only crypto, not network. If the job hangs, add `--disable-features` for the prompting feature in the test only. |
| 6 | The migration loses data when `rename` crosses volumes (data dir on another volume). | — | Both dirs are under the same `<data>`, so the volume is the same. If `rename` fails, log it and leave `<data>/profile` in place. Never delete before every move has succeeded. |
| 7 | Helium's subtree pull conflicts every time because of the in-place branding edits. | Step 9 with 0.21.1.1. | Move the branding edits out of Helium's patch files into `chromium/patches/baa/` so upstream files stay pristine (a phase 6 follow-up if step 9 hurts). |
| 8 | Security-fix latency (Helium lag + bump + one or two days of build) is long. | Record it per bump. | A manual `chromium.yml` dispatch on the bump branch at once. Paid runners if it matters. |

## 10. Open questions for the user

1. **Windows:** remove the WebView2 build in phase 5 and keep Windows for the `//baa` port
   (recommended; it was never released and lacks extensions), or move the wry backend into
   `baa-web` as a second `WebHost` and keep shipping a reduced Windows build?
2. **Proprietary codecs:** Helium's flags ship H.264/AAC (`proprietary_codecs=true`,
   `ffmpeg_branding="Chrome"`). Keep them in baa's DMG as Helium does, or turn them off?
3. **Release chains on every `chromium/` push to `main`** (one to two days of free runner time each,
   newest wins), or only on manual dispatch and Helium bumps?
4. **Licence:** `GPL-3.0-only` or `GPL-3.0-or-later` for the repo?

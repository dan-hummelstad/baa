# ADK refactor (from the code-quality review, 2026-11-06)

An Opus review (thermo-nuclear bar) of chromium/baa/* and crates/baa-web/src/core_host* concluded:
**"not yet"**. The architecture is right and M3 needs no redesign. The flat typed C ABI plus Chrome's
own observers covers tab groups, fullscreen, find counts, badges and openPopup without new patches.
The implementation still carries M1/M2 debt that makes every new delegate expensive.

## Findings, ranked

1. **Tab lifecycle uses flags and registries where one model would do** (tab.mm).
   - Overlapping fields: `closing_by_baa`, `closed_sent`, `bare`, `close_bare`, `reported`.
   - Two registries: `Tabs()` and `BareTabs()`.
   - The "ReportClosed + Detach + erase" sequence is copied six times.
   - Global mode flags: `g_creating`, and `g_docking`/`g_front` in devtools.mm.
2. **Pending requests are reinvented per feature.** `Dialogs`, `AppModals`, `Prompts` and `Menus`
   each have their own registry, weak tab, drop and answer handling. Adding one M3 delegate touches
   about 15 places.
3. **The ABI is mirrored by hand twice** (baa-web and harness `ffi.rs`), and core_host decodes enums
   with magic numbers.
4. **DevTools docking is in the wrong layer.** Rust injects JS from inside `CorePage::apply` and gets
   the answer back through a generic `page_message` console channel that only this feature uses.
5. **`core_host.rs` is 1426 lines.**
   - `pages` and `bare` are two maps for the same callbacks.
   - The page view is held both by `Shared.view` and by the `Container`.
   - The `Shared` construction is copy-pasted between pages and the sheet.
6. **`tab.mm` is 1166 lines** with four concerns: profiles, tabs, dialogs, and identity-wrapper pairs.
7. 16 callback null-checks guard an invariant nothing enforces.
8. One-time start-up is scattered, some of it as hidden side effects. The download watch is a global,
   though it belongs to each profile.
9. Close notifications are inconsistent (none on `baa_tab_close`/`baa_profile_close`, sent on quit),
   so Rust compensates with `forget_profile` and `is_quitting`. "Rust may keep a dead handle" is false
   because addresses get reused.
10. Dead code: `PageEvent::ContextMenu`/`ActiveTabLoad`, `Permission::ScreenShare`, `create_popup`'s
    `tab_url`/`tab_title`, `BaaTab::find_text`, the harness `EDIT_*`, and duplicated
    `CGPoint`/`CGRect`.
11. **Windows seam.** The ABI's `void*` view and event are fine. Every `.mm` file imports Cocoa, but
    only a few spots are AppKit. The context-menu hook exists only for macOS.
12. Adoption matches by URL, a deliberate shortcut. If Chrome-made tabs become common, replace it
    with a token.

## Plan

### Now (behaviour-preserving, no ABI change)

1. The harness uses baa-web's `ffi.rs` through `#[path]`. Its own copy is deleted, and `EDIT_*` move
   into the harness's native.rs.
2. Every `BAA_*` constant and C↔Rust enum conversion moves into `ffi.rs`. A test reads `baa.h` with
   `include_str!` and checks the version and the enum values.
3. Delete the dead code from #10, including the UI arms that handle it.
4. Split `core_host.rs` into `dispatch`, `page`, `slot`, `host` and `menu` (rows move there). The
   split is a pure move.
5. Merge `pages` and `bare` into one map. The view lives in `Container`. Add a shared `Shared::open`
   for pages and the sheet. Delete the `chrome_closed` wrapper.
6. C++:
   - CHECK in `baa_install_app` that every callback is set, and delete the guards.
   - Delete the identity-wrapper pairs and `Tabs_Erase`.
   - Move `Window` into `window.h`.
   - Gather one-time setup into `baa::EnsureStarted()`.
   - The download watch hangs off `BaaProfile`.
7. Split `tab.mm` into `profile.mm`, `dialogs.mm` (with `AppModal` from menu.mm and one kind map) and
   `tab.mm`.
8. Tab lifecycle:
   - `Forget(tab, tell)` replaces the six copies.
   - Unregister before destroying.
   - Deletion goes only through `OnWillDetach(kDelete)`.
   - A `ScopedInsertion` replaces the global flags.
   - One registry.
   - This deletes `closing_by_baa`, `closed_sent`, `bare`, `g_creating`, `g_docking` and `g_front`.
9. Fix the handle-lifetime text in baa.h.

### At M3 start (one ABI 5 bump, both sides together)

10. A `requests.mm` registry for every id-answered ask, plus one `request_closed` callback replacing
    `dialog_closed`/`permission_closed`. `PageEvent::Dialog` carries an id, which deletes
    `Shared.dialog`. Each M3 ask then costs one typed ask callback, one typed answer function and one
    feature file.
11. DevTools docking moves into C++: a `side` argument and a typed `devtools_dock` callback. This
    deletes `page_message` and the Rust dock logic.
12. Tab handles become `uint64_t` ids that are never reused. `tab_closed(reason)` is sent exactly once
    per handle, which deletes `forget_profile`. Download ids also come from the shared id counter.

### When Windows starts

13. Portable files become `.cc`, with `platform_mac.mm` and `platform_win.cc`. Add a twin
    context-menu hook in the views delegate. Merge the hook headers into `chrome/browser/baa_hooks.h`.

### Optional

14. Token-based adoption, if Chrome-made tabs become common.

**Target:** after steps 1–10, a new M3 delegate costs about 10 edits with no hand-mirrored layout.

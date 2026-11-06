# Settings: first-party GPUI dialog, skinned WebUI for the rest (draft, 2026-11-06)

Spec, not built. Written against Chromium 156 (Helium tree at `/Users/Shared/hb/helium-macos/build/src`;
paths starting `chrome/`, `components/`, `content/` are in that tree), ABI 4 (`chromium/baa/baa.h`) and
the ADK refactor (`PLAN-adk-refactor.md`; core_host is now `dispatch/page/slot/host/menu`, tab.mm is
`profile/dialogs/tab`). Work starts after the refactor's "Now" steps land.

The user's request: daily pages (baa settings, extensions, the daily part of Chrome settings) are
first-party GPUI. Everything else (flags, internals, about) stays Chrome WebUI in baa's skin. The
command bar autocompletes `baa://` pages. Settings become a dialog with tabs, and they gain the
settings a browser user expects.

## Decisions

| # | Question | Decision |
|---|---|---|
| 1 | Where the dialog lives | An **overlay card in the browser window**, as the current settings card and the command bar are. Pages are native views under gpui's layer, so gpui draws over them. A separate NSWindow would need a second root view, its own focus handling and its own key routing for no gain. In lil-baa, ⌘, opens the dialog in the browser window. |
| 2 | Scope of browser settings | **Global** for behaviour settings (cookies mode, HTTPS, DNS, fonts, downloads, …): baa-model is the source of truth, and baa-web pushes the values to every profile as it opens and again on every change. This is how `PREFS_OFF` works now (`core_host/host.rs:103`). **Per Space** for what is tied to a site's storage: site exceptions, extensions, and clearing data. Arc does it this way. It also needs **no pref getters and no change notifications**. ponytail: a value changed in a Chrome WebUI page is overwritten at the next profile open. The ceiling: none of these prefs has a WebUI route left, except `baa://settings/<unrouted subpage>` (#5). |
| 3 | Site defaults | Global, stored in baa-model as *changed kinds only*. The value shown for an unchanged kind comes from `Permission::chrome_default()`. A harness test proves that table matches Chrome (WP-B). |
| 4 | Chrome settings sheet | **Deleted** (`core_host/sheet.rs`, `WebHost::show_chrome_page`, `ChromeSheet`, `render_chrome_sheet`, `Command::ChromeSettings`). The Chrome settings pages that are left open as normal tabs, the same way flags does. That leaves one WebUI mechanism instead of two. |
| 5 | Routing | One table, `baa-ui/src/pages.rs`. Routing runs only at entry points: command bar, `navigate`/`open_url`, `chrome_tabs` adoption, and links from lil-baa/Peek. It never runs on `UrlChanged`, so the client-side router of a WebUI settings tab is never interrupted. `baa://` and `chrome://` are the same page. |
| 6 | Extensions page | **A tab of the dialog.** The "own tab" machinery goes away: `EXTENSIONS_URL` tabs, `own_tab`, `is_own`, and the page-area render in `browser.rs` (~429-434, ~1776). A restored session drops `baa://extensions` tabs once on load. |
| 7 | WebUI skin | `chrome_skin(theme)` (`extensions.rs:563`, CSS variables) is kept and extended with a small base layer. It is injected into **every** WebUI page in normal tabs: baa-web sets one guarded init script on every page it creates or adopts (§3). |
| 8 | Default browser | Done in Rust, with no ABI: NSWorkspace `URLForApplicationToOpenURL:` (read) and `setDefaultApplicationAtURL:toOpenURLsWithScheme:completionHandler:` for `http` and `https` (set; macOS shows its own confirmation). Chrome's `DefaultBrowserWorker` (`chrome/browser/shell_integration.h:290`) is async and acts on the same main bundle. It needs the C++ only when Windows starts. |
| 9 | Downloads folder picker | gpui `cx.prompt_for_paths(PathPromptOptions { directories: true, .. })` (gpui `app.rs:1575`). The result is pushed as a pref. No ABI. |
| 10 | Languages | A text field of language codes (`en-US, en, sv`). `None` means the system default, which is not pushed. "More language settings…" opens the WebUI `baa://settings/languages` in a tab. No language-list ABI. On macOS, spellcheck is a single toggle because dictionaries are the system's. |
| 11 | Passwords and autofill | Helium forces Chrome's password manager off through a policy handler (`chromium/helium/patches/helium/core/disable-password-manager.patch`: `kCredentialsEnableService`/`kCredentialsEnablePasskeys` false). Address and card autofill default to off (`components/autofill/core/common/autofill_prefs.cc:45,76`), and Helium removes their UI (`helium/settings/remove-autofill.patch`). The Passwords tab therefore keeps today's two rows (extension managers through the native-messaging bridge, and Apple Passwords app; context in `baa-passwords-autofill.md`) and adds no autofill toggles. |
| 12 | Safe Browsing | No row: compiled out (`safe_browsing_mode=0`, PLAN-chromium-phase4.md §7). |
| 13 | Reset | baa `Settings::default()` (pushed as usual) plus, if ticked, "also clear site exceptions in every Space": Rust lists the rules per kind and unsets each one. No `ProfileResetter`: its `EXTENSIONS` flag disables extensions, and its other flags cover things baa pushes anyway. |
| 14 | Search engine | Stays global in baa-model, because the command bar resolves searches itself. It is pushed to every profile's `TemplateURLService`, so that Chrome's own uses agree: the context menu's "Search X for…", `chrome.search`, Helium's bangs, and M3's `AutocompleteController`. Helium's default is DuckDuckGo (`helium/core/search/engine-defaults.patch`). baa's is Google (`settings.rs:63`), and baa's value wins (open question 3). |

## 1. Pages: who draws what

| Page | Drawn by | Opens as |
|---|---|---|
| baa settings (all tabs below) | GPUI | dialog |
| `baa://extensions[/…][?id=]` | GPUI | dialog, Extensions tab (details when `id`) |
| `baa://settings` and the routed subpaths (table in §4) | GPUI | dialog, at that tab and row |
| `baa://history`, `baa://downloads` | GPUI (unchanged) | command bar History mode, Library |
| Every other `baa://settings/…` (`languages`, `fonts`, `content/all`, `certificates`, …) | Chrome WebUI, skinned | normal tab |
| `baa://flags`, `version`, `credits`, `gpu`, `net-internals`, `net-export`, `extensions-internals`, `inspect`, `chrome-urls`, `components`, `policy`, `prefs-internals`, `discards`, `process-internals`, `media-internals`, `webrtc-internals`, `site-engagement`, `sandbox`, `accessibility`, `crashes`, `management`, `certificate-manager`, `bookmarks` | Chrome WebUI, skinned | normal tab |

The hosts exist in this tree (`chrome/common/webui_url_constants.h:41-624`,
`content/public/common/url_constants.h:41-60`). The `baa` scheme is Helium's WebUI scheme, renamed
(`content/public/common/url_constants.h:22`, `helium/core/override-chrome-protocol.patch`). Only
`Navigate()` rewrites it to `chrome://`, so a raw `LoadURL` does not (that is why `sheet.rs:29`
exists).

## 2. The dialog

**Shape.** A scrim plus a centred card, `min(860, vw-64) × min(620, vh-80)`, with radius `r(14.)`, the
surface tokens, and ASCII outline/box title when `ascii()`. Left: a 200 px rail with a search field
at the top and then the tab list (icon + label). Right: the tab title, a scrolling body (one
`ScrollHandle`), and ✕.

**Navigation.**
- ⌘, toggles the dialog (existing `OpenSettings`).
- Esc clears the search first, then closes (`close_overlays`).
- ⌃⇥ / ⌃⇧⇥ select the next / previous tab.
- The search field has focus on open, and typing filters the rows.
- ↑/↓ moves through results and Enter jumps to the result.
- A click on the scrim closes the dialog.
- `open_settings_at(tab, row)` is the single entry point for routes, the command bar and buttons:
  it selects the tab, calls `scroll_to_item` on the row (gpui `div.rs:4128`) and flashes it.

**Search.** Each row is declared once in a static index `ROWS: &[Row { tab, id, label, keywords }]`.
The renderers take labels from the index, so search, the command bar and the UI cannot drift apart.
`fuzzy::score` runs over label + keywords. Results show grouped by tab and click through to the row.

**Motion (PLAN.md §10).**
- Open: the existing `settings-in` Glide, a scrim fade and a 14 px drop.
- Tab switch: the body crossfades with a 6 px rise (`("settings-tab", i)`, Glide), and the rail's
  selection pill moves on a `Spring` to the new row.
- Toggles: the existing knob spring. Segments: the existing hover feel.
- A row flashes when it is jumped to (`("settings-flash", id)`, fade out over about 600 ms).
- The card size is fixed, so there is no height animation.
- Reduce motion snaps everything (motion.rs already does this).

**Controls.** The existing `toggle`, `segments` and `action_button` from `prefs.rs` move into
`settings/mod.rs` unchanged. Two additions: a single-line `TextField` row (from `input.rs`) and a
Space picker (segments of Space icon + name) for the per-Space tabs. The picker defaults to the
active Space.

**Files.** `prefs.rs` is replaced by `settings/mod.rs` (shell, index, search, controls),
`settings/tabs.rs` (General, Appearance, Search, Passwords, Spaces, Keyboard, System) and
`settings/privacy.rs` (Privacy, the clear-data sheet, Site Settings). `extensions.rs` keeps its list
and details render, now called by the dialog, and loses the sheet and the routing. `pages.rs` is the
route table plus the command-bar entries.

### Tabs and rows

Scope: G = global (baa-model, pushed to every profile), S = per Space (Chrome is the store and is
read live). New baa-model fields are marked *new*. Pref names are verified in the tree. "Helium"
gives Helium's default where it differs from Chrome's.

**General**

| Row | Control | Backing | Scope |
|---|---|---|---|
| Default browser | "baa is your default browser" text, or a "Make Default" button | NSWorkspace (decision #8) | G |
| On launch | segments: Restore Today tabs / Pinned only | *new* `restore_today: bool` (true). `false` archives Today tabs at startup. | G |
| Links from other apps | segments: Little baa / New tab | *new* `external_links: ExternalLinks` (LilBaa) | G |
| Links that open a new tab | segments: Peek / New tab | *new* `peek_links: bool` (true) | G |
| Archive Today tabs after | segments (existing) | `archive_after_hours` | G |
| Save downloads to | path text + "Change…" | *new* `web.download_dir: Option<PathBuf>` → `download.default_directory` (`chrome/common/pref_names.h:1580`), string | G |
| Ask where to save each file | toggle | *new* `web.ask_where_to_save` → `download.prompt_for_download` (`pref_names.h:419`, registered `chrome/browser/download/download_prefs.cc:354`) | G |
| Preferred languages | text field, placeholder "System default" | *new* `web.languages: Option<String>` → `intl.accept_languages` (`components/language/core/browser/pref_names.h:16`) | G |
| Spell check | toggle | *new* `web.spellcheck` (true) → `browser.enable_spellchecking` (`components/spellcheck/browser/pref_names.h:11`, registered `chrome/browser/spellchecker/spellcheck_factory.cc:75`) | G |
| More language settings… | link | WebUI `baa://settings/languages` | S |
| Import: Spaces from Arc; History from … | buttons (existing) | `import::*` | G |

**Appearance**

| Row | Control | Backing | Scope |
|---|---|---|---|
| Window style | Arc / ASCII (existing) | `chrome` | G |
| Sidebar side | Left / Right (existing) | `sidebar_side` | G |
| Website appearance | System / Light / Dark | *new* `page_scheme: PageScheme` → existing `baa_tab_set_color_scheme` on every non-WebUI page | G |
| Default page zoom | segments 75 / 90 / 100 / 110 / 125 / 150 / 175 % | *new* `web.default_zoom: f64` (1.0) → `baa_profile_set_default_zoom` (new). The UI's per-tab zoom map starts from it (`browser.rs:1361`). | G |
| Font size | Small / Medium / Large / Very large (12/16/20/24) | *new* `web.font_size` (16) → `webkit.webprefs.default_font_size` (`pref_names.h:352`, registered `chrome/browser/ui/prefs/prefs_tab_helper.cc:465`, default 16) | G |
| Minimum font size | Off / 9 / 12 / 16 | *new* `web.min_font_size` (0) → `webkit.webprefs.minimum_font_size` (`pref_names.h:356`) | G |
| Reduce motion | toggle (existing) | `reduce_motion` | G |
| Highlight the focused element | toggle | *new* `web.focus_highlight` → `settings.a11y.focus_highlight` (`pref_names.h:921`, registered `chrome/browser/ui/browser_ui_prefs.cc:257`) | G |
| Caret browsing | toggle | *new* `web.caret_browsing` → `settings.a11y.caretbrowsing.enabled` (`pref_names.h:2989`, registered `browser_ui_prefs.cc:249`) | G |

**Search**

| Row | Control | Backing | Scope |
|---|---|---|---|
| Search engine | radio list: `SEARCH_ENGINES` + user-added (✕ to remove) | `search_engine` + *new* `search_engines: Vec<SearchEngine>` → `baa_profile_set_search_engine` (new) | G |
| Add search engine | name + URL with `%s`, Add | `search_engines` | G |
| Suggestions while typing | toggle (existing) | `search_suggest` (baa's own `fetch`). `search.suggest_enabled` stays off through `PREFS_OFF`. | G |

**Privacy & Security**

| Row | Control | Backing | Scope |
|---|---|---|---|
| Cookies | Allow all / Block third-party / Block all | *new* `web.cookies` (BlockThirdParty) → `profile.cookie_controls_mode` (`components/content_settings/core/common/pref_names.h:19`; values `cookie_settings.h:41-46`, default 1 at `cookie_settings.cc:79-82`): Allow all 0, else 1. "Block all" also sets the COOKIES default rule to BLOCK. | G |
| Clear cookies and site data when you quit | toggle | *new* `web.clear_cookies_on_quit` → COOKIES default rule SESSION_ONLY (BLOCK wins) | G |
| HTTPS | Off / Upgrade when possible / Always (warn first) | *new* `web.https` (Balanced, Helium's: `helium/core/prefer-https-by-default.patch`) → `https_only_mode_enabled` (`pref_names.h:100`) + `https_first_balanced_mode_enabled` (`pref_names.h:103`), the same pair `HttpsFirstModeService::UpdatePrefs` writes (`chrome/browser/ssl/https_first_mode_settings_tracker.cc:650-690`) | G |
| Preload pages | Off / Standard / Extended | *new* `web.preload` (Off, Helium's: `inox-patchset/modify-default-prefs.patch:41`) → `net.network_prediction_options` (`pref_names.h:440`; enum `chrome/browser/preloading/preloading_prefs.h:20-27`: 0 standard, 2 off, 3 extended) | G |
| Secure DNS | Off / Automatic / Cloudflare / Quad9 / Custom (template field) | *new* `web.secure_dns` (Automatic) → local state `dns_over_https.mode` ("off" / "automatic" / "secure") + `dns_over_https.templates` (`pref_names.h:2294,2304`, registered `chrome/browser/net/default_dns_over_https_config_source.cc:41-42`) | G |
| Send "Do Not Track" | toggle | *new* `web.do_not_track` → `enable_do_not_track` (`pref_names.h:1013`, registered `browser_ui_prefs.cc:183`) | G |
| Global Privacy Control | toggle | *new* `web.global_privacy_control` → `helium.global_privacy_control` (`pref_names.h:1016`, registered `browser_ui_prefs.cc:186`; `helium/core/network/global-privacy-control-pref.patch`) | G |
| Clear browsing data… | button → sheet inside the card | `baa_profile_clear_data` (new) + baa-model history/downloads | S |

Clear-data sheet:
- Time range: Last hour / Last 24 hours / Last 7 days / Last 4 weeks / All time.
- Data: Browsing history, Download history, Cookies and site data, Cached images and files.
- Spaces: This Space / All Spaces. Incognito Spaces are left out; they keep nothing.
- "Clear" runs one call per profile. baa-model's history and download records go too (new
  `Store::clear_history_since`, `clear_downloads_since`). It replaces the "Clear browsing history"
  command and row.

**Site Settings** (Space picker at the top)

| Row | Control | Backing | Scope |
|---|---|---|---|
| One row per kind: Location, Camera, Microphone, Notifications, Clipboard, MIDI, Pop-ups and redirects, Sound, JavaScript, Automatic downloads, Insecure content, Motion sensors, Window management | segments of the states valid for the kind (Ask/Allow/Block; Allow/Block for Pop-ups, Sound, JavaScript, Insecure content, Motion sensors) | default: *new* `web.site_defaults: Vec<(Permission, PermissionState)>` → `baa_profile_set_site_rule(…, "*", …)` | G |
| Under each kind, its exceptions | list of pattern + state segments + ✕ | `baa_profile_site_rules` / `baa_profile_set_site_rule` (new) | S |
| Cookie exceptions | the same list for COOKIES (Allow / Clear on quit / Block) | the same | S |
| One site (from the site popover's "Site settings…" and `baa://settings/content/siteDetails?site=`) | every kind for that site | existing `site_permission`/`set_site_permission`, with the new kinds | S |
| All sites and storage… | link | WebUI `baa://settings/content/all` | S |

ContentSettingsType names: `components/content_settings/core/common/content_settings_types.mojom`:
COOKIES 26, JAVASCRIPT 28, POPUPS 34, GEOLOCATION 37, NOTIFICATIONS 45, MIXEDSCRIPT 47, MEDIASTREAM_MIC 48,
MEDIASTREAM_CAMERA 49, AUTOMATIC_DOWNLOADS 52, MIDI_SYSEX 57, SOUND 91, SENSORS 104,
CLIPBOARD_READ_WRITE 191, WINDOW_MANAGEMENT 227.

**Passwords**

The two existing rows (Password managers → `set_up_passwords`, Apple Passwords → the Passwords app).
The detail text adds "Chrome's built-in password manager is off in baa." (decision #11).

**Extensions** (Space picker at the top)

The existing list and details render (`extensions.rs:194`: enable, site access, permissions, options,
remove with confirmation) plus a "Chrome Web Store" button (it opens a tab and closes the dialog).
Data comes from `extension_details`/`change_extension`, which are unchanged.

**Spaces**

One row per Space: icon, name, colour swatch, "incognito" tag, and Rename / Colour / Delete buttons
(the existing `start_rename`, `cycle_space_color`, `delete_active_space`, generalised to take a
SpaceId). "New Space" and "New Incognito Space" buttons.

**Keyboard**

A read-only table: each action in `actions.rs` with its keys from `window.bindings_for_action`
(gpui `window.rs:5939`). "Edit keymap.json…" creates the file as `[]` if it is missing and opens it
with the system default app (the path comes from the app shell, WP-E). The edit takes effect at the
next launch, as now.

**System**

| Row | Control | Backing | Scope |
|---|---|---|---|
| Graphics acceleration | toggle, "applies when baa restarts" | *new* `web.hardware_acceleration` (true) → local state `hardware_acceleration_mode.enabled` (`pref_names.h:2539`, registered `chrome/browser/gpu/gpu_mode_manager.cc:37-41`; read once at start) | G |
| Memory saver | toggle | *new* `web.memory_saver` (true, Helium's: `helium/core/memory-saving-by-default.patch`) → local state `performance_tuning.high_efficiency_mode.state` int 0 or 2 (`components/performance_manager/public/user_tuning/prefs.h:27-36`) | G |
| Developer tools | Right / Bottom / Left / Window (existing) | `devtools_dock` | G |
| Chrome pages | links: Flags, About, Certificates, GPU, Net internals, Inspect devices, All Chrome pages | WebUI tabs (§3) | S |
| Reset settings… | confirm row, with a checkbox "also clear site exceptions in every Space" | decision #13 | G |

## 3. Skinned WebUI in normal tabs

- **URL mapping (baa-web, `page.rs`).** Every load maps `baa://` to `chrome://` (`sheet.rs::chrome_url`
  moves here). Every `UrlChanged`/`tab_created` url maps `chrome://` back to `baa://`, so the UI and
  the session see only `baa://`. Typed `chrome://x` works as well, because routing treats both
  schemes the same.
- **The script.** `skin_loader` moves from `sheet.rs` to `core_host/skin.rs`. Its first line becomes
  `if (location.protocol !== 'chrome:' && location.protocol !== 'baa:') return;`, so websites pay one
  comparison and nothing else (no localStorage access). The rest is unchanged: a `<style
  id=baa-skin>`, `window.__baaSkin(css)`, and the localStorage cache that prevents a flash.
- **Where it is set.**
  - On every page baa-web creates or adopts, through `baa_tab_set_init_script`.
  - `create_page` with a WebUI url creates `about:blank` first, sets the script, then loads (as
    `Sheet::open` does now).
  - An adopted tab that is already on a WebUI page also gets one `tab_eval(loader)`.
  - Not on extension popups or DevTools front-ends (they are never `chrome:`/`baa:`).
- **The CSS.** `chrome_skin(theme)` stays in baa-ui (it owns the theme). It is the existing `--cr-*`,
  `--cr-fallback-color-*` and `--google-blue-*` map from `surface::*` + `theme.accent`, plus a base
  layer for pages that do not use cr variables (net-internals, gpu, version, flags tables):
  `body, table, th, td, input, select, button, a, code, pre`, with background/colour/border/font
  taken from the same tokens. Element rules only for generic HTML, never per page, so Chrome updates
  keep working. Variables inherit through shadow roots; that is why the skin targets them.
- **Per Space, light and dark.**
  - `WebHost::set_webui_skin(profile, css, dark)`. The UI calls it from render when the active
    Space's `chrome_skin` string changes, and caches the last string per profile. Each Space's WebUI
    tabs therefore carry that Space's accent.
  - The host stores the css per profile and calls `__baaSkin(css)` on that profile's live WebUI
    pages.
  - `dark` = `ascii()`: Arc surfaces are light and ASCII surfaces are dark.
  - On a WebUI page, the host sets `baa_tab_set_color_scheme(dark ? 2 : 1)`, so Chrome's own
    `prefers-color-scheme` rules match the skin. On a website, it sets the user's
    `page_scheme`.
  - This decision is taken in the `UrlChanged` handling of `page.rs`. Risk: `OverrideWebkitPrefs`
    can reset the scheme (PLAN-chromium-phase4.md §7 risk). The fix is to apply it again after
    each commit, which the `UrlChanged` hook already does.
- **Routing a typed first-party page.** `navigate("baa://settings/privacy")` opens the dialog at
  Privacy and creates no page (`pages::route`). A Chrome-made tab (`chrome_tabs`) on a first-party
  url is closed and routed. The UI never routes on `UrlChanged`, so in-page links inside a
  `baa://settings/languages` tab stay in WebUI. ponytail: that tab's left nav reaches every Chrome
  settings page. The ceiling is decision #2.

## 4. Command bar: `baa://` autofill

`pages.rs` holds `PAGES: &[Page { url, label, keywords, route }]`:
- The first-party routes: `baa://settings`, `settings/search`, `settings/privacy`,
  `settings/clearBrowserData`, `settings/content`, `settings/content/<kind>`, `settings/appearance`,
  `settings/downloads` → General, `settings/autofill` → Passwords, `settings/system`, `settings/reset`, `settings/defaultBrowser`,
  `settings/onStartup`, `settings/accessibility` → Appearance, `extensions`, `history`, `downloads`.
- The WebUI pages from the §1 table.
- `route(url)` matches by longest prefix. An unknown `settings/…` is WebUI; an unknown host is a
  plain page load.

Ranking in `command_bar::rows`:
1. **Query looks like a scheme** (starts with `baa:` or `chrome:`, or is a prefix of `baa://`, minimum
   3 chars): rows are `PAGES` whose url (normalised to `baa://`) starts with the query. Exact match
   first, then shorter urls first, at most 6. The typed "Open" row comes after them. **Tab** (or →
   at the end of the field) fills the field with the selected row's url. That is the autofill.
   Search suggestions and history are skipped for scheme queries.
2. **Plain words:** `PAGES` labels and the settings `ROWS` labels and keywords join the existing fuzzy
   `scored` pool after commands and Spaces. Ties keep push order: tabs, Spaces, commands, pages,
   setting rows. A setting row shows "Settings › Privacy" as detail. History stays last and
   suggestions stay second, as now.

New `Item::Page(&'static Page)` and `Item::Setting(&'static settings::Row)`.
- Submitting a page calls `open_own_page(url)`: dialog at its tab, or a WebUI tab.
- Submitting a setting calls `open_settings_at(tab, row)`.
- `Command::ChromeSettings` and its seven `COMMANDS` entries are deleted; `PAGES` replaces them.
  `COMMANDS` drops from 34 to 27.
- `Command::Settings` (⌘, chip) and `Command::Extensions` stay.

## 5. ABI additions (all go into the ABI 5 bump)

ABI 5 is the refactor's M3-start bump (steps 10-12). These additions join it. If the settings C++ is
ready first, it opens ABI 5, and steps 10-12 go into the same unreleased version. There is one bump
per released engine. Diff against `chromium/baa/baa.h`:

```c
// Permission kinds: new bits after OTHER (site settings only; never in `permission` asks).
  BAA_PERMISSION_POPUPS = 1 << 7,
  BAA_PERMISSION_SOUND = 1 << 8,
  BAA_PERMISSION_JAVASCRIPT = 1 << 9,
  BAA_PERMISSION_COOKIES = 1 << 10,
  BAA_PERMISSION_AUTOMATIC_DOWNLOADS = 1 << 11,
  BAA_PERMISSION_INSECURE_CONTENT = 1 << 12,   // MIXEDSCRIPT
  BAA_PERMISSION_SENSORS = 1 << 13,
  BAA_PERMISSION_WINDOW_MANAGEMENT = 1 << 14,

enum { BAA_SITE_UNSET = -1, BAA_SITE_ASK = 0, BAA_SITE_ALLOW = 1, BAA_SITE_BLOCK = 2,
       BAA_SITE_SESSION_ONLY = 3 /* cookies only */ };

// ---- Prefs -----------------------------------------------------------------
// `profile` NULL means Local State (global prefs). True when the pref is registered,
// user-modifiable (not forced by policy) and of that type; false changes nothing.
// Replaces baa_profile_set_pref_bool.
BAA_EXPORT bool baa_pref_set_bool(BaaProfile* profile, const char* name, bool value);
BAA_EXPORT bool baa_pref_set_int(BaaProfile* profile, const char* name, int value);
BAA_EXPORT bool baa_pref_set_string(BaaProfile* profile, const char* name, const char* value);

// The default zoom of the profile's pages; `factor` 1.0 is 100%.
BAA_EXPORT void baa_profile_set_default_zoom(BaaProfile* profile, double factor);

// Makes the engine the profile's default search, adding it when no engine has
// this URL. `url` holds %s for the query. Applied once the profile's engine list
// has loaded (maybe after this returns). False if Chrome refuses it as default.
BAA_EXPORT bool baa_profile_set_search_engine(BaaProfile* profile, const char* name,
                                              const char* keyword, const char* url);

// ---- Site settings ---------------------------------------------------------
// Rules the user set for `kind` (one BAA_PERMISSION_* bit, not OTHER): first the
// default as pattern "*", then each exception (Chrome's pattern string, e.g.
// "[*.]example.com"). Synchronous; returns the count, -1 for a gone profile.
typedef void (*BaaSiteRuleVisitor)(void* ctx, const char* pattern, int state);
BAA_EXPORT int baa_profile_site_rules(BaaProfile* profile, uint32_t kind,
                                      BaaSiteRuleVisitor visit, void* ctx);
// "*" sets the default; anything else an exception for that pattern. BAA_SITE_UNSET
// removes the exception (or restores Chrome's default for "*"). False for a bad
// pattern or a state the kind does not take.
BAA_EXPORT bool baa_profile_set_site_rule(BaaProfile* profile, uint32_t kind,
                                          const char* pattern, int state);

// ---- Browsing data ---------------------------------------------------------
enum {
  BAA_CLEAR_HISTORY = 1 << 0,
  BAA_CLEAR_DOWNLOADS = 1 << 1,
  BAA_CLEAR_SITE_DATA = 1 << 2,  // cookies, storage, service workers
  BAA_CLEAR_CACHE = 1 << 3,
};
// Deletes `types` (BAA_CLEAR_* bits) written since `since_ms` (Unix ms; 0 = all
// time); clear_done(id) when finished, never inside this call.
BAA_EXPORT void baa_profile_clear_data(BaaProfile* profile, uint64_t id,
                                       int64_t since_ms, uint32_t types);

// In BaaCallbacks:
  void (*clear_done)(void* ctx, uint64_t id);
```

| Function | Chromium mechanism |
|---|---|
| `baa_pref_set_*` | `profile->GetPrefs()` or `g_browser_process->local_state()`, then `FindPreference(name)`, check `IsUserModifiable()` and `GetType()`, then `SetBoolean/SetInteger/SetString`. The existing `baa::SetPrefBool` (`chromium/baa/tab.mm` ~1013-1023) is generalised. `download.default_directory` is a FilePath pref stored as a string, so `SetString` writes it. |
| `baa_profile_set_default_zoom` | `profile->GetZoomLevelPrefs()` (`chrome/browser/profiles/profile.h:309`) → `SetDefaultZoomLevelPref(blink::ZoomFactorToZoomLevel(f))` (`chrome/browser/ui/zoom/chrome_zoom_level_prefs.h:51`) |
| `baa_profile_set_search_engine` | `TemplateURLServiceFactory::GetForProfile` (`chrome/browser/search_engines/template_url_service_factory.h:24`). If `!loaded()` (`template_url_service.h:548`): `Load()` (`:530`) and `RegisterOnLoadedCallback` (`:537`), with the subscription kept on `BaaProfile`. Then find in `GetTemplateURLs()` (`:328`) by `url()` with `%s`→`{searchTerms}`, else `Add(make_unique<TemplateURL>(data))` (`:272`; `TemplateURLData::SetShortName/SetKeyword/SetURL`). Then `CanMakeDefault` (`:410`) → `SetUserSelectedDefaultSearchProvider` (`:418`). |
| `baa_profile_site_rules` | `HostContentSettingsMapFactory::GetForProfile`. The default comes from `GetDefaultContentSetting(type)` (`components/content_settings/core/browser/host_content_settings_map.h:114`). Exceptions are `GetSettingsForOneType(type)` (`:192`) filtered to `source == SettingSource::kUser` and to a non-wildcard primary pattern, then `primary_pattern.ToString()`. |
| `baa_profile_set_site_rule` | For "*": `SetDefaultContentSetting(type, s)` (`:208`; `CONTENT_SETTING_DEFAULT` for UNSET). Otherwise `ContentSettingsPattern::FromString` → `SetContentSettingCustomScope(p, Wildcard(), type, s)` (`:231`). The kind map in `permissions.mm:146-158` gains the new types. SESSION_ONLY only for COOKIES. Allow/Block only for POPUPS, SOUND, JAVASCRIPT, MIXEDSCRIPT, SENSORS. |
| `baa_profile_clear_data` | `profile->GetBrowsingDataRemover()` (`content/public/browser/browser_context.h:149`) → `RemoveAndReply(begin, base::Time::Max(), mask, ORIGIN_TYPE_UNPROTECTED_WEB, observer)` (`content/public/browser/browsing_data_remover.h:232,179`). Masks: `chrome_browsing_data_remover::DATA_TYPE_HISTORY` / `DATA_TYPE_SITE_DATA` (`chrome/browser/browsing_data/chrome_browsing_data_remover_constants.h:24,56`), `content::BrowsingDataRemover::DATA_TYPE_DOWNLOADS` / `DATA_TYPE_CACHE` (`browsing_data_remover.h:89-90`). The observer lives in a `flat_map<uint64_t, unique_ptr<…>>` until `OnBrowsingDataRemoverDone`, which posts `clear_done`. |

**Not added, on purpose:** pref getters and change notifications (decision #2), a languages list
(decision #10), default browser (decision #8), a directory chooser (decision #9), a search-engine
list (baa-model holds it), `ProfileResetter` (decision #13), and string-list prefs (nothing needs
one on macOS).

## 6. WebHost contract diff (`crates/baa-web/src/lib.rs`)

```rust
pub use baa_model::{Permission, PermissionState};   // moved to baa-model (settings need them)

trait WebHost {
    /// baa's global browser settings into every profile, now and as each one opens. Sends only what changed.
    fn apply_settings(&self, settings: &baa_model::Settings);
    /// The user's rules for `kind` in `profile`, the default ("*") first.
    fn site_rules(&self, profile: ProfileId, kind: Permission) -> Vec<(String, PermissionState)>;
    /// `None` removes the rule.
    fn set_site_rule(&self, profile: ProfileId, kind: Permission, pattern: &str, state: Option<PermissionState>);
    fn clear_data(&self, profile: ProfileId, since: Option<std::time::SystemTime>, what: ClearData, cx: &mut App, done: Callback<()>);
    fn is_default_browser(&self) -> bool;
    /// macOS asks the user; check again with `is_default_browser` later.
    fn make_default_browser(&self);
    /// Stylesheet over `profile`'s Chrome pages (`baa://…`); `dark` matches its colours.
    fn set_webui_skin(&self, profile: ProfileId, css: String, dark: bool);
    // removed: show_chrome_page, ChromeSheet
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct ClearData { pub history: bool, pub downloads: bool, pub site_data: bool, pub cache: bool }
```

`Permission` gains `Popups, Sound, JavaScript, Cookies, AutomaticDownloads, InsecureContent, Sensors,
WindowManagement` (`ScreenShare` goes in the refactor). `PermissionState` gains `SessionOnly`.

## 7. baa-model additions (`settings.rs`)

The fields marked *new* in §2. `Settings` is `#[serde(default)]`, so old JSON loads unchanged. The
types are `ExternalLinks {LilBaa, Tab}`, `PageScheme {System, Light, Dark}` and `WebPrefs` (the
`web.*` fields), with `Cookies {AllowAll, BlockThirdParty, BlockAll}`, `Https {Off, Balanced,
Strict}`, `Preload {Off, Standard, Extended}` and `SecureDns {Off, Automatic, Custom(String)}`.
Cloudflare and Quad9 are `Custom` with their template (the segment control matches on the string).
`Permission::SITE_KINDS` gives the order in the UI, and `Permission::chrome_default()` the defaults
table. `Store::clear_history_since(ms)` and `clear_downloads_since(ms)` are added.

The mapping to Chrome lives in baa-web (`core_host/settings.rs`, pure):
`fn writes(&Settings) -> Vec<Write>`, with `enum Write { Bool(&'static str, bool), Int(..),
Str(..), LocalBool(..), LocalInt(..), LocalStr(..), SiteDefault(Permission, PermissionState),
Engine{..}, Zoom(f64) }`. The host applies `writes` to each profile on open (before `PREFS_OFF`, so
privacy pins still win), and on `apply_settings` only the diff against the last list. Local-state
writes go once, not per profile.

## 8. Work packages

Order: refactor "Now" lands, then **WP-0**, then WP-A, WP-B, WP-C and WP-D in parallel, then WP-E.
WP-B and WP-C merge together, because they are the two sides of ABI 5.

**WP-0 Contract** (planning lead; one PR touching the contracts only)
- The `baa.h` diff from §5, with `BAA_ABI_VERSION` 5 if it opens the bump.
- The `lib.rs` trait diff from §6, with core_host bodies that log and return the empty answer.
- `Permission`/`PermissionState` moved into baa-model with the new variants; the baa-model field
  and type stubs with defaults.
- `mock.rs` (baa-ui MockHost) implements the new methods and records the calls.
- Accepted when `cargo check` passes for every crate and the existing tests pass.

**WP-A baa-model** (`crates/baa-model`)
- Fills in §7: defaults, `chrome_default()`, and the two `clear_*_since` functions.
- Tests:
  - Old settings JSON (today's shape) loads with every new default.
  - The round trip keeps every field.
  - `SecureDns::Custom` survives serde.
  - `clear_history_since` keeps older rows and removes newer ones; the same for downloads.

**WP-B //baa C++ + harness** (`chromium/baa/*`, `crates/baa-chromium`)
- New `settings.mm`: pref setters, default zoom, search engine, clear data. Add it to `BUILD.gn`.
- `permissions.mm`: new kinds, rules, SESSION_ONLY.
- `baa_profile_set_pref_bool` is deleted.
- New harness commands in `main.rs`: `prefb|prefi|prefs <1|local> <name> <value>` (they replace
  `pref`), `rules <kind>`, `rule <kind> <pattern> <state>`, `clear <types> <since_ms>`,
  `engine <name> <keyword> <url>`, `zoomdefault <factor>`. Each logs `baa: <call> … -> <result>`.
- Acceptance (no GUI driving; harness `tests/settings.txt` + `settings.expect` under `ci.py`):

  | Test | must: |
  |---|---|
  | Prefs | `prefb 1 enable_do_not_track true -> 1`, `prefb 1 no.such true -> 0`, `prefi 1 enable_do_not_track 1 -> 0` (wrong type), `prefb local hardware_acceleration_mode.enabled false -> 1` |
  | Font | `prefi 1 webkit.webprefs.default_font_size 24`, then `load data:text/html,<p>x` and `eval document.title=getComputedStyle(document.body).fontSize` → `title="24px"` |
  | DNT | `prefb 1 enable_do_not_track true`, then `eval document.title='dnt:'+navigator.doNotTrack` → `title="dnt:1"` |
  | Defaults table | `rules <kind>` for each of the 14 kinds prints `* <state>`. The .expect lists exactly the values of `Permission::chrome_default()`. This is the test that keeps decision #3 honest. |
  | Exceptions | `rule popups [*.]example.com allow -> 1`; `rules popups` shows the line; `rule popups [*.]example.com unset`; the line is gone. `rule cookies [*.]a.test session -> 1`; `rule popups x session -> 0`. |
  | Clear data | `load https://example.com`, `eval document.cookie='a=1;max-age=999'`, `clear site 0` → `clear_done id=1`, reload, `eval document.title='c:'+document.cookie` → `title="c:"` |
  | Search engine | `engine Kagi kagi.com https://kagi.com/search?q=%s -> 1`, then right-click on selected text (`menu`) shows the label `Search Kagi for` |
  | Zoom | `eval` `devicePixelRatio` before and after `zoomdefault 1.5` on a fresh load; the ratio is 1.5 |

**WP-C baa-web** (`crates/baa-web`)
- `ffi.rs` mirrors the header. The refactor's `include_str!("baa.h")` test covers the new constants.
- `core_host/settings.rs` (`writes` + apply), the `host.rs` profile-open hook, site rules, clear data
  (id → callback map, as `fetch`), default browser in `native.rs` (NSWorkspace), `core_host/skin.rs`
  (loader + per-profile css), the `page.rs` url mapping and WebUI colour scheme.
- `sheet.rs` and `show_chrome_page` are deleted.
- Tests:
  - Pure, `writes`: `Settings::default()` gives the exact expected list. `BlockAll` gives int 1 +
    `SiteDefault(Cookies, Block)`. `clear_cookies_on_quit` gives SessionOnly, and BlockAll wins over
    it. `Https::Strict` gives the pair (true, false). `languages: None` writes nothing.
  - Fake Core: two open profiles + `apply_settings` → each gets the writes and local state gets them
    once. A second `apply_settings` with one change → exactly one call. A profile opened later gets
    the full list, then `PREFS_OFF`.
  - Fake Core: `site_rules` decodes the visitor (`*` first, `BAA_SITE_SESSION_ONLY` →
    `SessionOnly`). `set_site_rule(None)` sends `BAA_SITE_UNSET`.
  - Fake Core: `clear_data` → `clear_data(id, since, bits)`. A queued `clear_done(id)` runs the
    callback once, and an unknown id is ignored.
  - Fake Core: `create_page(.., "baa://flags")` → `tab_create(about:blank)`, `set_init_script`
    (contains the protocol guard), `load_url("chrome://flags")`. `tab_state` with
    `chrome://flags/` → `UrlChanged("baa://flags/")`. `set_webui_skin` → `tab_eval(__baaSkin…)` only
    on that profile's WebUI pages. A WebUI url sets scheme 1 or 2; a website sets the `page_scheme`
    value.

**WP-D baa-ui** (`crates/baa-ui`)
- The dialog (§2), `pages.rs` (§4), Extensions as a dialog tab, and the command bar rows + Tab
  completion.
- Behaviour for the new baa fields: `restore_today` at startup, `external_links` in the lil-baa
  routing, `peek_links` in `OpenRequested` handling, zoom base = `web.default_zoom`.
- `apply_settings` on load and after every `save_settings`; `set_webui_skin` from render.
- Deleted: the chrome sheet, the own extensions tab, `Command::ChromeSettings`, `prefs.rs`.
- Tests: gpui `TestAppContext` + MockHost in `tests.rs`:
  - `OpenSettings` opens the dialog at General. Esc closes it. Esc with search text only clears
    the text.
  - `open_url("baa://settings/privacy")` → dialog at Privacy, `mock.created` is empty.
    `"chrome://settings/content/popups"` → Site Settings. `"baa://extensions/?id=x"` → the
    Extensions tab with x open.
  - `open_url("baa://flags")` → a page is created at `baa://flags`. `"baa://settings/fonts"` → a
    page (WebUI).
  - Flipping a toggle saves to the store (reload `Store` and check), and `mock.applied` (the last
    `apply_settings`) has the new value.
  - Command bar: `"baa://se"` → the first row is a `baa://settings…` page. Tab fills the field.
    Enter on `baa://extensions` opens the Extensions tab. `"cookies"` lists the Cookies setting
    row, and selecting it opens Privacy with that row selected.
  - Settings search: `"zoom"` shows "Default page zoom" under Appearance.
  - Clear data: tick history + site data, All Spaces, Last hour → one `mock.clear_data` per
    non-incognito Space with `since ≈ now-1h`, and baa-model history newer than 1h is gone.
  - Site settings: the mock returns `[("*",Block),("[*.]a.com",Allow)]` for Popups → both rows are
    drawn. ✕ records `set_site_rule(.., "[*.]a.com", None)`.
  - Default browser: mock `false` → a "Make Default" button, and a click records
    `make_default_browser`.
  - A session that contains a `baa://extensions` tab restores without it.
  - `restore_today = false` → Today tabs are archived at startup.

**WP-E app shell** (`crates/baa`)
- Pass the `keymap.json` path into baa-ui (a `Browser::new` argument or `baa_ui::init` field) for
  the Keyboard tab. Nothing else.
- `cargo build -p baa` once, then a manual pass by the user (no agent GUI tests).

## Risks

| Risk | Cheapest test | Fallback |
|---|---|---|
| Some kinds have moved to `PermissionSetting` storage (`SetPermissionSettingCustomScope`, `host_content_settings_map.h:261`), so `GetSettingsForOneType` or `SetContentSettingCustomScope` misbehave for them (geolocation). | The WP-B exceptions test, run for each of the 14 kinds | Use the permission-setting variants for those types in `permissions.mm` |
| Pref writes on an ephemeral (OTR) profile go through to its parent profile's store | Harness: `prefb` on an ephemeral profile, then read the parent at `chrome://prefs-internals` | Skip OTR profiles in `apply`. They inherit from the parent, and the values are global anyway. |
| The skin init script runs on every website document (cost, CSP) | The guard returns before touching the DOM; a fake-Core test checks the guard is the first statement | None needed |
| Memory saver discards hidden baa tabs, and the view swap is missed | Existing view-swap path (`BaaTabState.view`); manual check | Default `memory_saver` off |
| `SetWebPreferences` resets the WebUI colour scheme | Covered by re-applying on each `UrlChanged` | The same, plus `OnWebContentsFocused` |

## Open questions for the user

1. **Global settings** (decision #2): browser behaviour settings are global, and site exceptions,
   extensions and clearing data are per Space. Is that right, or do you want per-Space cookies,
   HTTPS and so on as Chrome profiles have them?
2. **Extension developer mode** (load unpacked, pack, inspect views): add a GPUI row, which needs
   one more ABI call (`UnpackedInstaller`)? Without it, Chrome's extensions WebUI cannot be reached,
   because `baa://extensions` always routes to the dialog.
3. **Default search engine:** baa-model says Google and Helium says DuckDuckGo (PLAN.md §11 M3 also
   said DDG). baa's value will win in every profile. Which one should the default be?
4. **Dialog vs window:** an overlay card in the browser window (as now) is planned. Do you want a
   separate macOS settings window (Safari style) instead?

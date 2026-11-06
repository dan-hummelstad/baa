# M3: ABI 6, Chrome's dialogs in GPUI, tab groups, omnibox ranking (draft, 2026-11-06)

Spec, not built. Written against Chromium 156: the Helium tree is at
`/Users/Shared/hb/helium-macos/build/src` with baa's patches applied (`/Users/Shared/hb/src` does not
exist). Paths that start with `chrome/`, `components/`, `content/` or `extensions/` are in that tree.
The base is ABI 5 (`chromium/baa/baa.h`) and the ADK refactor steps 1–9 (`PLAN-adk-refactor.md`).
The work packages follow the PLAN-settings.md model: one contract package first, then packages for
the two sides of the ABI that merge together.

**M3 is done when:**
- ABI 6 is in, on both sides at once. Tab handles are never-reused `uint64_t` ids. `tab_closed`
  arrives exactly once per id, with a reason. One registry answers every ask. DevTools docking
  lives in C++.
- HTTP auth, external-protocol, hung-page and extension-install prompts are GPUI sheets on the tab.
  Chrome's views dialogs no longer appear for baa's tabs.
- A crashed page says so and reloads.
- Page fullscreen fills the window.
- The find bar shows `3/12`.
- Live tabs in a folder are a Chrome tab group with the folder's name. An extension's
  `chrome.tabs.group` puts tabs into a folder.
- The command bar ranks history and suggestions with Chrome's `AutocompleteController`.
  `fetch`/`parse_suggestions` are gone.

Everything is checked by build, `cargo test` and harness files that are written but not run (§6).

## 1. Scope

| # | Item | In/out | Why |
|---|---|---|---|
| 1 | Refactor 10: one id registry for asks, `request_closed` | **In** (WP-C1a/C1b, WP-R1) | Without it, each new ask in this milestone costs ~15 edits. |
| 2 | Refactor 11: DevTools dock in C++ | **In** (WP-C1a, WP-R1) | Removes `page_message`, `devtools.rs` and the dock logic in `page.rs`. A typed `devtools_dock` callback replaces them. |
| 3 | Refactor 12: `uint64_t` tab ids, `tab_closed(reason)` exactly once, download ids from the shared counter | **In** (WP-C1a, WP-R1) | Removes `forget_profile`, the `seq` field of `Pending` and the address-reuse caveat. |
| 4 | GPUI HTTP auth | **In** | One 3-line hook (`LoginHandler::Create`). The rest is `LoginHandler` subclassed by //baa. |
| 5 | GPUI external-protocol prompt | **In** | One 3-line hook at the top of `ExternalProtocolHandler::RunExternalProtocolDialog`. Chrome keeps its block-state and remember logic. |
| 6 | GPUI hung-renderer prompt | **In** | No patch. A `TabDialogsViews` subclass overrides the three hung methods. |
| 7 | GPUI extension install / permissions prompt | **In** | One 3-line hook at the top of `ShowExtensionInstallDialogImpl`. Chrome's own strings are used. |
| 8 | Sad tab (page crashed) | **In** | `BaaTabState.crashed` → `PageEvent::Crashed(bool)`. About 10 lines on each side. |
| 9 | Page fullscreen | **In** | Today `Window::EnterFullscreen` does nothing (`window.mm:627`), so videos only go fullscreen inside the tab. Adds one callback and one function. |
| 10 | Find match counts | **In** | One `FindResultObserver` and one callback. The find bar already exists. |
| 11 | `chrome.tabGroups` ↔ folders | **In**, minimal (§2.4) | Groups already work on our Browsers. Folders → groups for live tabs. Chrome → folders for membership and title only. |
| 12 | Command bar on `AutocompleteController` | **In**, minimal (§2.5) | Replaces `fetch` + `parse_suggestions` + the typed-mode history search. The bar keeps its own tab, Space, command and page rows. |
| 13 | `chrome.commands` shortcuts | Deferred (M4) | Needs an `ExtensionKeybindingRegistry` subclass fed from the `key` callback (there is no focus manager). This is real work. Only extension shortcuts need it (1Password's ⌘⇧X is the visible loss). |
| 14 | `chrome.action.openPopup` | Deferred (M4) | Mostly called from command shortcuts (#13). It also needs `IsToolbarVisible() == true`, which wakes other toolbar code paths (risk 1 of phase 4). |
| 15 | Dynamic action icons and badges | Deferred (M4) | Needs a push channel for per-tab action state plus bitmap transport. Static icons work today. |
| 16 | Dangerous-download keep/discard | Deferred | Safe Browsing is compiled out, so only file-type warnings exist. Auto-validate stays (phase 4 #6). |
| 17 | `getDisplayMedia` picker in GPUI | Deferred | Chrome's views picker works as a parentless window. A GPUI one needs a live thumbnail stream. |
| 18 | Tab-strip order that follows the sidebar | Deferred | Only extensions' `tab.index` sees it. Groups make their tabs contiguous by themselves. |
| 19 | Omnibox shortcuts learning (`ShortcutsBackend::AddOrUpdateShortcut`) | Deferred | Without learning, the shortcuts provider stays empty and is left out. HistoryQuick already ranks typed visits (`baa_tab_load_url` uses `PAGE_TRANSITION_TYPED`). |
| 20 | Refactor 13 (Windows seam), 14 (token adoption) | Out | Those plans keep their own triggers. |

### Who draws what after M3

| Feature | M2 | M3 |
|---|---|---|
| alert/confirm/prompt, leave site? | GPUI sheet | same sheet, now with an id |
| HTTP auth | Chrome views, parentless | GPUI sheet: two fields |
| External protocol | Chrome views | GPUI sheet: "Open Zoom?" plus "Always allow" checkbox |
| Hung renderer | Chrome views | GPUI sheet: Exit page / Wait |
| Extension install, `permissions.request` | Chrome views, browser-modal | GPUI sheet with Chrome's permission lines |
| Crashed page | blank page | GPUI overlay: "This page crashed" + Reload |
| Page fullscreen | inside the tab | the page alone, filling the window (open question 4) |

## 2. Design

### 2.1 One ask channel (refactor step 10, extended)

JS dialogs and Chrome's own dialogs become **one** callback, `dialog(tab, id, const BaaDialog*)`. All
of them answer through **one** function, `baa_answer(id, const BaaAnswer*)`. Chrome withdraws any of
them, and permission asks too, with **one** `request_closed(tab, id)`. This is a deliberate change to
the refactor plan's "one typed ask callback and one typed answer per feature". The registry already
knows each request's kind, so a single answer struct keeps Rust free of routing tables. Each new
dialog then costs one `BAA_DIALOG_*` value and one class in `asks.mm`. Context menus keep
`baa_context_menu_done`, because their answer is a command id. Permission asks keep their own
callback, because their UI is a banner, not a sheet. Their answer is `baa_answer(id, {ok})`.

`requests.mm` (C++):
```cpp
// A pending ask the app answers by id. Chrome may own the object (a PermissionPrompt) or the
// registry may own it (a dialog view); either way it is registered while unanswered.
class Request {
 public:
  Request(uint64_t tab);                       // takes an id from NextId(), registers itself
  virtual ~Request();                          // unregisters (silently)
  uint64_t id() const; uint64_t tab() const;
  virtual void Answer(const BaaAnswer&) {}     // ask kinds
  virtual void Command(int command) {}         // context menus
  virtual void Drop() = 0;                     // the tab is gone or leaves our windows: answer
                                               // Chrome "no"/close, never touch the app
};
Request* FindRequest(uint64_t id);
void WithdrawRequest(uint64_t id);             // Chrome dropped it: unregister + request_closed
void DropRequestsOf(uint64_t tab, bool tell);  // Forget(): every request of the tab
```
The existing `BeginRequest/FinishRequest` (the app's ids for clear/load-unpacked) stay as they are.
They run in the other direction and are small.

### 2.2 Chrome's dialogs (no Chrome views for baa's tabs)

| Dialog | Interception | Kind | Strings (Chrome's own, localized) | Answer |
|---|---|---|---|---|
| HTTP auth | `g_baa_login_handler` at the top of `LoginHandler::Create` (`chrome/browser/ui/views/login_handler_views.cc:230`). It returns a `baa::LoginPrompt : LoginHandler` (ctor protected, `chrome/browser/ui/login/login_handler.h:82`) for a baa tab. `BuildViewImpl(authority, explanation, …)` (`:91`) sends the dialog. `CloseDialog()` (`:96`) withdraws it. | `BAA_DIALOG_AUTH` | title `IDS_LOGIN_DIALOG_TITLE`, message = `authority` + "\n" + `explanation` (`login_handler.cc:346-372`) | ok → `SetAuth(user, password)`, else `CancelAuth(true)` (`login_handler.h:68,73`) |
| External protocol | `g_baa_external_protocol` at the top of `ExternalProtocolHandler::RunExternalProtocolDialog` (`chrome/browser/ui/views/external_protocol_dialog.cc:95`). Not `SetDelegateForTesting`: its `GetBlockState(scheme, profile)` loses the initiating origin (`external_protocol_handler.cc:122-131`), so remembered allows would never apply. | `BAA_DIALOG_EXTERNAL_PROTOCOL` | `IDS_EXTERNAL_PROTOCOL_TITLE`(program), message as `GetMessageTextForOrigin` (`external_protocol_dialog.cc:43-51`), OK/cancel labels (`:129-134`), checkbox only if `MayRememberAllowDecisionsForThisOrigin` and the pref allow it (`:165-178`) | as `OnDialogAccepted` (`:205-223`): if checked, `SetBlockState(scheme, origin, DONT_BLOCK, profile)`; then `LaunchUrlWithoutSecurityCheck(url, wc, doc)` |
| Hung renderer | `baa::TabDialogs : TabDialogsViews` (`chrome/browser/ui/views/tab_dialogs_views.h:12-14`), set with the protected `UserDataKey()` (`chrome/browser/ui/tab_dialogs.h:73`) in `AttachDialogs`, restored in `ReturnDialogsToChrome`. It overrides `Show/Hide/IsShowingHungRendererDialog` (`tab_dialogs.h:44-49`), which `BrowserWebContentsDelegate::RendererUnresponsive/Responsive` call (`browser_web_contents_delegate.cc:1097-1118`). | `BAA_DIALOG_UNRESPONSIVE` | `IDS_BROWSER_HANGMONITOR_RENDERER_TITLE` (plural 1), message = the tab title, ok `IDS_BROWSER_HANGMONITOR_RENDERER_END`, cancel `…_WAIT` (`hung_renderer_view.cc:327,421-478`) | ok → as `ForceCrashHungRenderer` (`:451-462`: `CrashDumpHungChildProcess` + `Shutdown(RESULT_CODE_HUNG)`); cancel → `hang_monitor_restarter.Run()`. `Hide` withdraws. |
| Extension install / re-enable / `permissions.request` | `g_baa_install_dialog` at the top of `ShowExtensionInstallDialogImpl` (`chrome/browser/ui/views/extensions/extension_install_dialog_view.cc:85`). Tab: `show_params->GetParentWebContents()` if it is a baa tab, else the profile's active tab (`show_params->profile()`, `extension_install_prompt_show_params.h:50,56`). If there is none, Chrome's own dialog shows. | `BAA_DIALOG_EXTENSION_INSTALL` | `prompt->GetDialogTitle()`, `GetPermissionsHeading()` + each `GetPermission(i)` on its own line, `GetAcceptButtonLabel()` (`extensions/browser/install_prompt_data.h:80-106`) | `DoneCallbackPayload(ACCEPTED or USER_CANCELED)` (`extension_install_prompt_client.h:32-46`) |

All three hooks go into one patch, `dialog-hooks.patch`. They are function pointers declared in
`chrome/browser/baa_hooks.h` (created by `color-scheme-hook.patch`) and set by //baa in
`EnsureStarted`. Null keeps Chrome's behaviour. A hook that does not take the request leaves its
arguments alone, so Chrome's path continues unchanged.

### 2.3 Fullscreen, find counts, crashes

- **Fullscreen.** `Window` keeps a `fullscreen_` bool. `EnterFullscreen` sets it and sends
  `fullscreen(active tab, true)`, then posts
  `ExclusiveAccessManager::From(browser)->fullscreen_controller()->WindowFullscreenStateChanged()`
  (`exclusive_access_manager.h:40,58`; `fullscreen_controller.cc:384,631-635`, which waits for that
  call). `ExitFullscreen` mirrors it. `IsFullscreen()` returns the bool (`window.mm:308`).
  - In the page, Esc is already Chrome's: `BrowserWebContentsDelegate::PreHandleKeyboardEvent`
    hands it to the exclusive-access manager before baa's `key` callback
    (`browser_web_contents_delegate.cc:311-322`).
  - `baa_tab_exit_fullscreen(tab)` → `wc->ExitFullscreen(true)` (`content/public/browser/web_contents.h:1586`).
    It covers the cases where the user leaves through baa: the window leaves macOS fullscreen, the
    user switches Space, or ⌃⌘F.
  - ponytail: Chrome is told "fullscreen" at once, not after the window animation. Ceiling: the page
    lays out once at the old size, mid-animation.
- **Find counts.** `BaaTab` adds a `find_in_page::FindResultObserver` on its `FindTabHelper`
  (`components/find_in_page/find_result_observer.h:18-24`, `find_tab_helper.h:45,112`) →
  `find_result(tab, active_match_ordinal, number_of_matches)`.
- **Crash.** `BaaTabState.crashed = wc->IsCrashed()` (`web_contents.h:981`).
  `PrimaryMainFrameRenderProcessGone` (`web_contents_observer.h:280`) calls `SendState`. The sad-tab
  guard in `no-views-guards.patch` stays.

### 2.4 Tab groups ↔ folders

Our Browsers already support groups (`TabStripModel::SupportsTabGroups`, `tab_strip_model.h:719`).
- **Keys.** A group is a `uint64_t` key.
  - The UI's keys are `FolderId.0`, with the top bit clear.
  - A group that something other than baa made (an extension) gets `NextId() | 1ull << 63` from C++.
  - C++ keeps a key → `TabGroupId` map per Browser. A group that closes (`TabGroupChange::kClosed`,
    `tab_strip_model_observer.h:256-262`) leaves the map, and the next `set_group` with that key
    makes a new group.
- **baa → Chrome.** `baa_tab_set_group(tab, key, title)`:
  - key 0 → `RemoveFromGroup({i})` (`tab_strip_model.h:711`).
  - known key → `AddToExistingGroup({i}, g)` (`:697`).
  - unknown key → `AddToNewGroup({i})` (`:688`).
  - Then `ChangeTabGroupVisuals(g, {title, kGrey})` (`:763`) if the title differs.
  - Color is always grey (folders have none). `collapsed` is not mirrored: collapsing can move the
    active tab.
  - A guard flag stops the echo below.
- **Chrome → baa.** `Window` overrides `TabGroupedStateChanged` and `OnTabGroupChanged`
  (`tab_strip_model_observer.h:554-564`) and forwards them to `groups.mm`. Outside the guard:
  - a membership change sends `tab_group(tab, key or 0, title)`;
  - `kVisualsChanged` sends `tab_group` for every tab of the group, with the new title.
  - Moves are not reported (#18).
- **UI rule (baa-ui `groups.rs`).** In `Browser::sync`, every live page whose tab is in a folder
  gets `set_group(Some((key, folder name)))`, and pages outside folders get `None`. Only changes are
  sent: the last value applied per tab is kept. On `GroupChanged`:
  - an unknown key → a new folder (named with the title, or "Group") in the tab's Space, remembered
    as `chrome_keys[key] = folder`;
  - then the tab moves into that folder (so it becomes pinned, Arc-like);
  - a non-empty title that differs renames the folder;
  - `None` takes the tab out of its folder (it stays pinned).
- A group exists in Chrome only while at least one of the folder's tabs is live. That is the snag
  phase 4 named, settled this way.

### 2.5 Command bar on AutocompleteController

- **C++ (`autocomplete.mm`).** One `AutocompleteController` per profile, made on first use, modelled
  on `OmniboxPageHandler::CreateController` (`chrome/browser/ui/webui/omnibox/omnibox_page_handler.cc:255-270`):
  `make_unique<ChromeAutocompleteProviderClient>(profile)` (`chrome_autocomplete_provider_client.h:47`).
  - Providers: `kHistoryQuick | kHistoryURL | kSearch | kCalculator`, in an
    `AutocompleteControllerConfig` (`autocomplete_controller_config.h:13-35`).
    `DefaultOmniboxProviders()` (`autocomplete_classifier.cc:56`) is not used: it brings zero-suggest,
    open tabs (baa has its own) and featured search.
  - Input: `AutocompleteInput(text, OmniboxEventProto::OTHER, ChromeAutocompleteSchemeClassifier(profile))`
    (`autocomplete_input.h:45-51`; usage as `omnibox_page_handler.cc:210-225`).
  - `OnResultChanged` (`autocomplete_controller.h:136`) → `autocomplete(id, matches, n, done)`, with
    `done = controller->done()` (`:293`).
  - `SEARCH_WHAT_YOU_TYPED` and `URL_WHAT_YOU_TYPED` matches are dropped, because baa's first row
    covers them.
  - A new `baa_profile_autocomplete` restarts the query (`Start`, `:198`). Empty text calls `Stop`.
    Results for an older id are never sent.
  - The controller dies with the profile: `BaaProfile` owns it.
- **Network suggestions.** These need `search.suggest_enabled`. It leaves `PREFS_OFF`
  (`crates/baa-privacy/src/chromium.rs:15`), and baa-web's `settings.rs` writes it from
  `Settings::search_suggest`. The engine's suggest URL is the `TemplateURL`'s: built-in engines
  (Google, DDG, Brave) keep theirs, because `baa_profile_set_search_engine` reuses built-ins.
- **Rust.**
  - `WebHost::autocomplete(profile, text, update)` replaces `fetch`.
  - In `command_bar.rs`, `suggestions: Vec<String>` and the typed-mode `history` become
    `matches: Vec<Suggestion>`. Search matches render as suggestion rows (`MAX_SUGGESTIONS`). Page
    matches render as history rows (`MAX_HISTORY`), after open tabs, without URLs that are already
    open.
  - History mode (⌘Y) and the Library keep baa-model's history.
  - Deleted: `Settings::suggest_url`, `parse_suggestions`, `baa_profile_fetch`, `fetch_done`,
    `net.mm`, `Dispatch.fetches`.
- History becomes per Space (each Space is a Chrome profile): open question 2.

## 3. ABI 6 (`chromium/baa/baa.h` diff)

Only the parts that change are shown. Every `BaaTab* tab` parameter and return value becomes
`BaaTabId tab`; those rows are listed once, not repeated.

```diff
@@ header comment
-// - Handles (BaaProfile*, BaaTab*) are opaque and owned by BaaCore. A call on
-//   a handle that is gone (closed, or the app is terminating) does nothing and
-//   returns NULL/0/false, but only until its address is reused: a new tab or
-//   profile may get the address of a closed one. Never use a handle after
-//   tab_closed, or after your own baa_tab_close / baa_profile_close; drop it
-//   there.
+// - A tab is a BaaTabId: unique for the life of the process, never reused, 0
+//   is "none". Every id gets tab_closed exactly once (also for baa_tab_close and
+//   baa_profile_close, then inside that call). A call with a closed id does
+//   nothing and returns 0/false.
+// - Ask ids (dialog, permission, context_menu), download ids and Chrome's
+//   tab-group keys come from the same counter as tab ids.
+// - BaaProfile* is opaque, valid until baa_profile_close (its tabs get
+//   tab_closed first) or app termination.
@@
-#define BAA_ABI_VERSION 5
+#define BAA_ABI_VERSION 6
 
 typedef struct BaaProfile BaaProfile;
-// Any page baa shows: a tab in a Browser, an extension popup, or a DevTools
-// front-end. Calls that only make sense for a tab (baa_tab_activate) do
-// nothing on the others.
-typedef struct BaaTab BaaTab;
+// Any page baa shows: a tab in a Browser, an extension popup, or a DevTools
+// front-end. Calls that only make sense for a tab (baa_tab_activate,
+// baa_tab_set_group) do nothing on the others.
+typedef uint64_t BaaTabId;
@@ BaaTabState
   bool audible;
   bool secure;  // committed over valid TLS (security level SECURE)
+  bool crashed;  // the renderer is gone (crash, killed, "Exit page"); a reload
+                 // brings it back
 } BaaTabState;
@@ BaaDownload
 typedef struct {
-  uint32_t id;           // unique per run; baa_download_cancel takes it
+  uint64_t id;           // from the shared counter; baa_download_cancel takes it
@@
+enum {
+  BAA_CLOSED_BY_CHROME = 0,  // window.close(), chrome.tabs.remove, a popup closing itself
+  BAA_CLOSED_BY_QUIT = 1,    // the app is quitting (keep it in the session)
+  BAA_CLOSED_BY_APP = 2,     // baa_tab_close or baa_profile_close
+};
+
+enum {
+  BAA_DOCK_RIGHT = 0,
+  BAA_DOCK_BOTTOM = 1,
+  BAA_DOCK_LEFT = 2,
+  BAA_DOCK_UNDOCKED = 3,  // devtools_dock only: the user picked a window
+};
+
+// A question the page or Chrome waits on. JS dialogs (ALERT..BEFOREUNLOAD_RELOAD)
+// leave title and the labels "" (the app words them); Chrome's own fill them in
+// the UI language.
+typedef struct {
+  int kind;                  // BAA_DIALOG_*
+  const char* message;       // may hold '\n'-separated lines
+  const char* default_text;  // PROMPT only
+  const char* title;
+  const char* ok_label;
+  const char* cancel_label;
+  const char* check_label;   // a checkbox's label; "" for none
+} BaaDialog;
+
+typedef struct {
+  bool ok;               // OK / Allow / Open / Sign in / Exit page; false: Cancel, Deny, Wait
+  const char* text;      // PROMPT's text, AUTH's username; NULL means ""
+  const char* password;  // AUTH only
+  bool checked;          // the dialog's checkbox
+} BaaAnswer;
+
+// One omnibox match.
+typedef struct {
+  const char* url;          // where it goes
+  const char* contents;     // the main text (a URL, search terms, "= 4")
+  const char* description;  // the second text (a page title), "" if none
+  bool search;              // a search or calculator result, not a page
+} BaaMatch;
@@ BaaCallbacks
   bool (*key)(void* ctx, void* ns_event, bool after_page);
-  void (*tab_created)(void* ctx, BaaTab* tab, BaaTab* opener, ...);
+  void (*tab_created)(void* ctx, BaaTabId tab, BaaTabId opener, BaaProfile* profile,
+                      const char* url, bool background, bool user_gesture);
   (tab_state, tab_favicon: BaaTabId)
-  void (*dialog)(void* ctx, BaaTab* tab, uint64_t id, int kind,
-                 const char* message, const char* default_text);
-  void (*dialog_closed)(void* ctx, BaaTab* tab, uint64_t id);
-  void (*tab_closed)(void* ctx, BaaTab* tab);
+  // A page dialog or one of Chrome's (auth, external protocol, unresponsive
+  // page, extension install). The asker waits until baa_answer(id, ...) or
+  // request_closed(id).
+  void (*dialog)(void* ctx, BaaTabId tab, uint64_t id, const BaaDialog* dialog);
+  // Exactly once per id; `reason` BAA_CLOSED_*. The id is dead after this
+  // returns; the tab's NSView stays valid until then.
+  void (*tab_closed)(void* ctx, BaaTabId tab, int reason);
   void (*permission)(void* ctx, BaaTabId tab, uint64_t id, const char* origin,
                      uint32_t kinds);
-  void (*permission_closed)(void* ctx, BaaTab* tab, uint64_t id);
+  // Chrome withdrew the dialog or permission ask `id` unanswered (navigation,
+  // tab closed, page responsive again). Never after an answer, never twice.
+  void (*request_closed)(void* ctx, BaaTabId tab, uint64_t id);
   (context_menu, download, resized: BaaTabId)
-  void (*page_message)(void* ctx, BaaTab* tab, const char* message);
-  void (*fetch_done)(void* ctx, uint64_t id, bool ok, const char* body,
-                     size_t len);
+  // The DevTools front-end `front` was moved with its own dock menu
+  // (BAA_DOCK_*).
+  void (*devtools_dock)(void* ctx, BaaTabId front, int side);
+  // The page went fullscreen (true; Chrome already counts the window as
+  // fullscreen) or left it (false).
+  void (*fullscreen)(void* ctx, BaaTabId tab, bool on);
+  // baa_tab_find's progress: the active match (1-based, 0 for none) and the
+  // number found so far.
+  void (*find_result)(void* ctx, BaaTabId tab, int active, int count);
+  // Not baa_tab_set_group (an extension) changed the tab's group: `group` is the
+  // app's key, or a new key with the top bit set; 0 for none. `title` is the
+  // group's title.
+  void (*tab_group)(void* ctx, BaaTabId tab, uint64_t group, const char* title);
+  // Matches for baa_profile_autocomplete(id), best first, replacing the last
+  // list for id; `done` on the last one. Never inside the call.
+  void (*autocomplete)(void* ctx, uint64_t id, const BaaMatch* matches,
+                       size_t count, bool done);
   void (*clear_done)(void* ctx, uint64_t id);
   void (*load_unpacked_done)(void* ctx, uint64_t id, bool ok, const char* message);
 } BaaCallbacks;
@@ Profiles
-// GET `url` through the profile's network stack without cookies;
-// fetch_done(id, ...) later, never inside this call.
-BAA_EXPORT void baa_profile_fetch(BaaProfile* profile, const char* url,
-                                  uint64_t id);
+// Starts Chrome's omnibox for `text` (history, search suggestions,
+// calculator); results by autocomplete(id, ...). A later call stops earlier
+// ones (they send nothing more); "" only stops.
+BAA_EXPORT void baa_profile_autocomplete(BaaProfile* profile, uint64_t id,
+                                         const char* text);
@@ Extensions / Tabs: every BaaTab* becomes BaaTabId (0 = none):
-BAA_EXPORT BaaTab* baa_extension_popup(...)  / baa_tab_create(...)
+BAA_EXPORT BaaTabId baa_extension_popup(...) / baa_tab_create(...)
@@
-// Highlights `text`; repeating the same text moves to the next (or previous)
-// match. Empty `text` ends the search and clears the highlight.
+// Highlights `text`; repeating the same text moves to the next (or previous)
+// match; counts come by find_result. Empty `text` ends the search and clears
+// the highlight.
@@
-BAA_EXPORT BaaTab* baa_tab_open_devtools(BaaTab* tab, bool inspect, int x,
-                                         int y);
+// ... `side` (BAA_DOCK_RIGHT/BOTTOM/LEFT) preselects the front-end's dock menu;
+// a change there comes back as devtools_dock.
+BAA_EXPORT BaaTabId baa_tab_open_devtools(BaaTabId tab, bool inspect, int x,
+                                          int y, int side);
+
+// The user left page fullscreen through the app (the window left macOS
+// fullscreen, a Space switch). fullscreen(tab, false) follows.
+BAA_EXPORT void baa_tab_exit_fullscreen(BaaTabId tab);
+
+// Puts the tab in the tab group `group` (the app's key, top bit clear; or a
+// key from tab_group) titled `title`, making the group if the key has none.
+// 0 takes it out of its group. No tab_group callback for this.
+BAA_EXPORT void baa_tab_set_group(BaaTabId tab, uint64_t group,
+                                  const char* title);
@@ Dialogs
 enum {
   BAA_DIALOG_ALERT = 0,
   BAA_DIALOG_CONFIRM = 1,
   BAA_DIALOG_PROMPT = 2,
   BAA_DIALOG_BEFOREUNLOAD = 3,
   BAA_DIALOG_BEFOREUNLOAD_RELOAD = 4,
+  BAA_DIALOG_AUTH = 5,               // a username and a password field
+  BAA_DIALOG_EXTERNAL_PROTOCOL = 6,  // a link wants an app
+  BAA_DIALOG_UNRESPONSIVE = 7,       // OK ends the page, Cancel waits
+  BAA_DIALOG_EXTENSION_INSTALL = 8,  // also re-enable and permissions.request
 };
-BAA_EXPORT void baa_dialog_answer(uint64_t id, bool ok, const char* text);
-BAA_EXPORT void baa_permission_answer(uint64_t id, bool allow);
+// Answers a `dialog` or `permission` ask (for a permission, `ok` allows).
+// Unknown, answered or withdrawn ids do nothing.
+BAA_EXPORT void baa_answer(uint64_t id, const BaaAnswer* answer);
-BAA_EXPORT void baa_download_cancel(uint32_t id);
+BAA_EXPORT void baa_download_cancel(uint64_t id);
```

The `ffi.rs` mirror and `header_matches` gain the constants `CLOSED_*`, `DOCK_*` and `DIALOG_AUTH…EXTENSION_INSTALL`.

## 4. UI contract (`crates/baa-web/src/lib.rs` diff)

```diff
 pub enum DialogKind {
     Alert, Confirm, Prompt, BeforeUnload { is_reload: bool },
+    /// HTTP auth: a username and a password field.
+    Auth,
+    /// A link wants an app (mailto:, zoommtg:).
+    ExternalProtocol,
+    /// The page stopped responding: OK ends it, Cancel waits.
+    Unresponsive,
+    /// An extension asks to be added, re-enabled, or for more permissions.
+    ExtensionInstall,
 }
+
+/// A question the page or Chrome waits on, until [`PageHandle::answer_dialog`]
+/// or [`PageEvent::AskClosed`]. JS dialogs leave `title` and the labels empty.
+#[derive(Debug, Clone, PartialEq)]
+pub struct Dialog {
+    pub id: u64,
+    pub kind: DialogKind,
+    pub message: String,
+    pub default_text: String,
+    pub title: String,
+    pub ok_label: String,
+    pub cancel_label: String,
+    /// Empty: no checkbox.
+    pub check_label: String,
+}
+
+#[derive(Debug, Clone, Default, PartialEq)]
+pub struct DialogAnswer {
+    pub ok: bool,
+    /// A prompt's text, or the username.
+    pub text: String,
+    pub password: String,
+    pub checked: bool,
+}
+
+/// One of Chrome's omnibox matches.
+#[derive(Debug, Clone, PartialEq)]
+pub struct Suggestion {
+    pub url: String,
+    pub text: String,
+    pub description: String,
+    /// A search or calculator result, not a page.
+    pub search: bool,
+}
 
 pub struct Download {
-    pub id: u32,
+    pub id: u64,
 
 pub enum PageEvent {
     ...
-    PermissionDismissed(u64),
-    Dialog { kind: DialogKind, message: String, default_text: String },
-    DialogClosed,
+    Dialog(Dialog),
+    /// The permission ask or dialog `id` went away unanswered.
+    AskClosed(u64),
+    /// The page went fullscreen, or left it. While on, the UI shows the page alone.
+    Fullscreen(bool),
+    /// Find in page: the active match (1-based, 0 for none) and the count so far.
+    FindResult { active: u32, total: u32 },
+    /// The renderer is gone; `reload` brings it back.
+    Crashed(bool),
+    /// An extension put the tab in group `group` (a key from `set_group`, or a
+    /// new one) titled `title`, or took it out (`None`).
+    GroupChanged { group: Option<u64>, title: String },
     DevToolsDock(String),   // unchanged; now from the typed callback
 }
 
 pub trait PageHandle {
-    fn answer_dialog(&self, ok: bool, text: &str);
+    fn answer_dialog(&self, id: u64, answer: DialogAnswer);
+    /// The user left page fullscreen through the app (window left macOS fullscreen, Space switch).
+    fn exit_fullscreen(&self);
+    /// Chrome's tab group for this tab, so extensions see its folder; `None`
+    /// takes it out. The UI's keys keep the top bit clear.
+    fn set_group(&self, group: Option<(u64, &str)>);
 }
 
 pub trait WebHost {
-    fn fetch(&self, profile: ProfileId, url: &str, done: Box<dyn FnOnce(Option<String>)>);
+    /// Chrome's omnibox matches for `text` in `profile`, best first. `update`
+    /// runs each time the list improves; the next call (or empty `text`) ends
+    /// the previous one, whose `update` is dropped.
+    fn autocomplete(&self, profile: ProfileId, text: &str, update: Box<dyn FnMut(Vec<Suggestion>, &mut App)>);
-    fn cancel_download(&self, id: u32);
+    fn cancel_download(&self, id: u64);
 }
```
`answer_permission(id, allow)` stays. Core maps it to `baa_answer(id, {ok: allow})`.

## 5. Patches

One new patch, `chromium/patches/baa/dialog-hooks.patch` (added to `series` after
`color-scheme-hook.patch`). Every hook is about 3 lines plus the pointer's definition:

| File | Hook |
|---|---|
| `chrome/browser/baa_hooks.h` | Declares `BaaLoginHandlerFn g_baa_login_handler` (`std::unique_ptr<LoginHandler>(const net::AuthChallengeInfo&, content::WebContents*, content::LoginDelegate::LoginAuthRequiredCallback&)`), `BaaExternalProtocolFn g_baa_external_protocol` (`bool(const GURL&, content::WebContents*, const std::optional<url::Origin>&, content::WeakDocumentPtr, const std::u16string&)`) and `BaaInstallDialogFn g_baa_install_dialog` (`bool(std::unique_ptr<ExtensionInstallPromptShowParams>&, ExtensionInstallPrompt::DoneCallback&, std::unique_ptr<extensions::InstallPromptData>&)`). Forward declarations only. |
| `chrome/browser/ui/views/login_handler_views.cc:230` | `if (g_baa_login_handler) if (auto h = g_baa_login_handler(auth_info, web_contents, auth_required_callback)) return h;` |
| `chrome/browser/ui/views/external_protocol_dialog.cc:95` | after the `program_name.empty()` return: `if (g_baa_external_protocol && g_baa_external_protocol(url, web_contents, initiating_origin, initiator_document, program_name)) return;` |
| `chrome/browser/ui/views/extensions/extension_install_dialog_view.cc:85` | `if (g_baa_install_dialog && g_baa_install_dialog(show_params, done_callback, prompt)) return;` |

Nothing else needs a patch. The hung renderer uses `SetUserData`. Fullscreen uses our own
`ExclusiveAccessContext`. Groups use the tab-strip observer. Find uses the observer. Autocomplete
uses the public controller.

**A patch change means a fresh prep** (`local-build.sh`: any change under `chromium/patches` re-preps
the tree). So the patch is one package (WP-P), built once, early.

## 6. Verification policy

- **Build only.** C++ packages run `chromium/local-build.sh`. It takes
  `/Users/Shared/hb-local-build.lock` and refuses while `pgrep -x siso` finds a build. Check
  `pgrep -x siso` first. Never run two Chromium builds, and never `gn gen` by hand in the tree.
  - If the lock is held, wait for it and do not delete it.
  - Then `cargo check -p baa-chromium` for the harness.
- Rust packages run `cargo check -p <crate>`, `cargo test -p <crate>` and
  `cargo clippy -p <crate> -- -D warnings`.
- Nobody runs `crates/baa-chromium/ci.sh` or `ci.py`, launches the app, runs `cargo bundle`, or
  drives a GUI.
  - Harness files (`tests/*.txt`, `*.expect`) are written to the house format (see `dialogs.txt` /
    `dialogs.expect`) and not run.
  - The user runs them, and the manual pass, after integration.
- Only WP-Z runs `cargo build -p baa`, once at the end.
- No commits unless the lead asks. Each package ends with a short report: files, tests added, and
  anything left as `ponytail:`.

## 7. Work packages

Order:
1. WP-0 first.
2. Then, in parallel: WP-P, WP-C1a, WP-R1, WP-U1–U4.
3. WP-C1a and WP-R1 **merge together** (they are the two sides of ABI 6).
4. Then WP-C1b and WP-C3/C4/C5 can be edited in parallel. WP-C2 waits for WP-C1b and WP-P.
5. WP-R2 runs after WP-R1.
6. WP-Z comes last.

C++ edits can run in parallel; C++ **builds** serialize on the lock. Run at most two C++ packages at
once.

| WP | Owner (CLAUDE.md) | Depends on | Parallel with | Shares files with |
|---|---|---|---|---|
| 0 Contract | lead (cross-crate, contract only) | — | — | — |
| P Dialog hooks patch | engine | 0 | C1a, R1, U* | none (patches only) |
| C1a ABI 6 + ids + dock (C++) | engine | 0 | P, R1, U* | needs R1's first commit (`ffi.rs`) for the harness |
| R1 ABI 6 + ids + dock (Rust core_host) | baa-web | 0 | P, C1a, U* | — |
| C1b Request registry (C++) | engine | C1a+R1 merged | C3, C4, C5 | `tab.mm` (`Forget` only) with C3 |
| C2 Chrome dialogs (C++) | engine | C1b, P | C3, C4, C5 | `BUILD.gn` (one line), harness `main.rs` |
| C3 Fullscreen, find, crash (C++) | engine | C1a+R1 | C1b, C2, C5 | `window.mm`/`window.h` with C4: different functions, merge with care |
| C4 Tab groups (C++) | engine | C1a+R1 | C1b, C2, C5 | `window.mm`/`window.h` with C3 |
| C5 Autocomplete (C++) | engine | C1a+R1 | C1b, C2, C3, C4 | `BUILD.gn`, `profile.mm` |
| R2 M3 plumbing (Rust core_host) | baa-web | R1 | C*, U* | — |
| U1 Dialog sheets | baa-ui | 0 | all | `input.rs` |
| U2 Command bar | baa-ui | 0 | all | — |
| U3 Fullscreen, find count, crash | baa-ui | 0 | all but U4 on `browser.rs` | `browser.rs` with U4 (different functions) |
| U4 Groups | baa-ui | 0 | all but U3 on `browser.rs` | `browser.rs` with U3 |
| Z Integration | integration | all | — | `crates/baa` only if needed |

**WP-0 Contract** (lead; Rust only, against ABI 5 still)
- `crates/baa-web/src/lib.rs`: §4 exactly.
- `core_host` bodies so it compiles against ABI 5:
  - `answer_dialog(id, a)` → `dialog_answer(id, a.ok, a.text)`;
  - `exit_fullscreen` and `set_group` do nothing;
  - `autocomplete` drops `update`;
  - `PageEvent::Dialog(Dialog{..})` is built from the ABI 5 callback, with `Shared.dialog`
    supplying the id until WP-R1;
  - `dialog_closed`/`permission_closed` → `AskClosed(id)`;
  - `fetch` and the `fetches` map are deleted;
  - `Download.id as u64`.
- `crates/baa-ui/src/mock.rs`: the new methods. It records `answered: Vec<(u64, DialogAnswer)>`,
  `groups: Vec<(u64 /*page*/, Option<(u64, String)>)>`, `autocompletes: Vec<String>` and
  `exited_fullscreen`. A `MockHost::suggest(Vec<Suggestion>)` runs the last `update`.
- baa-ui call sites, compile only:
  - `prompts.rs` `on_ask`/`answer_dialog` move to `Dialog(d)`/`AskClosed`/`answer_dialog(id, ..)`;
  - `browser.rs` `on_page_event` gets no-op arms for the new events;
  - `command_bar.rs` `request_suggestions` calls `host.autocomplete` and keeps
    `search`-matches' `text` as today's `suggestions`;
  - download ids become `u64`.
- `crates/baa-model/src/settings.rs`: delete `suggest_url`, `parse_suggestions` and their test, and
  the `lib.rs` re-export.
- `crates/baa-privacy/src/chromium.rs`: drop `"search.suggest_enabled"` from `PREFS_OFF`.
- Done when: `cargo check` and `cargo test` pass for baa-web, baa-ui, baa-model and baa-privacy,
  and clippy is clean.
- Tests: the existing ones, updated (`dialogs.rs`, `tests.rs` dialog cases with ids).

**WP-P Dialog hooks patch** (engine)
- Files: `chromium/patches/baa/dialog-hooks.patch`, `chromium/patches/series`. §5 exactly. The
  pointers stay null, so behaviour is unchanged.
- If `gn check` refuses `chrome/browser/ui/views/*` including `chrome/browser/baa_hooks.h`, fall
  back to a header beside each hooked file, as `baa_context_menu_hook.h` does, and say so in the
  report.
- Done when: `local-build.sh` (a fresh prep) builds.

**WP-C1a ABI 6, tab ids, closed reasons, dock in C++** (engine)
- `chromium/baa/baa.h`: §3 in full, including the M3 functions.
- `tab.mm`/`profile.mm`/`extensions.mm`/`devtools.mm`/`downloads.mm`/`dialogs.mm`/`permissions.mm`/`menu.mm`/`window.mm`/`internal.h`:
  - `BaaTab` gets `const uint64_t id` from `NextId()`. `TabFromId(id)` replaces pointer lookups;
    `WeakTab` is unchanged. Every ABI entry takes and returns `BaaTabId`.
  - `Forget(t, reason)` always sends `tab_closed(id, reason)` (if the app knew the tab):
    `BY_APP` from `baa_tab_close`/`CloseTabsOf`, `BY_QUIT` when `baa_is_quitting()`, else
    `BY_CHROME`.
  - Download ids come from `NextId()`.
  - `BaaTabState.crashed` is filled in, and `PrimaryMainFrameRenderProcessGone` calls `SendState`.
- Dialogs: send `BaaDialog` (JS kinds, wording fields ""). `baa_answer` routes to the existing
  dialog, app-modal and permission maps (WP-C1b replaces them). `dialog_closed` and
  `permission_closed` become `request_closed`.
- DevTools (refactor step 11): `baa_tab_open_devtools(…, side)`.
  - `devtools.mm` keeps the front's side. When a front finishes loading a new `devtools://`
    document, it evals the dock script (moved verbatim from `core_host/devtools.rs`, both halves),
    with the side's name.
  - `BaaTab::OnDidAddMessageToConsole` handles `baa:devtools-dock:<side>` only for DevTools fronts,
    and sends `devtools_dock(front, BAA_DOCK_*)`.
  - `page_message` is gone.
- Stubs for the M3 functions: `baa_tab_exit_fullscreen`, `baa_tab_set_group` and
  `baa_profile_autocomplete` do nothing; the new callbacks are never sent. `net.mm` and
  `baa_profile_fetch` are deleted, along with the `BUILD.gn` line.
- Harness (`crates/baa-chromium/src/main.rs`, after rebasing on WP-R1's `ffi.rs` commit):
  - new callback signatures;
  - the log lines: `tab_closed tab=N reason=R`, `request_closed tab=N id=I`, `devtools_dock tab=N side=S`;
  - `answer <id> <ok> [text] [password] [checked]` through `baa_answer`;
  - `devtools <inspect> <x> <y> <side>`;
  - `fetch` is removed.
- Harness `tests/`: update `dialogs.expect` (`dialog_closed` → `request_closed`), `misc2`/`all`
  (drop fetch lines), and `close.expect` (now `tab_closed … reason=2` for `close`).
- Done when: `local-build.sh` builds and `cargo check -p baa-chromium` passes.

**WP-R1 ABI 6 on the Rust side** (baa-web)
- First commit, alone: `core_host/ffi.rs` mirrors §3 (`pub type TabId = u64`, `Dialog`, `Answer`,
  `Match`, the new constants in `header_matches`). WP-C1a's harness uses it.
- `dispatch.rs`, `page.rs`, `host.rs`, `core_host.rs`, `devtools.rs`, `slot.rs`, `tests/*`:
  - `*mut Tab` → `TabId` (0 = none) in `Shared.tab`, `Dispatch.tabs`, `Pending` (drop `seq`; key
    the expiry by id).
  - `tab_closed(tab, reason)`: `BY_APP` only releases; `BY_QUIT` releases without
    `PageEvent::Closed`; `BY_CHROME` as today.
  - Delete `forget_profile`, because `profile_close` now reports each tab.
  - `dialog` → `PageEvent::Dialog(Dialog)` with the id. `Shared.dialog` is deleted.
  - `request_closed` → `AskClosed(id)`.
  - `answer_dialog` and `answer_permission` → `baa_answer`.
  - `devtools_dock` → `DevToolsDock(name)`. `create_devtools` passes `side`.
  - Delete `devtools.rs`, the dock script, `docked_url` and the `apply` eval.
  - The new M3 callbacks are no-op handlers (WP-R2 fills them).
- Tests (fake Core):
  - `tab_closed(BY_QUIT)` emits no `Closed`;
  - `profile_close` reporting `tab_closed(BY_APP)` for two tabs leaves `Dispatch.tabs` empty and
    emits nothing;
  - a `dialog` with `BAA_DIALOG_AUTH` → `Dialog { kind: Auth, title, .. }`;
  - `answer_dialog(7, {ok, text:"u", password:"p"})` → one `answer 7 1 u p 0`;
  - `request_closed` → `AskClosed`;
  - `devtools_dock(BAA_DOCK_LEFT)` → `DevToolsDock("left")`;
  - an id used after `tab_closed` is ignored.
- Done when: `cargo test -p baa-web` passes (including `header_matches` against WP-C1a's `baa.h`),
  and clippy is clean.

**WP-C1b Request registry** (engine; no ABI change)
- `requests.mm`/`internal.h`: §2.1 `Request`, `FindRequest`, `WithdrawRequest`, `DropRequestsOf`.
- `dialogs.mm` (`DialogView`, `AppModal`), `permissions.mm` (`Prompt`) and `menu.mm` (menus)
  derive from `Request`. Their maps `Dialogs()`, `AppModals()`, `Prompts()` and `Menus()` are
  deleted.
- `tab.mm` `Forget`: `DropRequestsOf(id, tell)` replaces `DropPermissionsOf`, `DropMenusOf` and
  `DropDialogsOf`.
- `baa_answer` is `if (auto* r = FindRequest(id)) r->Answer(*a);`. `baa_context_menu_done` is the
  same with `Command`.
- Done when: it builds, and the harness `dialogs`, `perm1`, `perm2`, `menu` and `leave` files need no
  change (behaviour-preserving).

**WP-C2 Chrome's dialogs in GPUI** (engine)
- New `asks.mm`:
  - `baa::LoginPrompt : LoginHandler, Request`;
  - the external-protocol request;
  - `baa::TabDialogs : TabDialogsViews` with an unresponsive request;
  - the install request, each per §2.2;
  - the three hook functions, set in `EnsureStarted` (`app_mac.mm`).
- `dialogs.mm` `AttachDialogs`/`ReturnDialogsToChrome` also swap `TabDialogs`. Add `asks.mm` to
  `BUILD.gn`, plus deps if `gn` asks.
- The install request: `disable_ui_for_tests` stays (post-install bubble, phase 4 #12).
- Harness:
  - `download-server.py` gains `/auth`: `401` + `WWW-Authenticate: Basic realm="baa"` until
    `Authorization` is `Basic dTpw` (u:p).
  - New `tests/asks.txt` + `asks.expect`:
    - auth: `load http://127.0.0.1:PORT/auth` → `must: baa: dialog tab=0 id=\d+ kind=5`; `answer <id> 1 u p` → `title=` of the ok page;
    - external protocol: `geval location.href='mailto:a@b.test'` → `must: kind=6`; then `answer <id> 0` (never 1 in CI: it would launch Mail);
    - hung: `eval setTimeout(()=>{for(;;){}},100)`, `sleep 20000` → `must: kind=7`; `answer <id> 1` → `must: tab_state .*crashed=1`;
    - install: the test extension's page runs `geval chrome.permissions.request({permissions:['bookmarks']})` (add `bookmarks` to `optional_permissions` in `test-ext/manifest.json`) → `must: kind=8`; `answer <id> 0`.
- Done when: it builds, `cargo check -p baa-chromium` passes, and the test files are written.

**WP-C3 Fullscreen, find counts** (engine)
- `window.mm`/`window.h`: `fullscreen_`, `EnterFullscreen`/`ExitFullscreen`/`IsFullscreen` per
  §2.3, and `WindowFullscreenStateChanged` posted.
- `tab.mm`: `baa_tab_exit_fullscreen`, plus the `FindResultObserver` on `BaaTab` (add it in `Attach`
  and remove it in `Detach`) → `find_result`.
- Harness: `fsexit` command. `tests/fullscreen.txt`/`.expect`:
  - `geval document.body.requestFullscreen()` → `must: fullscreen tab=0 on=1`;
  - `eval document.title='fs:'+!!document.fullscreenElement` → `title="fs:true"`;
  - `fsexit` → `on=0`.
  - `find Example` on example.com → `must: find_result tab=0 active=1 count=1`.
- Done when: it builds and the harness checks.

**WP-C4 Tab groups** (engine)
- New `groups.mm`: per Browser, the key ↔ `TabGroupId` map, the guard flag, `baa_tab_set_group`,
  and the two observer forwards from `window.mm` (`TabGroupedStateChanged`, `OnTabGroupChanged`)
  per §2.4.
- Harness: `group <tab> <key> <title>`, plus `log: tab_group tab=N group=G title=T`.
  `tests/groups.txt`/`.expect`:
  - `group 0 7 Work`, then test-ext `probe` / CDP eval `chrome.tabGroups.query({})` → title `Work`;
  - the extension's `chrome.tabs.group({tabIds:[id]})` → `must: tab_group tab=1 group=9223372036854775\d+ title=""`;
  - `group 0 0 -` → the query is empty.
- Done when: it builds and the harness checks.

**WP-C5 Autocomplete** (engine)
- New `autocomplete.mm`: the controller per §2.5 and `baa_profile_autocomplete`. The controller is
  owned by `BaaProfile` (`profile.mm`: a `std::unique_ptr` member, reset in
  `OnProfileWillBeDestroyed`).
- `BUILD.gn` deps: `//components/omnibox/browser`, plus whatever `gn` names for
  `chrome/browser/autocomplete`.
- Harness: `ac <id> <text>`, logging `autocomplete id=I n=N done=D first="url|contents|search"`.
  `tests/autocomplete.txt`/`.expect`:
  - after `load https://example.com/`, `ac 1 example` → `must: autocomplete id=1 .*example.com`;
  - `ac 2 2+2` → `must: autocomplete id=2 .*search=1`;
  - `ac 3 ""` → no later `id=2` line (`count: 0` after it).
- Done when: it builds and the harness checks.

**WP-R2 M3 plumbing in core_host** (baa-web)
- `fullscreen` → `PageEvent::Fullscreen`. `exit_fullscreen` → `tab_exit_fullscreen`.
- `find_result` → `FindResult`.
- `tab_state.crashed` diff → `Crashed(bool)`, in `emit_changes`.
- `tab_group` → `GroupChanged` (0 → `None`). `set_group` → `tab_set_group(id, key, title)`, with
  `None` → key 0.
- `autocomplete`:
  - `Dispatch.autocomplete: RefCell<Option<(u64, Box<dyn FnMut(..)>)>>`. A new call replaces it and
    calls `profile_autocomplete(id, text)`. The callback queues `update` only for the current id.
  - `BaaMatch` → `Suggestion`.
- `settings.rs`: write `search.suggest_enabled` = `settings.search_suggest`. Its `writes` test gets
  the new row.
- Tests (fake Core):
  - `fullscreen(tab, true)` → `Fullscreen(true)`;
  - `find_result(tab,2,5)` → `FindResult{2,5}`;
  - a state with `crashed` flips → `Crashed(true)`, then `Crashed(false)` once;
  - `set_group(Some((3,"A")))` → `tab_set_group <id> 3 A`;
  - `tab_group(tab, 1<<63|5, "X")` → `GroupChanged{Some(..),"X"}`;
  - `autocomplete` twice → the first `update` never runs after the second call; `done` results
    arrive once per callback;
  - `writes(Settings{search_suggest:false})` has `search.suggest_enabled=false`.
- Done when: `cargo test -p baa-web` and clippy pass.

**WP-U1 Dialog sheets** (baa-ui: `prompts.rs`, `input.rs`, new `tests_dialogs.rs`)
- `DialogAsk` holds `Dialog`, a second `TextField` (password) and `checked`.
- Title: `dialog.title` if not empty, else today's wording. Labels: Chrome's if not empty.
- `Auth`: two fields, Tab moves between them, and the password field renders as bullets
  (`TextField::render` gets a `masked: bool`).
- A checkbox row when `check_label` is not empty. `message` lines render as a list for
  `ExtensionInstall`.
- `Unresponsive`: Esc = Wait (cancel).
- `answer_dialog(id, DialogAnswer{ok, text, password, checked})`. `AskClosed(id)` removes a
  permission ask or a dialog with that id.
- Tests (TestAppContext + MockHost):
  - an `Auth` dialog → type "u", Tab, "p", Enter → `mock.answered == [(id, {ok, "u", "p", false})]`;
  - an `ExternalProtocol` dialog with a check label → the click toggles → `checked: true`;
  - `AskClosed(other id)` keeps the sheet, `AskClosed(id)` removes it;
  - a JS `Confirm` dialog still says "<site> says".

**WP-U2 Command bar on autocomplete** (baa-ui: `command_bar.rs`)
- `Bar.matches: Vec<Suggestion>` replaces `suggestions` and the typed-mode use of `history`.
- `request_suggestions` → `host.autocomplete(profile, query, update)`, after `SUGGEST_DELAY` as now,
  and only when the bar's `seq` is current. Empty text calls `autocomplete("")`.
- `rows()`:
  - the typed row first, as today;
  - search matches as suggestion rows (≤ `MAX_SUGGESTIONS`);
  - the scored tabs, Spaces, commands, pages and settings;
  - then page matches as history rows (≤ `MAX_HISTORY`), without URLs of open tabs and without the
    typed URL.
- History mode keeps baa-model.
- Tests (unit, `rows`): `"gh"` with matches `[search "gh cli", page "https://github.com"]` → typed,
  suggestion, …, history row. The page match equal to an open tab's URL is dropped.
- TestAppContext: typing calls `mock.autocompletes` with the text after the delay, and a
  `mock.suggest(..)` update shows its rows.

**WP-U3 Fullscreen, find count, crash** (baa-ui: `browser/page_area.rs`, `browser.rs` find bar, `browser/header.rs` if needed)
- `Fullscreen(true)` on the active tab:
  - the page fills the window (no sidebar, header or card insets);
  - `window.toggle_fullscreen()` if the window is not already fullscreen, remembering that baa did
    it (open question 4).
- `Fullscreen(false)` restores, leaving macOS fullscreen only if baa entered it.
- A Space switch, a tab switch, or the window leaving macOS fullscreen while a page is fullscreen →
  `page.exit_fullscreen()`.
- `FindResult` → the find bar shows `active/total` (`0/0` in muted ink).
- `Crashed(true)` → an overlay over the page area: "This page crashed" + Reload (`page.reload()`).
- Tests:
  - `Fullscreen(true)` → `b.fullscreen == Some(tab)`; switching tab → `mock.exited_fullscreen`;
  - `FindResult{3,12}` → the find bar state holds it;
  - `Crashed(true)` then the Reload click → `mock` recorded `reload`.

**WP-U4 Groups** (baa-ui: new `groups.rs`, `browser.rs` `sync` + `on_page_event` arm, new `tests_groups.rs`)
- Per §2.4 UI rule. State `Groups { applied: HashMap<TabId, Option<(u64, String)>>, chrome_keys: HashMap<u64, FolderId> }`.
- Tests (MockHost):
  - a live tab moved into folder F "Work" → `mock.groups` has `(page, Some((F.0, "Work")))` once;
    the second `sync` sends nothing;
  - folder renamed → one call with the new title;
  - `GroupChanged{Some(1<<63|9), "News"}` → a new folder "News" holds the tab (pinned), and no
    `set_group` echo;
  - a second tab with the same key joins the same folder;
  - `GroupChanged{None}` → the tab leaves the folder.

**WP-Z Integration** (integration)
- Merge order: 0 → (C1a+R1) → C1b → C2–C5, R2, U1–U4.
- `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo build -p baa` once,
  and one `local-build.sh` on the merged tree.
- Hand the user the list of harness files to run (`asks`, `fullscreen`, `groups`, `autocomplete`,
  plus the updated `dialogs`, `close`, `misc2`, `all`, `devtools`) and the manual checks:
  - httpbin basic-auth;
  - a `zoommtg:` / `mailto:` link;
  - a Web Store install;
  - YouTube fullscreen;
  - find on a long page;
  - a `chrome://crash` tab;
  - an extension that groups tabs (any tab manager);
  - command bar ranking after a day of use.

## 8. Risks

| # | Risk | Cheapest test | Fallback |
|---|---|---|---|
| 1 | `gn check` rejects `chrome/browser/ui/views/*` including `chrome/browser/baa_hooks.h` | WP-P's build | A hook header per directory, like `baa_context_menu_hook.h` |
| 2 | `TabGroupSyncService`'s desktop delegate (`chrome/browser/ui/tabs/saved_tab_groups/tab_group_sync_delegate_desktop.cc`) or `SavedTabGroup*` listeners reach for a BrowserView when a group is made | `groups.txt`; grep the log for `Check failed` | A guard in `no-views-guards.patch`, or `--disable-features` for the saved-groups feature in `CORE_DISABLED_FEATURES` |
| 3 | Saved tab groups persist in the profile and pile up across launches | Look at `chrome://tab-group-sync-internals` (if present) after two runs | Disable the saved-groups feature (as #2) |
| 4 | The controller's `SearchProvider` sends suggest requests outside typing (prefetch), which the privacy proxy test would catch | CI privacy job on the merged engine | Set `search.suggest_enabled` only while the command bar is open |
| 5 | The hung-renderer timeout in the harness is longer than the `sleep` | `asks.txt` | Raise the sleep; `--disable-hang-monitor` is not an option (it removes the feature) |
| 6 | `LoginHandler` without a `BrowserView` derefs one (password manager's HTTP-auth path, `GetHttpAuthManagerForLogin`) | `asks.txt` auth step | Clear `login_model_data` in our `BuildViewImpl`; a guard line if needed |
| 7 | `requestFullscreen` from a background page (Space not shown) | manual | Exit at once if the tab is not active (UI) |
| 8 | Merging C3 and C4 on `window.mm` | — | C3 first, C4 rebases (both add separate overrides) |

## 9. Open questions for the user

1. **Groups direction.** If an extension groups tabs (`chrome.tabs.group`), baa puts them in a
   folder, which pins them. The other option is one-way only: folders show as groups, and extension
   changes are ignored and then overwritten. *Default: two-way, as above.*
2. **Command bar history per Space.** Chrome keeps history per profile, which here means per Space.
   Today the bar searches baa-model's global history. *Default: per Space, through
   AutocompleteController. History mode (⌘Y) and the Library stay global.*
3. **External protocol "Always allow"** (Chrome's per-origin remember checkbox). *Default: show it.*
4. **Page fullscreen.** Fill the window and also enter macOS fullscreen (as Chrome and Arc do), or
   only fill the window? *Default: enter macOS fullscreen if not already in it, and leave it when the
   page exits.*
5. **Deferrals.** These move to M4: `chrome.commands` (so 1Password's ⌘⇧X does not work yet),
   `chrome.action.openPopup`, badges and dynamic icons, the screen-share picker, and dangerous-download
   keep/discard. *Default: defer all of them. If one comes back, `chrome.commands` first.*
6. **Dialog wording.** Chrome's localized strings for its own dialogs, or baa's English copy?
   *Default: Chrome's, because they come localized and stay exact (permission lines).*
7. **Search suggestions through Chrome** (`search.suggest_enabled` follows the "suggestions"
   setting; same endpoints as today). *Default: yes.*

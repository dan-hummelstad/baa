// C ABI between BaaCore.framework (//baa) and the Rust shell (crates/baa-web,
// module `core_host`). Spec: PLAN-chromium-phase3.md.
//
// Rules for both sides:
// - Threading: every function and every callback runs on the main thread
//   (Chromium's UI thread). Nothing here is thread-safe.
// - A tab is a BaaTabId: unique for the life of the process, never reused, 0
//   is "none". Every id gets tab_closed exactly once (also for baa_tab_close and
//   baa_profile_close, then inside that call). A call with a closed id does
//   nothing and returns 0/false.
// - Ask ids (dialog, permission, context_menu), download ids and Chrome's
//   tab-group keys come from the same counter as tab ids.
// - BaaProfile* is opaque, valid until baa_profile_close (its tabs get
//   tab_closed first) or app termination.
// - Strings in: UTF-8, NUL-terminated, borrowed for the duration of the call.
// - Strings out: UTF-8, NUL-terminated, borrowed, valid only until the
//   callback (or the visitor call) returns. Copy what you keep.
// - Callbacks may run synchronously inside a baa_* call (e.g. tab_created
//   while baa_tab_load_url runs a page's script). The Rust side therefore only
//   queues work in a callback and never touches gpui state inline. The key
//   callback is the one exception (it must answer synchronously).
#ifndef BAA_BAA_H_
#define BAA_BAA_H_

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define BAA_EXPORT __attribute__((visibility("default")))

// Bumped on every ABI change; the Rust loader refuses a mismatch.
#define BAA_ABI_VERSION 6
BAA_EXPORT int baa_abi_version(void);

typedef struct BaaProfile BaaProfile;
// Any page baa shows: a tab in a Browser, an extension popup, or a DevTools
// front-end. Calls that only make sense for a tab (baa_tab_activate,
// baa_tab_set_group) do nothing on the others.
typedef uint64_t BaaTabId;

// What a tab looks like now. Sent whole on every change; the receiver diffs.
typedef struct {
  const char* url;          // visible URL
  const char* title;        // "" until the page has one
  const char* favicon_url;  // "" when the page has none (yet)
  // The tab's NSView. Changes only when Chrome swaps the tab's WebContents
  // (discard/reload); the receiver then puts the new view where the old was.
  void* view;
  bool loading;
  bool can_go_back;
  bool can_go_forward;
  bool audible;
  bool secure;  // committed over valid TLS (security level SECURE)
  bool crashed;  // the renderer is gone (crash, killed, "Exit page"); a reload
                 // brings it back
} BaaTabState;

// One row of a page's context menu, flattened depth-first. A SUBMENU row is
// followed by its items at depth + 1.
typedef struct {
  const char* label;  // "" for a separator; no '&' mnemonics
  int command;        // for baa_context_menu_done; -1 on separators/submenus
  uint8_t depth;
  uint8_t type;       // BAA_MENU_*
  bool enabled;
  bool checked;
} BaaMenuItem;

enum {
  BAA_MENU_COMMAND = 0,
  BAA_MENU_SEPARATOR = 1,
  BAA_MENU_SUBMENU = 2,
  BAA_MENU_INSPECT = 3,  // a COMMAND that is "Inspect" (Chrome would open DevTools)
};

// Permission kinds, as bits.
enum {
  BAA_PERMISSION_CAMERA = 1 << 0,
  BAA_PERMISSION_MICROPHONE = 1 << 1,
  BAA_PERMISSION_LOCATION = 1 << 2,
  BAA_PERMISSION_NOTIFICATIONS = 1 << 3,
  BAA_PERMISSION_CLIPBOARD = 1 << 4,
  BAA_PERMISSION_MIDI = 1 << 5,
  BAA_PERMISSION_OTHER = 1 << 6,  // any request type not above
  // Site settings only: never in a `permission` ask's `kinds`.
  BAA_PERMISSION_POPUPS = 1 << 7,
  BAA_PERMISSION_SOUND = 1 << 8,
  BAA_PERMISSION_JAVASCRIPT = 1 << 9,
  BAA_PERMISSION_COOKIES = 1 << 10,
  BAA_PERMISSION_AUTOMATIC_DOWNLOADS = 1 << 11,
  BAA_PERMISSION_INSECURE_CONTENT = 1 << 12,  // mixed content
  BAA_PERMISSION_SENSORS = 1 << 13,           // motion sensors
  BAA_PERMISSION_WINDOW_MANAGEMENT = 1 << 14,
};

// A site setting's state. Not every kind takes every state: popups, sound,
// javascript, insecure content and sensors take ALLOW and BLOCK only (their
// default is one of them, never ASK); SESSION_ONLY is for COOKIES only.
// BAA_SITE_UNSET is only ever an argument: it removes a rule.
enum {
  BAA_SITE_UNSET = -1,
  BAA_SITE_ASK = 0,
  BAA_SITE_ALLOW = 1,
  BAA_SITE_BLOCK = 2,
  BAA_SITE_SESSION_ONLY = 3,
};

typedef struct {
  uint64_t id;           // from the shared counter; baa_download_cancel takes it
  const char* url;
  const char* path;      // target file; "" until chosen
  int64_t received_bytes;
  int64_t total_bytes;   // -1 or 0 when unknown
  int state;             // BAA_DOWNLOAD_*
} BaaDownload;

enum {
  BAA_DOWNLOAD_IN_PROGRESS = 0,
  BAA_DOWNLOAD_COMPLETE = 1,
  BAA_DOWNLOAD_CANCELED = 2,  // also interrupted
};

enum {
  BAA_CLOSED_BY_CHROME = 0,  // window.close(), chrome.tabs.remove, a popup closing itself
  BAA_CLOSED_BY_QUIT = 1,    // the app is quitting (keep it in the session)
  BAA_CLOSED_BY_APP = 2,     // baa_tab_close or baa_profile_close
};

enum {
  BAA_DOCK_RIGHT = 0,
  BAA_DOCK_BOTTOM = 1,
  BAA_DOCK_LEFT = 2,
  BAA_DOCK_UNDOCKED = 3,  // devtools_dock only: the user picked a window
};

// A question the page or Chrome waits on. JS dialogs (ALERT..BEFOREUNLOAD_RELOAD)
// leave title and the labels "" (the app words them); Chrome's own fill them in
// the UI language.
typedef struct {
  int kind;                  // BAA_DIALOG_*
  const char* message;       // may hold '\n'-separated lines
  const char* default_text;  // PROMPT only
  const char* title;
  const char* ok_label;
  const char* cancel_label;
  const char* check_label;   // a checkbox's label; "" for none
} BaaDialog;

typedef struct {
  bool ok;               // OK / Allow / Open / Sign in / Exit page; false: Cancel, Deny, Wait
  const char* text;      // PROMPT's text, AUTH's username; NULL means ""
  const char* password;  // AUTH only
  bool checked;          // the dialog's checkbox
} BaaAnswer;

// One omnibox match.
typedef struct {
  const char* url;          // where it goes
  const char* contents;     // the main text (a URL, search terms, "= 4")
  const char* description;  // the second text (a page title), "" if none
  bool search;              // a search or calculator result, not a page
} BaaMatch;

typedef struct {
  // A ⌘/⌃ or Escape key-down in a page. `after_page` is false before the page
  // sees it and true when the page did not consume it. Return true to take
  // it. Synchronous: the event is decided by the return value.
  bool (*key)(void* ctx, void* ns_event, bool after_page);

  // Chrome made a tab baa did not ask for: window.open, target=_blank,
  // ⌘/middle-click, chrome.tabs.create, chrome.windows.create, an options
  // page. `opener` is the tab it came from, 0 if none (an extension, or a
  // tab baa does not know). `profile` is the BaaProfile it lives in, NULL if
  // it is a profile baa never opened (e.g. Chrome's primary incognito).
  // `background`: it did not become its window's active tab.
  // The tab is live and loading; baa adopts it or closes it.
  void (*tab_created)(void* ctx,
                      BaaTabId tab,
                      BaaTabId opener,
                      BaaProfile* profile,
                      const char* url,
                      bool background,
                      bool user_gesture);

  // Any field of the tab's state may have changed.
  void (*tab_state)(void* ctx, BaaTabId tab, const BaaTabState* state);

  // The favicon at `icon_url` as PNG (2x when the site has it). `png` is
  // borrowed for the call.
  void (*tab_favicon)(void* ctx,
                      BaaTabId tab,
                      const char* icon_url,
                      const uint8_t* png,
                      size_t png_len);

  // A page dialog or one of Chrome's (auth, external protocol, unresponsive
  // page, extension install). The asker waits until baa_answer(id, ...) or
  // request_closed(id). beforeunload comes too (kinds BAA_DIALOG_BEFOREUNLOAD*).
  void (*dialog)(void* ctx, BaaTabId tab, uint64_t id, const BaaDialog* dialog);

  // Chrome closed the tab (window.close(), chrome.tabs.remove, its window
  // closing, app quit) or the app did (baa_tab_close, baa_profile_close: inside
  // that call). Exactly once per id; `reason` BAA_CLOSED_*. The id is dead after
  // this returns; the tab's NSView stays valid until then.
  void (*tab_closed)(void* ctx, BaaTabId tab, int reason);

  // A site asks for `kinds` (BAA_PERMISSION_* bits; camera + microphone come
  // as one request). It waits until baa_answer(id, ...); `request_closed` with
  // the same id if Chrome dropped the request first (navigation, tab closed).
  // Only sent while the tab is its window's active
  // tab: Chrome holds requests from background tabs until activation.
  void (*permission)(void* ctx, BaaTabId tab, uint64_t id, const char* origin,
                     uint32_t kinds);
  // Chrome withdrew the dialog or permission ask `id` unanswered (navigation,
  // tab closed, page responsive again). Never after an answer, never twice.
  void (*request_closed)(void* ctx, BaaTabId tab, uint64_t id);

  // Right-click in a page. (x, y) in the tab's view, points, top-left origin.
  // Answer with baa_context_menu_done(id, command or -1) exactly once; until
  // then the page counts the menu as open.
  void (*context_menu)(void* ctx, BaaTabId tab, uint64_t id, float x, float y,
                       const BaaMenuItem* items, size_t count);

  // A download started or changed. `tab` is the page it came from, 0 if
  // none (a service worker, a closed tab).
  void (*download)(void* ctx, BaaTabId tab, const BaaDownload* download);

  // An extension popup's content wants this size (points).
  void (*resized)(void* ctx, BaaTabId tab, int width, int height);

  // The DevTools front-end `front` was moved with its own dock menu
  // (BAA_DOCK_*).
  void (*devtools_dock)(void* ctx, BaaTabId front, int side);

  // The page went fullscreen (true; Chrome already counts the window as
  // fullscreen) or left it (false).
  void (*fullscreen)(void* ctx, BaaTabId tab, bool on);

  // baa_tab_find's progress: the active match (1-based, 0 for none) and the
  // number found so far.
  void (*find_result)(void* ctx, BaaTabId tab, int active, int count);

  // Not baa_tab_set_group (an extension) changed the tab's group: `group` is the
  // app's key, or a new key with the top bit set; 0 for none. `title` is the
  // group's title.
  void (*tab_group)(void* ctx, BaaTabId tab, uint64_t group, const char* title);

  // Matches for baa_profile_autocomplete(id), best first, replacing the last
  // list for id; `done` on the last one. Never inside the call.
  void (*autocomplete)(void* ctx, uint64_t id, const BaaMatch* matches,
                       size_t count, bool done);

  // baa_profile_clear_data(id, ...) finished (or failed to start: then at once,
  // on a later turn). Sent exactly once per call, never inside it.
  void (*clear_done)(void* ctx, uint64_t id);

  // baa_profile_load_unpacked(id, ...) finished. `message` is the new
  // extension's id when `ok`, else Chrome's error text. Sent exactly once per
  // call, never inside it.
  void (*load_unpacked_done)(void* ctx, uint64_t id, bool ok,
                             const char* message);
} BaaCallbacks;

// ---- App -------------------------------------------------------------------

// Makes NSApp a BaaApplication (a BrowserCrApplication carrying gpui's
// `platform` ivar), installs the BrowserWindow hook and keeps argv and a copy
// of `callbacks` (ctx is passed back to each). Call it before anything else
// touches NSApp; the first -[NSApp run] then runs ChromeMain, which owns the
// run loop from there. argv must stay valid for the life of the process.
BAA_EXPORT void baa_install_app(int argc,
                                const char** argv,
                                const BaaCallbacks* callbacks,
                                void* ctx);

// ChromeMain's result, once -[NSApp run] has returned.
BAA_EXPORT int baa_exit_code(void);

// True once Chrome has begun quitting (⌘Q, AttemptExit, terminate). tab_closed
// callbacks that arrive after this are the quit closing tabs, not the user.
BAA_EXPORT bool baa_is_quitting(void);

// ---- Profiles --------------------------------------------------------------
// A BaaProfile is a Chrome Profile plus the Browser its tabs live in. Valid
// until baa_profile_close or app termination.

// The profile in directory `dir_name` directly under --user-data-dir
// (baa uses "space-<id>"), created if missing. Loads synchronously.
// The same name returns the same handle. NULL if Chrome refuses.
BAA_EXPORT BaaProfile* baa_profile_open(const char* dir_name);

// A new in-memory (off-the-record) profile with its own storage, nothing
// persisted. Each call is a separate one.
BAA_EXPORT BaaProfile* baa_profile_open_ephemeral(void);

// Closes the profile's tabs at once (no beforeunload; each gets tab_closed with
// BAA_CLOSED_BY_APP, inside this call), drops
// the handle, and destroys an ephemeral profile's storage. A disk profile's
// directory stays; deleting it is the caller's job (at next launch, before
// baa_install_app).
BAA_EXPORT void baa_profile_close(BaaProfile* profile);

// ---- Prefs -----------------------------------------------------------------

// Sets a user pref by its Chrome name. `profile` NULL means Local State
// (browser-wide prefs). True when the pref is registered, the user may change
// it (policy does not force it) and it has this type; otherwise false and
// nothing changes. A dead profile is false too. The pref changes at once;
// nothing is notified back. A string pref that holds a path takes the path.
BAA_EXPORT bool baa_pref_set_bool(BaaProfile* profile, const char* name,
                                  bool value);
BAA_EXPORT bool baa_pref_set_int(BaaProfile* profile, const char* name,
                                 int value);
BAA_EXPORT bool baa_pref_set_string(BaaProfile* profile, const char* name,
                                    const char* value);
// Resets a user pref to its default. False for an unknown pref, one that
// policy forces, or a dead profile. `profile` NULL means Local State.
BAA_EXPORT bool baa_pref_clear(BaaProfile* profile, const char* name);

// Whether this app is the system's default web browser, and asking the system
// to make it so (the system confirms with its own dialog; there is no result).
BAA_EXPORT bool baa_is_default_browser(void);
BAA_EXPORT void baa_set_default_browser(void);

// The default zoom of the profile's pages; `factor` 1.0 is 100%. Pages that
// have their own zoom (baa_tab_set_zoom) keep it. An ephemeral profile uses
// its parent's, so this sets the parent's.
BAA_EXPORT void baa_profile_set_default_zoom(BaaProfile* profile,
                                             double factor);

// Makes the engine the profile's default search. An engine with this URL, or
// any engine of the same site (Chrome's own Google, say), is reused: a
// user-added one gets the new name, keyword and URL, a built-in one keeps its
// own. Otherwise the engine is added. `url` holds %s for the query; `keyword`
// is its short name for the omnibox ("" derives one). Applied once the
// profile's engine list has loaded, maybe after this returns. False if the
// arguments are unusable (no %s, empty name, not http(s)) or, when the list is
// loaded, Chrome refuses it as a default (policy or an extension controls
// search); true means accepted, and then still pending if the list was not
// loaded. An ephemeral profile uses its parent's list.
BAA_EXPORT bool baa_profile_set_search_engine(BaaProfile* profile,
                                              const char* name,
                                              const char* keyword,
                                              const char* url);

// ---- Site settings ---------------------------------------------------------

// What the profile remembers for `kind` (one BAA_PERMISSION_* bit, not OTHER)
// on `url`'s site: BAA_SITE_*.
BAA_EXPORT int baa_profile_site_permission(BaaProfile* profile,
                                           const char* url, uint32_t kind);
BAA_EXPORT void baa_profile_set_site_permission(BaaProfile* profile,
                                                const char* url,
                                                uint32_t kind, int state);

// Visits the rules the user set for `kind` (one BAA_PERMISSION_* bit, not
// OTHER): first the default as pattern "*" (Chrome's own when the user set
// none), then each exception with Chrome's pattern string (e.g.
// "[*.]example.com"), `state` a BAA_SITE_* value. Synchronous; strings are
// borrowed for the visit. Returns the number visited, -1 for a gone profile or
// an unknown kind.
typedef void (*BaaSiteRuleVisitor)(void* ctx, const char* pattern, int state);
BAA_EXPORT int baa_profile_site_rules(BaaProfile* profile, uint32_t kind,
                                      BaaSiteRuleVisitor visit, void* ctx);

// "*" sets the kind's default, anything else an exception for that pattern.
// BAA_SITE_UNSET removes the exception (or restores Chrome's default for "*").
// False for a bad pattern, an unknown kind or a state the kind does not take;
// then nothing changes.
BAA_EXPORT bool baa_profile_set_site_rule(BaaProfile* profile, uint32_t kind,
                                          const char* pattern, int state);

// ---- Browsing data ---------------------------------------------------------

enum {
  BAA_CLEAR_HISTORY = 1 << 0,
  BAA_CLEAR_DOWNLOADS = 1 << 1,
  BAA_CLEAR_SITE_DATA = 1 << 2,  // cookies, storage, service workers
  BAA_CLEAR_CACHE = 1 << 3,
};

// Deletes the `types` (BAA_CLEAR_* bits) the profile wrote since `since_ms`
// (Unix ms; 0 = all time). Then `clear_done(id)`, never inside this call. `id`
// is the caller's, unique among its pending calls. Chrome's own stores only:
// the caller keeps any records of its own.
BAA_EXPORT void baa_profile_clear_data(BaaProfile* profile, uint64_t id,
                                       int64_t since_ms, uint32_t types);

// Starts Chrome's omnibox for `text` (history, search suggestions,
// calculator); results by autocomplete(id, ...). A later call stops earlier
// ones (they send nothing more); "" only stops.
BAA_EXPORT void baa_profile_autocomplete(BaaProfile* profile, uint64_t id,
                                         const char* text);

// ---- Extensions ------------------------------------------------------------

enum {
  BAA_SITE_ACCESS_NONE = 0,  // not the user's choice (no host permissions)
  BAA_SITE_ACCESS_ON_CLICK = 1,
  BAA_SITE_ACCESS_ON_SPECIFIC_SITES = 2,
  BAA_SITE_ACCESS_ON_ALL_SITES = 3,
};

typedef struct {
  const char* id;
  const char* name;
  const char* description;
  const char* version;
  const char* icon_path;    // absolute; action's default icon, else manifest
                            // icon, nearest 32 px; "" if none
  const char* popup_url;    // the action's default popup; "" if none
  const char* options_url;  // "" if none
  const char* permissions;  // Chrome's permission messages, '\n'-separated
  int site_access;          // BAA_SITE_ACCESS_*
  bool enabled;
  bool unpacked;            // loaded from a folder (developer mode): reloadable
} BaaExtension;

typedef void (*BaaExtensionVisitor)(void* ctx, const BaaExtension* extension);

// Every user-visible extension of the profile (no component extensions, no
// themes), synchronously. Returns the count, -1 for a gone profile.
BAA_EXPORT int baa_profile_extensions(BaaProfile* profile,
                                      BaaExtensionVisitor visit, void* ctx);

enum {
  BAA_EXTENSION_ENABLE = 0,       // value: 1 enable, 0 disable
  BAA_EXTENSION_REMOVE = 1,       // value ignored
  BAA_EXTENSION_SITE_ACCESS = 2,  // value: BAA_SITE_ACCESS_ON_*
  BAA_EXTENSION_RELOAD = 3,       // unpacked extensions only; value ignored
};

// Carries out the change at once (RELOAD starts the reload); false if it was
// refused or the id is unknown.
BAA_EXPORT bool baa_profile_extension_change(BaaProfile* profile,
                                             const char* extension_id,
                                             int change, int value);

// Loads the unpacked extension in folder `path` (the developer mode "Load
// unpacked"; `extensions.ui.developer_mode` is a pref the caller sets with
// baa_pref_set_bool). Result by `load_unpacked_done(id, ...)`, never inside this
// call. The extension then shows in baa_profile_extensions.
BAA_EXPORT void baa_profile_load_unpacked(BaaProfile* profile, uint64_t id,
                                          const char* path);

// Clicks the extension's toolbar action on the profile's active tab (grants
// activeTab, fires action.onClicked). Returns the popup page when the action
// has one (it reports tab_state/resized and tab_closed when it closes
// itself), else 0.
BAA_EXPORT BaaTabId baa_extension_popup(BaaProfile* profile,
                                       const char* extension_id);

// ---- Tabs ------------------------------------------------------------------

// A background tab in `profile`'s browser, loading `url`. 0 when the
// profile is gone or the app is terminating. No tab_created for it.
BAA_EXPORT BaaTabId baa_tab_create(BaaProfile* profile, const char* url);

// Closes the tab now, skipping beforeunload. tab_closed(BAA_CLOSED_BY_APP)
// comes inside this call.
BAA_EXPORT void baa_tab_close(BaaTabId tab);

// The tab's current state, as in the tab_state callback (an adopted tab's
// first snapshot). `state`'s strings are valid until the next baa_* call.
BAA_EXPORT bool baa_tab_get_state(BaaTabId tab, BaaTabState* state);

BAA_EXPORT void baa_tab_load_url(BaaTabId tab, const char* url);
BAA_EXPORT void baa_tab_reload(BaaTabId tab);
BAA_EXPORT void baa_tab_stop(BaaTabId tab);
// Moves through history: -1 is back, 1 forward. Out of range does nothing.
BAA_EXPORT void baa_tab_go_to_offset(BaaTabId tab, int offset);

// Calls `visit` once per back/forward entry, oldest first, synchronously;
// returns the current entry's index (-1 for a gone tab).
typedef void (*BaaHistoryVisitor)(void* ctx, const char* title, const char* url);
BAA_EXPORT int baa_tab_history(BaaTabId tab, BaaHistoryVisitor visit, void* ctx);

// Runs `script` in the main frame's page world; the result is dropped.
// `user_gesture`: as if the user had just clicked (Picture in Picture,
// fullscreen).
BAA_EXPORT void baa_tab_eval(BaaTabId tab, const char* script, bool user_gesture);

// `factor` 1.0 is 100%.
BAA_EXPORT void baa_tab_set_zoom(BaaTabId tab, double factor);

// Highlights `text`; repeating the same text moves to the next (or previous)
// match; counts come by find_result. Empty `text` ends the search and clears
// the highlight.
BAA_EXPORT void baa_tab_find(BaaTabId tab, const char* text, bool forward);

// The system print dialog for the page.
BAA_EXPORT void baa_tab_print(BaaTabId tab);
BAA_EXPORT void baa_tab_set_muted(BaaTabId tab, bool muted);

// Makes the tab its window's active tab and that window Chrome's last active
// one, so extensions' "active tab in the current window" is this tab. Call
// when the tab is shown. Visibility itself follows the NSView (hidden or out
// of a window = hidden page); there is no call for it.
BAA_EXPORT void baa_tab_activate(BaaTabId tab);

// Keyboard focus to the page (its view becomes first responder).
BAA_EXPORT void baa_tab_focus(BaaTabId tab);

// Runs `script` in the main world of every new main-frame document of the
// tab, as soon as its root element exists. NULL or "" stops it.
BAA_EXPORT void baa_tab_set_init_script(BaaTabId tab, const char* script);

// prefers-color-scheme of the profile's pages: 0 follows the system, 1 light,
// 2 dark. `webui` applies to Chrome's own pages (chrome: and baa: URLs),
// `website` to everything else; each document gets the one for its committed
// URL, and live tabs follow a change at once.
BAA_EXPORT void baa_profile_set_color_schemes(BaaProfile* profile, int website,
                                              int webui);

// DevTools for `tab`, to be docked by the caller: returns the front-end page
// (place its state.view; baa_tab_close closes DevTools). With `inspect`,
// selects the element at (x, y) in the page's view. `side` (BAA_DOCK_RIGHT/
// BOTTOM/LEFT) preselects the front-end's dock menu; a change there comes back
// as devtools_dock. Closes DevTools already open for the tab first. 0 if
// Chrome refused.
BAA_EXPORT BaaTabId baa_tab_open_devtools(BaaTabId tab, bool inspect, int x,
                                          int y, int side);

// DevTools for `tab` in Chrome's own window.
BAA_EXPORT void baa_tab_show_devtools_window(BaaTabId tab, bool inspect, int x,
                                             int y);

// The user left page fullscreen through the app (the window left macOS
// fullscreen, a Space switch). fullscreen(tab, false) follows.
BAA_EXPORT void baa_tab_exit_fullscreen(BaaTabId tab);

// Puts the tab in the tab group `group` (the app's key, top bit clear; or a
// key from tab_group) titled `title`, making the group if the key has none.
// 0 takes it out of its group. No tab_group callback for this.
BAA_EXPORT void baa_tab_set_group(BaaTabId tab, uint64_t group,
                                  const char* title);

// ---- Dialogs ---------------------------------------------------------------

enum {
  BAA_DIALOG_ALERT = 0,
  BAA_DIALOG_CONFIRM = 1,
  BAA_DIALOG_PROMPT = 2,
  BAA_DIALOG_BEFOREUNLOAD = 3,         // OK leaves the page
  BAA_DIALOG_BEFOREUNLOAD_RELOAD = 4,  // OK reloads it
  BAA_DIALOG_AUTH = 5,                 // a username and a password field
  BAA_DIALOG_EXTERNAL_PROTOCOL = 6,    // a link wants an app
  BAA_DIALOG_UNRESPONSIVE = 7,         // OK ends the page, Cancel waits
  BAA_DIALOG_EXTENSION_INSTALL = 8,    // also re-enable and permissions.request
};

// Answers a `dialog` or `permission` ask (for a permission, `ok` allows).
// Ids are never reused; unknown, answered or withdrawn ids do nothing.
BAA_EXPORT void baa_answer(uint64_t id, const BaaAnswer* answer);

// ---- Menus, downloads -------------------------------------------------------

// Closes the menu of a `context_menu` callback; `command` >= 0 runs that
// item first. Unknown ids do nothing.
BAA_EXPORT void baa_context_menu_done(uint64_t id, int command);

// Stops a running download; finished ones are left alone.
BAA_EXPORT void baa_download_cancel(uint64_t id);

#ifdef __cplusplus
}
#endif

#endif  // BAA_BAA_H_

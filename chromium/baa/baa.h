// C ABI between BaaCore.framework (//baa) and the Rust shell (crates/baa-web,
// module `core_host`). Spec: PLAN-chromium-phase3.md.
//
// Rules for both sides:
// - Threading: every function and every callback runs on the main thread
//   (Chromium's UI thread). Nothing here is thread-safe.
// - Handles (BaaProfile*, BaaTab*) are opaque and owned by BaaCore. A call on
//   a handle that is gone (closed, or the app is terminating) does nothing and
//   returns NULL/0/false, but only until its address is reused: a new tab or
//   profile may get the address of a closed one. Never use a handle after
//   tab_closed, or after your own baa_tab_close / baa_profile_close; drop it
//   there.
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
#define BAA_ABI_VERSION 5
BAA_EXPORT int baa_abi_version(void);

typedef struct BaaProfile BaaProfile;
// Any page baa shows: a tab in a Browser, an extension popup, or a DevTools
// front-end. Calls that only make sense for a tab (baa_tab_activate) do
// nothing on the others.
typedef struct BaaTab BaaTab;

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
  uint32_t id;           // unique per run; baa_download_cancel takes it
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

typedef struct {
  // A ⌘/⌃ or Escape key-down in a page. `after_page` is false before the page
  // sees it and true when the page did not consume it. Return true to take
  // it. Synchronous: the event is decided by the return value.
  bool (*key)(void* ctx, void* ns_event, bool after_page);

  // Chrome made a tab baa did not ask for: window.open, target=_blank,
  // ⌘/middle-click, chrome.tabs.create, chrome.windows.create, an options
  // page. `opener` is the tab it came from, NULL if none (an extension, or a
  // tab baa does not know). `profile` is the BaaProfile it lives in, NULL if
  // it is a profile baa never opened (e.g. Chrome's primary incognito).
  // `background`: it did not become its window's active tab.
  // The tab is live and loading; baa adopts it or closes it.
  void (*tab_created)(void* ctx,
                      BaaTab* tab,
                      BaaTab* opener,
                      BaaProfile* profile,
                      const char* url,
                      bool background,
                      bool user_gesture);

  // Any field of the tab's state may have changed.
  void (*tab_state)(void* ctx, BaaTab* tab, const BaaTabState* state);

  // The favicon at `icon_url` as PNG (2x when the site has it). `png` is
  // borrowed for the call.
  void (*tab_favicon)(void* ctx,
                      BaaTab* tab,
                      const char* icon_url,
                      const uint8_t* png,
                      size_t png_len);

  // A page's alert/confirm/prompt (kind BAA_DIALOG_*). The page is paused
  // until baa_dialog_answer(id, ...); `dialog_closed` with the same id if the
  // page went away first (then answering does nothing). beforeunload comes too
  // (kinds BAA_DIALOG_BEFOREUNLOAD*).
  void (*dialog)(void* ctx,
                 BaaTab* tab,
                 uint64_t id,
                 int kind,
                 const char* message,
                 const char* default_text);
  void (*dialog_closed)(void* ctx, BaaTab* tab, uint64_t id);

  // Chrome closed the tab (window.close(), chrome.tabs.remove, its window
  // closing, app quit). Not sent for baa_tab_close. The handle is dead after
  // this returns; the tab's NSView stays valid until then.
  void (*tab_closed)(void* ctx, BaaTab* tab);

  // A site asks for `kinds` (BAA_PERMISSION_* bits; camera + microphone come
  // as one request). It waits until baa_permission_answer(id, ...);
  // `permission_closed` with the same id if Chrome dropped the request first
  // (navigation, tab closed). Only sent while the tab is its window's active
  // tab: Chrome holds requests from background tabs until activation.
  void (*permission)(void* ctx, BaaTab* tab, uint64_t id, const char* origin,
                     uint32_t kinds);
  void (*permission_closed)(void* ctx, BaaTab* tab, uint64_t id);

  // Right-click in a page. (x, y) in the tab's view, points, top-left origin.
  // Answer with baa_context_menu_done(id, command or -1) exactly once; until
  // then the page counts the menu as open.
  void (*context_menu)(void* ctx, BaaTab* tab, uint64_t id, float x, float y,
                       const BaaMenuItem* items, size_t count);

  // A download started or changed. `tab` is the page it came from, NULL if
  // none (a service worker, a closed tab).
  void (*download)(void* ctx, BaaTab* tab, const BaaDownload* download);

  // An extension popup's content wants this size (points).
  void (*resized)(void* ctx, BaaTab* tab, int width, int height);

  // A console message from the page that starts with "baa:", without the
  // prefix (DevTools' dock picker sends "devtools-dock:<side>").
  void (*page_message)(void* ctx, BaaTab* tab, const char* message);

  // baa_profile_fetch finished. `body` is NULL when !ok.
  void (*fetch_done)(void* ctx, uint64_t id, bool ok, const char* body,
                     size_t len);

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

// Closes the profile's tabs at once (no beforeunload, no tab_closed), drops
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

// GET `url` through the profile's network stack without cookies;
// fetch_done(id, ...) later, never inside this call.
BAA_EXPORT void baa_profile_fetch(BaaProfile* profile, const char* url,
                                  uint64_t id);

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
// itself), else NULL.
BAA_EXPORT BaaTab* baa_extension_popup(BaaProfile* profile,
                                       const char* extension_id);

// ---- Tabs ------------------------------------------------------------------

// A background tab in `profile`'s browser, loading `url`. NULL when the
// profile is gone or the app is terminating. No tab_created for it.
BAA_EXPORT BaaTab* baa_tab_create(BaaProfile* profile, const char* url);

// Closes the tab now, skipping beforeunload. No tab_closed for it.
BAA_EXPORT void baa_tab_close(BaaTab* tab);

// The tab's current state, as in the tab_state callback (an adopted tab's
// first snapshot). `state`'s strings are valid until the next baa_* call.
BAA_EXPORT bool baa_tab_get_state(BaaTab* tab, BaaTabState* state);

BAA_EXPORT void baa_tab_load_url(BaaTab* tab, const char* url);
BAA_EXPORT void baa_tab_reload(BaaTab* tab);
BAA_EXPORT void baa_tab_stop(BaaTab* tab);
// Moves through history: -1 is back, 1 forward. Out of range does nothing.
BAA_EXPORT void baa_tab_go_to_offset(BaaTab* tab, int offset);

// Calls `visit` once per back/forward entry, oldest first, synchronously;
// returns the current entry's index (-1 for a gone tab).
typedef void (*BaaHistoryVisitor)(void* ctx, const char* title, const char* url);
BAA_EXPORT int baa_tab_history(BaaTab* tab, BaaHistoryVisitor visit, void* ctx);

// Runs `script` in the main frame's page world; the result is dropped.
// `user_gesture`: as if the user had just clicked (Picture in Picture,
// fullscreen).
BAA_EXPORT void baa_tab_eval(BaaTab* tab, const char* script, bool user_gesture);

// `factor` 1.0 is 100%.
BAA_EXPORT void baa_tab_set_zoom(BaaTab* tab, double factor);

// Highlights `text`; repeating the same text moves to the next (or previous)
// match. Empty `text` ends the search and clears the highlight.
BAA_EXPORT void baa_tab_find(BaaTab* tab, const char* text, bool forward);

// The system print dialog for the page.
BAA_EXPORT void baa_tab_print(BaaTab* tab);
BAA_EXPORT void baa_tab_set_muted(BaaTab* tab, bool muted);

// Makes the tab its window's active tab and that window Chrome's last active
// one, so extensions' "active tab in the current window" is this tab. Call
// when the tab is shown. Visibility itself follows the NSView (hidden or out
// of a window = hidden page); there is no call for it.
BAA_EXPORT void baa_tab_activate(BaaTab* tab);

// Keyboard focus to the page (its view becomes first responder).
BAA_EXPORT void baa_tab_focus(BaaTab* tab);

// Runs `script` in the main world of every new main-frame document of the
// tab, as soon as its root element exists. NULL or "" stops it.
BAA_EXPORT void baa_tab_set_init_script(BaaTab* tab, const char* script);

// prefers-color-scheme of the profile's pages: 0 follows the system, 1 light,
// 2 dark. `webui` applies to Chrome's own pages (chrome: and baa: URLs),
// `website` to everything else; each document gets the one for its committed
// URL, and live tabs follow a change at once.
BAA_EXPORT void baa_profile_set_color_schemes(BaaProfile* profile, int website,
                                              int webui);

// DevTools for `tab`, to be docked by the caller: returns the front-end page
// (place its state.view; baa_tab_close closes DevTools). With `inspect`,
// selects the element at (x, y) in the page's view. Closes DevTools already
// open for the tab first. NULL if Chrome refused.
BAA_EXPORT BaaTab* baa_tab_open_devtools(BaaTab* tab, bool inspect, int x,
                                         int y);

// DevTools for `tab` in Chrome's own window.
BAA_EXPORT void baa_tab_show_devtools_window(BaaTab* tab, bool inspect, int x,
                                             int y);

// ---- Dialogs ---------------------------------------------------------------

enum {
  BAA_DIALOG_ALERT = 0,
  BAA_DIALOG_CONFIRM = 1,
  BAA_DIALOG_PROMPT = 2,
  BAA_DIALOG_BEFOREUNLOAD = 3,         // OK leaves the page
  BAA_DIALOG_BEFOREUNLOAD_RELOAD = 4,  // OK reloads it
};

// Answers a `dialog` callback: OK (with the prompt's text) or Cancel. Ids are
// never reused; an unknown or already answered id does nothing.
BAA_EXPORT void baa_dialog_answer(uint64_t id, bool ok, const char* text);

// ---- Permissions, menus, downloads -----------------------------------------

// Answers `permission`. Unknown or answered ids do nothing.
BAA_EXPORT void baa_permission_answer(uint64_t id, bool allow);

// Closes the menu of a `context_menu` callback; `command` >= 0 runs that
// item first. Unknown ids do nothing.
BAA_EXPORT void baa_context_menu_done(uint64_t id, int command);

// Stops a running download; finished ones are left alone.
BAA_EXPORT void baa_download_cancel(uint32_t id);

#ifdef __cplusplus
}
#endif

#endif  // BAA_BAA_H_

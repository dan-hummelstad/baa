// C ABI between BaaCore.framework (//baa) and the Rust shell (crates/baa-web,
// module `core_host`). Spec: PLAN-chromium-phase3.md.
//
// Rules for both sides:
// - Threading: every function and every callback runs on the main thread
//   (Chromium's UI thread). Nothing here is thread-safe.
// - Handles (BaaProfile*, BaaTab*) are opaque and owned by BaaCore. A call on
//   a handle that is gone (closed, or the app is terminating) does nothing and
//   returns NULL/0/false. Rust may keep a dead handle; it must not dereference it.
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
#define BAA_ABI_VERSION 3
BAA_EXPORT int baa_abi_version(void);

typedef struct BaaProfile BaaProfile;
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
  // page went away first (then answering does nothing). beforeunload is not
  // sent here: Chrome shows its own sheet for it.
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

// ---- Dialogs ---------------------------------------------------------------

enum {
  BAA_DIALOG_ALERT = 0,
  BAA_DIALOG_CONFIRM = 1,
  BAA_DIALOG_PROMPT = 2,
};

// Answers a `dialog` callback: OK (with the prompt's text) or Cancel. Ids are
// never reused; an unknown or already answered id does nothing.
BAA_EXPORT void baa_dialog_answer(uint64_t id, bool ok, const char* text);

// ---- M2 --------------------------------------------------------------------
// Not here on purpose: permissions, context menu, downloads, extensions,
// fetch, site permissions and DevTools come with BAA_ABI_VERSION 4, shaped
// like `dialog`/baa_dialog_answer (a uint64_t id answered later). Outline:
// PLAN-chromium-phase3.md "M2 outline".

#ifdef __cplusplus
}
#endif

#endif  // BAA_BAA_H_

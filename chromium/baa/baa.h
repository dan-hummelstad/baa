// C ABI between BaaCore and the Rust shell. Every call runs on the main thread.
#ifndef BAA_BAA_H_
#define BAA_BAA_H_

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

#define BAA_EXPORT __attribute__((visibility("default")))

// Bumped when the ABI changes, so the Rust side can refuse a mismatched framework.
BAA_EXPORT int baa_abi_version(void);

typedef struct BaaWebContents BaaWebContents;

// A ⌘/⌃ or Escape key-down in a page. `after_page` is false before the page
// sees it and true when the page did not consume it. Return true to take it.
typedef bool (*BaaKeyFn)(void* ctx,
                         BaaWebContents* web_contents,
                         void* ns_event,
                         bool after_page);

// Makes NSApp a BaaApplication (a BrowserCrApplication) and keeps argv for
// ChromeMain. Call before anything else touches NSApp; the first -[NSApp run]
// then starts Chromium, which owns the run loop from there.
BAA_EXPORT void baa_install_app(int argc,
                                const char** argv,
                                BaaKeyFn key,
                                void* key_ctx);
// ChromeMain's result, once -[NSApp run] has returned.
BAA_EXPORT int baa_exit_code(void);

// A page in the last-used profile, loading `url`.
BAA_EXPORT BaaWebContents* baa_web_contents_create(const char* url);
// The page's NSView; owned by the page.
BAA_EXPORT void* baa_web_contents_view(BaaWebContents* web_contents);
BAA_EXPORT void baa_web_contents_focus(BaaWebContents* web_contents);

enum {
  BAA_EDIT_COPY = 0,
  BAA_EDIT_CUT = 1,
  BAA_EDIT_PASTE = 2,
  BAA_EDIT_SELECT_ALL = 3,
  BAA_EDIT_UNDO = 4,
  BAA_EDIT_REDO = 5,
};
BAA_EXPORT void baa_web_contents_edit(BaaWebContents* web_contents,
                                      int command);
BAA_EXPORT void baa_web_contents_destroy(BaaWebContents* web_contents);

#ifdef __cplusplus
}
#endif

#endif  // BAA_BAA_H_

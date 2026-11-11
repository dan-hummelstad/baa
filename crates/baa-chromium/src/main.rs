//! Phase 2 spike: a gpui window with a bar and one Chromium page from
//! BaaCore.framework. Run it with `crates/baa-chromium/ci.sh`.
//! Start-up order and why: PLAN-chromium-phase2.md.

#[cfg(target_os = "macos")]
#[path = "../../baa-web/src/core_host/ffi.rs"]
#[allow(dead_code)] // the harness drives a subset of the ABI
mod ffi;
#[cfg(target_os = "macos")]
mod harness;
#[cfg(target_os = "macos")]
#[path = "../../baa-web/src/core_host/native.rs"]
#[allow(dead_code)] // the harness uses part of what the app needs
mod native;

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    app::main()
}

#[cfg(target_os = "macos")]
mod app {
    use std::{
        cell::{Cell, RefCell},
        ffi::{c_char, c_int, c_void, CStr, CString},
        rc::Rc,
    };

    use gpui::{
        actions, div, prelude::*, px, relative, rgb, size, App, Bounds, DispatchPhase, Element, ElementId,
        FocusHandle, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, KeyBinding, KeyDownEvent,
        LayoutId, Menu, MenuItem, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollWheelEvent,
        SharedString, Style, Window, WindowBackgroundAppearance, WindowBounds, WindowOptions,
    };
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;

    use crate::{ffi, harness, native};

    actions!(spike, [Quit, FocusBar, NewTab]);

    thread_local! {
        static GPUI_VIEW: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) };    }

    static CORE: std::sync::OnceLock<&'static ffi::Core> = std::sync::OnceLock::new();

    fn core() -> &'static ffi::Core {
        CORE.get().expect("BaaCore not loaded")
    }

    fn empty_state() -> ffi::TabState {
        // SAFETY: raw pointers and bools; all-zero is null and false.
        unsafe { std::mem::zeroed() }
    }

    pub fn main() {
        // Before anything can start a thread (see ffi::load).
        let core = *CORE.get_or_init(ffi::load);

        let mut args: Vec<String> = std::env::args().collect();
        for flag in ["--no-first-run", "--no-startup-window", "--enable-logging=stderr"] {
            args.push(flag.into());
        }
        if !args.iter().any(|a| a.starts_with("--user-data-dir")) {
            let dir = std::env::temp_dir().join("baa-spike");
            args.push(format!("--user-data-dir={}", dir.display()));
        }
        // ChromeMain reads argv for the life of the process.
        let argv: &'static [*const c_char] = Vec::leak(
            args.into_iter().map(|a| CString::new(a).unwrap().into_raw() as *const c_char).collect(),
        );
        (core.install_app)(argv.len() as _, argv.as_ptr(), &CALLBACKS, std::ptr::null_mut());
        assert!(native::nsapp_has_gpui_ivar(), "NSApp lacks gpui's `platform` ivar");

        // gpui's [NSApp run] becomes ChromeMain; this closure runs from
        // didFinishLaunching once Chromium's browser process is up.
        gpui_platform::application().run(|cx: &mut App| {
            eprintln!("spike: gpui finished launching");
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-l", FocusBar, None),
                KeyBinding::new("cmd-t", NewTab, None),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.set_menus([Menu {
                name: "baa".into(),
                items: vec![MenuItem::action("Quit baa", Quit)],
                disabled: false,
            }]);
            let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                // An opaque Metal layer draws the punched hole black.
                window_background: WindowBackgroundAppearance::Transparent,
                ..Default::default()
            };
            let spike = cx.open_window(options, |window, cx| cx.new(|cx| Spike::new(window, cx))).unwrap();
            cx.activate(true);
            if std::env::var_os("BAA_SPIKE_SELFTEST").is_some() {
                self_test(spike, cx);
            }
            start_poller(cx);
            if let Ok(path) = std::env::var("BAA_SPIKE_SCRIPT") {
                run_script(path, cx);
            }
            // BAA_SPIKE_QUIT_AFTER=<s>: quit the way ⌘Q does.
            if let Some(secs) = std::env::var("BAA_SPIKE_QUIT_AFTER").ok().and_then(|s| s.parse().ok()) {
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(std::time::Duration::from_secs(secs)).await;
                    eprintln!("spike: quitting");
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
        });
        eprintln!("spike: run loop returned, exit code {}", (core.exit_code)());
        std::process::exit((core.exit_code)());
    }

    /// BAA_SPIKE_SELFTEST: real NSEvents through the window, page focused,
    /// then the bar. The checks: stderr (`spike: gpui ...`) for gpui's side,
    /// the page's `keys` log and textarea over DevTools for the page's side.
    fn self_test(spike: gpui::WindowHandle<Spike>, cx: &mut App) {
        // (key code, characters, ⌘); "focus bar" steps are gpui-side.
        const PAGE: &[(u16, &str, bool)] = &[
            (4, "h", false),
            (14, "e", false),
            (37, "l", false),
            (37, "l", false),
            (31, "o", false),
            (0, "a", true),  // ⌘A: select all (edit fallback)
            (7, "x", false), // replaces the selection
            (6, "z", true),  // ⌘Z: undo, back to "hello"
            (53, "\u{1b}", false),
            (17, "t", true), // ⌘T: gpui
            (37, "l", true), // ⌘L: gpui focuses the bar
            (0, "a", false), // into the bar, not the page
            (11, "b", false),
        ];
        cx.spawn(async move |cx| {
            let executor = cx.background_executor().clone();
            let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
            wait(3000).await;
            let window_number = spike
                .update(cx, |spike, window, cx| {
                    window.activate_window();
                    window.focus(&spike.page_focus, cx);
                    (core().tab_focus)(spike.page.tab);
                    harness::window_number(spike.page.view)
                })
                .unwrap();
            wait(500).await;
            // Into the textarea the way a user gets there: a click.
            let view = spike.update(cx, |spike, _, _| spike.page.view).unwrap();
            harness::post_click(view, 200., 140., false, false);
            wait(500).await;
            for &(code, chars, command) in PAGE {
                harness::post_key(window_number, code, chars, command);
                wait(150).await;
            }
            wait(500).await;
            spike.update(cx, |spike, _, _| eprintln!("spike: gpui bar text {:?}", spike.typed)).ok();
            eprintln!("spike: self-test done");
        })
        .detach();
    }

    /// ⌘/⌃/Escape from the page: gpui first; editing shortcuts the page left
    /// alone go back to the page as edit actions down the responder chain
    /// (nothing else would run them: there is no Chrome window or menu behind
    /// the page).
    extern "C" fn on_key(_: *mut c_void, event: *mut c_void, after_page: bool) -> bool {
        let event = event.cast::<AnyObject>();
        if !after_page {
            return native::offer_key_equivalent(event);
        }
        match harness::edit_command(event) {
            Some(command) => {
                harness::send_edit_action(command);
                true
            }
            None => false,
        }
    }

    static CALLBACKS: ffi::Callbacks = ffi::Callbacks {
        key: on_key,
        tab_created: on_tab_created,
        tab_state: on_tab_state,
        tab_favicon: on_tab_favicon,
        dialog: on_dialog,
        tab_closed: on_tab_closed,
        permission: on_permission,
        request_closed: on_request_closed,
        context_menu: on_context_menu,
        download: on_download,
        resized: on_resized,
        devtools_dock: on_devtools_dock,
        fullscreen: on_fullscreen,
        exclusive_access: on_exclusive_access,
        find_result: on_find_result,
        tab_group: on_tab_group,
        autocomplete: on_autocomplete,
        clear_done: on_clear_done,
        load_unpacked_done: on_load_unpacked_done,
        cdp: on_cdp,
        media: on_media,
        focus_requested: on_focus_requested,
        swipe: on_swipe,
    };

    thread_local! {
        /// Answers wait for the next turn, as the app's do: Chrome has not
        /// finished with a prompt or menu inside the callback that announced it.
        static LATER: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
    }

    fn later(f: impl FnOnce() + 'static) {
        LATER.with(|l| l.borrow_mut().push(Box::new(f)));
    }

    fn start_poller(cx: &mut App) {
        cx.spawn(async move |cx| loop {
            cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
            let jobs: Vec<_> = LATER.with(|l| l.borrow_mut().drain(..).collect());
            for job in jobs {
                job();
            }
        })
        .detach();
    }

    fn text(p: *const c_char) -> String {
        if p.is_null() {
            return "<null>".into();
        }
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }

    thread_local! {
        /// The page's tab is gone; its view must not be placed again.
        static PAGE_CLOSED: Cell<bool> = const { Cell::new(false) };
        /// Tabs the script can address: 0 is the page in the hole, then
        /// script-made and Chrome-made tabs in arrival order.
        static TABS: RefCell<Vec<ffi::TabId>> = const { RefCell::new(Vec::new()) };
        /// space-1, space-2, ephemeral.
        static PROFILES: RefCell<[*mut ffi::Profile; 3]> = const { RefCell::new([std::ptr::null_mut(); 3]) };
        /// The last `dialog` id: ids come from the shared counter, so scripts say `answer last`.
        static LAST_DIALOG: Cell<u64> = const { Cell::new(0) };
    }

    fn tab_index(tab: ffi::TabId) -> String {
        TABS.with(|t| t.borrow().iter().position(|x| *x == tab)).map_or("?".into(), |i| i.to_string())
    }

    extern "C" fn on_tab_created(
        _: *mut c_void,
        tab: ffi::TabId,
        opener: ffi::TabId,
        profile: *mut ffi::Profile,
        url: *const c_char,
        background: bool,
        gesture: bool,
    ) {
        let index = TABS.with(|t| {
            let mut t = t.borrow_mut();
            t.push(tab);
            t.len() - 1
        });
        let which = PROFILES.with(|p| p.borrow().iter().position(|x| *x == profile));
        eprintln!(
            "baa: tab_created tab={index} opener={} profile={} url={} background={background} gesture={gesture}",
            if opener == 0 { "NULL".into() } else { tab_index(opener) },
            if profile.is_null() { "NULL".into() } else { format!("{which:?}") },
            text(url),
        );
    }

    extern "C" fn on_tab_state(_: *mut c_void, tab: ffi::TabId, s: *const ffi::TabState) {
        let s = unsafe { &*s };
        eprintln!(
            "baa: tab_state tab={} url={} title={:?} icon={} loading={} back={} forward={} audible={} secure={} muted={}",
            tab_index(tab),
            text(s.url),
            text(s.title),
            text(s.favicon_url),
            s.loading,
            s.can_go_back,
            s.can_go_forward,
            s.audible,
            s.secure,
            s.muted
        );
        if s.crashed {
            eprintln!("baa: tab_crashed tab={}", tab_index(tab));
        }
    }

    extern "C" fn on_tab_favicon(_: *mut c_void, tab: ffi::TabId, url: *const c_char, png: *const u8, len: usize) {
        let head = unsafe { std::slice::from_raw_parts(png, len.min(8)) };
        eprintln!("baa: tab_favicon tab={} url={} bytes={len} magic={head:02x?}", tab_index(tab), text(url));
    }

    extern "C" fn on_dialog(_: *mut c_void, tab: ffi::TabId, id: u64, d: *const ffi::Dialog) {
        let d = unsafe { &*d };
        LAST_DIALOG.with(|l| l.set(id));
        eprintln!(
            "baa: dialog tab={} id={id} kind={} message={:?} default={:?} title={:?} ok={:?} cancel={:?} check={:?}",
            tab_index(tab),
            d.kind,
            text(d.message),
            text(d.default_text),
            text(d.title),
            text(d.ok_label),
            text(d.cancel_label),
            text(d.check_label)
        );
        // BAA_HARNESS_DIALOG=ok|cancel answers at once (default: only logs).
        if let Ok(answer) = std::env::var("BAA_HARNESS_DIALOG") {
            later(move || {
                core().answer_ok(id, answer == "ok");
                eprintln!("baa: answer id={id} {answer}");
            });
        }
    }

    extern "C" fn on_tab_closed(_: *mut c_void, tab: ffi::TabId, reason: c_int) {
        eprintln!("baa: tab_closed tab={} reason={reason}", tab_index(tab));
        if tab_index(tab) == "0" {
            PAGE_CLOSED.with(|c| c.set(true));
        }
    }

    /// BAA_HARNESS_PERMISSION=allow|deny answers permission asks (default: only logs).
    extern "C" fn on_permission(_: *mut c_void, tab: ffi::TabId, id: u64, origin: *const c_char, kinds: u32) {
        eprintln!("baa: permission tab={} id={id} origin={} kinds={kinds:#x}", tab_index(tab), text(origin));
        if let Ok(answer) = std::env::var("BAA_HARNESS_PERMISSION") {
            later(move || {
                core().answer_ok(id, answer == "allow");
                eprintln!("baa: answer id={id} {answer}");
            });
        }
    }

    extern "C" fn on_request_closed(_: *mut c_void, tab: ffi::TabId, id: u64) {
        eprintln!("baa: request_closed tab={} id={id}", tab_index(tab));
    }

    /// BAA_HARNESS_MENU=<label>|none picks a row by label (or closes with nothing).
    extern "C" fn on_context_menu(_: *mut c_void, tab: ffi::TabId, id: u64, x: f32, y: f32, items: *const ffi::MenuItem, count: usize) {
        let rows = unsafe { std::slice::from_raw_parts(items, count) };
        let inspect = rows.iter().filter(|r| r.kind == 3).count();
        eprintln!("baa: context_menu tab={} id={id} at={x},{y} rows={count} inspect={inspect}", tab_index(tab));
        for r in rows {
            eprintln!(
                "baa:   menu {}{} cmd={} kind={} enabled={} checked={}",
                " ".repeat(r.depth as usize * 2),
                text(r.label),
                r.command,
                r.kind,
                r.enabled,
                r.checked
            );
        }
        let want = std::env::var("BAA_HARNESS_MENU").unwrap_or_default();
        let command = rows.iter().find(|r| r.kind != 1 && text(r.label) == want).map_or(-1, |r| r.command);
        if !want.is_empty() {
            later(move || {
                eprintln!("baa: context_menu_done id={id} command={command}");
                (core().context_menu_done)(id, command);
            });
        }
    }

    extern "C" fn on_download(_: *mut c_void, tab: ffi::TabId, d: *const ffi::Download) {
        let d = unsafe { &*d };
        eprintln!(
            "baa: download tab={} id={} url={} path={} received={} total={} state={}",
            if tab == 0 { "NULL".into() } else { tab_index(tab) },
            d.id,
            text(d.url),
            text(d.path),
            d.received_bytes,
            d.total_bytes,
            d.state
        );
        if let Some(cancel_at) = std::env::var("BAA_HARNESS_DOWNLOAD_CANCEL").ok().and_then(|s| s.parse::<i64>().ok()) {
            if d.state == 0 && d.received_bytes >= cancel_at {
                let id = d.id;
                later(move || {
                    eprintln!("baa: download_cancel id={id}");
                    (core().download_cancel)(id);
                });
            }
        }
    }

    extern "C" fn on_resized(_: *mut c_void, tab: ffi::TabId, w: c_int, h: c_int) {
        eprintln!("baa: resized tab={} {w}x{h}", tab_index(tab));
    }

    extern "C" fn on_devtools_dock(_: *mut c_void, front: ffi::TabId, side: c_int) {
        eprintln!("baa: devtools_dock tab={} side={side}", tab_index(front));
    }

    extern "C" fn on_fullscreen(_: *mut c_void, tab: ffi::TabId, on: bool) {
        eprintln!("baa: fullscreen tab={} on={on}", tab_index(tab));
    }

    extern "C" fn on_exclusive_access(_: *mut c_void, tab: ffi::TabId, origin: *const c_char, kind: c_int) {
        eprintln!("baa: exclusive_access tab={} origin={:?} kind={kind}", tab_index(tab), text(origin));
    }

    extern "C" fn on_find_result(_: *mut c_void, tab: ffi::TabId, active: c_int, count: c_int) {
        eprintln!("baa: find_result tab={} active={active} count={count}", tab_index(tab));
    }

    extern "C" fn on_tab_group(_: *mut c_void, tab: ffi::TabId, group: u64, title: *const c_char) {
        eprintln!("baa: tab_group tab={} group={group} title={:?}", tab_index(tab), text(title));
    }

    extern "C" fn on_autocomplete(_: *mut c_void, id: u64, matches: *const ffi::Match, count: usize, done: bool) {
        let ms = if count == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(matches, count) } };
        let first = ms.first().map_or(String::new(), |m| format!("{}|{}|{}", text(m.url), text(m.contents), m.search as u8));
        eprintln!("baa: autocomplete id={id} n={count} done={done} first={first:?}");
    }

    extern "C" fn on_clear_done(_: *mut c_void, id: u64) {
        eprintln!("baa: clear_done id={id}");
    }

    extern "C" fn on_load_unpacked_done(_: *mut c_void, id: u64, ok: bool, message: *const c_char) {
        eprintln!("baa: load_unpacked_done id={id} ok={ok} {}", text(message));
    }

    extern "C" fn on_cdp(_: *mut c_void, tab: ffi::TabId, id: u64, json: *const c_char) {
        let json: String = text(json).chars().take(300).collect();
        eprintln!("baa: cdp tab={} id={id} {json}", tab_index(tab));
    }

    #[allow(clippy::too_many_arguments)]
    extern "C" fn on_swipe(
        _: *mut c_void,
        tab: ffi::TabId,
        phase: c_int,
        forward: bool,
        amount: f32,
        _: *const u8,
        width: c_int,
        height: c_int,
    ) {
        eprintln!("baa: swipe tab={} phase={phase} forward={forward} amount={amount:.2} picture={width}x{height}", tab_index(tab));
    }
    extern "C" fn on_focus_requested(_: *mut c_void, tab: ffi::TabId) {
        eprintln!("baa: focus_requested tab={}", tab_index(tab));
    }

    extern "C" fn on_media(_: *mut c_void, tab: ffi::TabId, m: *const ffi::Media) {
        let m = unsafe { &*m };
        eprintln!(
            "baa: media tab={} title={:?} artist={:?} artwork={} playing={} actions={}",
            tab_index(tab),
            text(m.title),
            text(m.artist),
            text(m.artwork_url),
            m.playing,
            m.actions
        );
    }

    extern "C" fn visit_extension(_: *mut c_void, e: *const ffi::Extension) {
        let e = unsafe { &*e };
        eprintln!(
            "baa: extension id={} name={:?} version={} enabled={} unpacked={} site_access={} popup={} options={} icon={} permissions={:?}",
            text(e.id),
            text(e.name),
            text(e.version),
            e.enabled as u8,
            e.unpacked as u8,
            e.site_access,
            text(e.popup_url),
            text(e.options_url),
            text(e.icon_path),
            text(e.permissions)
        );
    }

    extern "C" fn visit_rule(ctx: *mut c_void, pattern: *const c_char, state: c_int) {
        let state = ["ask", "allow", "block", "session"].get(state as usize).copied().unwrap_or("?");
        // SAFETY: ctx is the Vec the `rules` step passes.
        unsafe { &mut *ctx.cast::<Vec<String>>() }.push(format!("{} {state}", text(pattern)));
    }

    fn rule_kind(name: &str) -> u32 {
        const NAMES: [&str; 15] = [
            "camera", "microphone", "location", "notifications", "clipboard", "midi", "other", "popups", "sound",
            "javascript", "cookies", "downloads", "insecure", "sensors", "windows",
        ];
        1 << NAMES.iter().position(|n| *n == name).unwrap_or_else(|| panic!("kind {name:?}"))
    }

    extern "C" fn visit_history(_: *mut c_void, title: *const c_char, url: *const c_char) {
        eprintln!("baa: history entry title={:?} url={}", text(title), text(url));
    }

    /// BAA_SPIKE_SCRIPT=<file>: one step per line, 1 s apart (`sleep <ms>`
    /// adds more). `use <n>` picks the tab later steps act on; 0 is the page.
    ///   load <url> | back | forward | reload | stop | history | state
    ///   eval <js> | geval <js> (with a user gesture) | find <text> | zoom <f>
    ///   cdpeval <js> (Runtime.evaluate through baa_tab_cdp) | media <BAA_MEDIA_* bit>
    ///   mute | unmute | print | activate | focus | close | escape | screenshot | sleep <ms> | quit
    ///   click | cmdclick | rclick <x> <y> (real NSEvents at points from the tab's top left)
    ///   tab <1|2|e> <url> (new tab in space-1, space-2, an ephemeral profile)
    ///   menu <x> <y> | devtools <side 0-2> [inspect <x> <y>] | devtools-window | popup <ext-id> | extensions
    ///   ext <id> enable|disable|remove|access <n> | site <kind> <url> [ask|allow|block]
    ///   prefb|prefi|prefs <1|local> <name> <value> | zoomdefault <f> | engine <name> <kw> <url>
    ///   rules <kind> | rule <kind> <pattern> <state> | clear <what> <since_ms> | unpacked <path> | reload-ext <id> | initscript <js> | schemes <website> <webui> | profile <1|2|e> | prefclear <1|local> <name> | defaultbrowser
    ///   reopen <1|2|e> <url> (close the current tab, make one at once) | closeprofile <1|2|e> | answer <id|last> <0|1> [text] [password] [checked 0|1]
    fn run_script(path: String, cx: &mut App) {
        let script = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        cx.spawn(async move |cx| {
            let executor = cx.background_executor().clone();
            let mut current = 0usize;
            let mut shots = 0;
            let (mut clears, mut unpacked, mut cdp_id) = (0u64, 0u64, 0u64);
            let mut which: &str = "1";
            // The page's own tab exists once the window does.
            executor.timer(std::time::Duration::from_secs(3)).await;
            for line in script.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                let (verb, arg) = line.split_once(' ').unwrap_or((line, ""));
                eprintln!("script: {line}");
                let core = core();
                let tab = TABS.with(|t| t.borrow().get(current).copied()).unwrap_or(0);
                let c = |s: &str| CString::new(s).unwrap();
                match verb {
                    "use" => current = arg.parse().unwrap(),
                    "load" => (core.tab_load_url)(tab, c(arg).as_ptr()),
                    "back" => (core.tab_go_to_offset)(tab, -1),
                    "forward" => (core.tab_go_to_offset)(tab, 1),
                    "reload" => (core.tab_reload)(tab, false),
                    "stop" => (core.tab_stop)(tab),
                    "history" => {
                        let at = (core.tab_history)(tab, visit_history, std::ptr::null_mut());
                        eprintln!("baa: history current={at}");
                    }
                    "state" => {
                        let mut s = empty_state();
                        let ok = (core.tab_get_state)(tab, &mut s);
                        let (url, title) = if ok { (text(s.url), text(s.title)) } else { Default::default() };
                        eprintln!("baa: get_state ok={ok} url={url} title={title:?} loading={}", s.loading);
                    }
                    "cdpeval" => {
                        // What baa-web's eval_value sends (cdp.rs allows Runtime.evaluate).
                        cdp_id += 1;
                        let expression = arg.replace('\\', "\\\\").replace('"', "\\\"");
                        let json = format!(
                            r#"{{"id":{cdp_id},"method":"Runtime.evaluate","params":{{"expression":"{expression}","returnByValue":true,"awaitPromise":true}}}}"#
                        );
                        (core.tab_cdp)(tab, cdp_id, c(&json).as_ptr());
                    }
                    "media" => (core.tab_media_action)(tab, arg.parse().unwrap()),
                    "eval" => (core.tab_eval)(tab, c(arg).as_ptr(), false),
                    "geval" => (core.tab_eval)(tab, c(arg).as_ptr(), true),
                    "ac" => {
                        let (id, q) = arg.split_once(' ').unwrap_or((arg, ""));
                        (core.profile_autocomplete)(profile_for("1"), id.parse().unwrap(), c(q.trim_matches('"')).as_ptr());
                    }
                    "find" => (core.tab_find)(tab, c(arg).as_ptr(), true),
                    // group <tab index> <key> <title>; title "-" is empty
                    "group" => {
                        let mut p = arg.splitn(3, ' ');
                        let (i, key, title) = (p.next().unwrap(), p.next().unwrap(), p.next().unwrap_or("-"));
                        let t = TABS.with(|t| t.borrow().get(i.parse::<usize>().unwrap()).copied()).unwrap_or(0);
                        (core.tab_set_group)(t, key.parse().unwrap(), c(if title == "-" { "" } else { title }).as_ptr());
                    }
                    "fsexit" => (core.tab_exit_fullscreen)(tab),
                    "zoom" => (core.tab_set_zoom)(tab, arg.parse().unwrap()),
                    "mute" => (core.tab_set_muted)(tab, true),
                    "unmute" => (core.tab_set_muted)(tab, false),
                    "print" => (core.tab_print)(tab),
                    "activate" => (core.tab_activate)(tab),
                    "focus" => (core.tab_focus)(tab),
                    "menu" => {
                        let mut s = empty_state();
                        if (core.tab_get_state)(tab, &mut s) {
                            let (x, y) = arg.split_once(' ').unwrap();
                            harness::post_click(s.view.cast(), x.parse().unwrap(), y.parse().unwrap(), false, true);
                        }
                    }
                    // devtools <side> [inspect <x> <y>]: docked DevTools for the current tab.
                    "devtools" => {
                        let (side, rest) = arg.split_once(' ').unwrap_or((arg, ""));
                        let side: c_int = side.parse().unwrap_or(0);
                        let at = rest.strip_prefix("inspect ").map(|a| a.split_once(' ').unwrap());
                        let (inspect, x, y) = at.map_or((false, 0, 0), |(x, y)| (true, x.parse().unwrap(), y.parse().unwrap()));
                        let front = (core.tab_open_devtools)(tab, inspect, x, y, side);
                        let index = TABS.with(|t| {
                            let mut t = t.borrow_mut();
                            t.push(front);
                            t.len() - 1
                        });
                        eprintln!("baa: devtools {current} -> {} ({front:?})", if front == 0 { "NULL".into() } else { index.to_string() });
                    }
                    "devtools-window" => (core.tab_show_devtools_window)(tab, false, 0, 0),
                    "popup" => {
                        let id = CString::new(arg).unwrap();
                        let popup = (core.extension_popup)(profile_for("1"), id.as_ptr());
                        let index = TABS.with(|t| {
                            let mut t = t.borrow_mut();
                            t.push(popup);
                            t.len() - 1
                        });
                        eprintln!("baa: popup {arg} -> {} ({popup:?})", if popup == 0 { "NULL".into() } else { index.to_string() });
                    }
                    "extensions" => {
                        let n = (core.profile_extensions)(profile_for("1"), visit_extension, std::ptr::null_mut());
                        eprintln!("baa: extensions count={n}");
                    }
                    // ext <id> enable|disable|remove|access <n>
                    "ext" => {
                        let mut p = arg.splitn(3, ' ');
                        let id = CString::new(p.next().unwrap()).unwrap();
                        let (change, value) = match (p.next().unwrap(), p.next()) {
                            ("enable", _) => (0, 1),
                            ("disable", _) => (0, 0),
                            ("remove", _) => (1, 0),
                            ("access", Some(n)) => (2, n.parse().unwrap()),
                            other => panic!("ext {other:?}"),
                        };
                        let ok = (core.profile_extension_change)(profile_for("1"), id.as_ptr(), change, value);
                        eprintln!("baa: ext {arg} -> {ok}");
                    }
                    // site <kind> <url> [ask|allow|block]
                    "site" => {
                        let mut p = arg.splitn(3, ' ');
                        let kind = rule_kind(p.next().unwrap());
                        let url = CString::new(p.next().unwrap()).unwrap();
                        if let Some(state) = p.next() {
                            let state = match state {
                                "allow" => 1,
                                "block" => 2,
                                _ => 0,
                            };
                            (core.profile_set_site_permission)(profile_for(which), url.as_ptr(), kind, state);
                        }
                        let now = (core.profile_site_permission)(profile_for(which), url.as_ptr(), kind);
                        eprintln!("baa: site {arg} = {}", ["ask", "allow", "block", "session"][now as usize]);
                    }
                    // prefb|prefi|prefs <1|local> <name> <value>: `local` is Local State.
                    "prefb" | "prefi" | "prefs" => {
                        let mut p = arg.splitn(3, ' ');
                        let (which, name, value) = (p.next().unwrap(), p.next().unwrap(), p.next().unwrap());
                        let profile = if which == "local" { std::ptr::null_mut() } else { profile_for(which) };
                        let ok = match verb {
                            "prefb" => (core.pref_set_bool)(profile, c(name).as_ptr(), value == "true" || value == "1"),
                            "prefi" => (core.pref_set_int)(profile, c(name).as_ptr(), value.parse().unwrap()),
                            _ => (core.pref_set_string)(profile, c(name).as_ptr(), c(value).as_ptr()),
                        };
                        eprintln!("baa: {verb} {arg} -> {}", ok as u8);
                    }
                    "zoomdefault" => {
                        (core.profile_set_default_zoom)(profile_for(which), arg.parse().unwrap());
                        eprintln!("baa: zoomdefault {arg}");
                    }
                    // engine <name> <keyword> <url> (a keyword of `-` is empty)
                    "engine" => {
                        let mut p = arg.splitn(3, ' ');
                        let (name, keyword, url) = (p.next().unwrap(), p.next().unwrap(), p.next().unwrap());
                        let keyword = if keyword == "-" { "" } else { keyword };
                        let ok = (core.profile_set_search_engine)(profile_for(which), c(name).as_ptr(), c(keyword).as_ptr(), c(url).as_ptr());
                        eprintln!("baa: engine {arg} -> {}", ok as u8);
                    }
                    "rules" => {
                        let mut rows: Vec<String> = Vec::new();
                        let ctx = std::ptr::addr_of_mut!(rows).cast();
                        let n = (core.profile_site_rules)(profile_for(which), rule_kind(arg), visit_rule, ctx);
                        eprintln!("baa: rules {arg} -> {n} [{}]", rows.join(", "));
                    }
                    // rule <kind> <pattern> <ask|allow|block|session|unset>
                    "rule" => {
                        let mut p = arg.splitn(3, ' ');
                        let (kind, pattern, state) = (p.next().unwrap(), p.next().unwrap(), p.next().unwrap());
                        let state = match state {
                            "unset" => -1,
                            "ask" => 0,
                            "allow" => 1,
                            "block" => 2,
                            "session" => 3,
                            other => panic!("state {other:?}"),
                        };
                        let ok = (core.profile_set_site_rule)(profile_for(which), rule_kind(kind), c(pattern).as_ptr(), state);
                        eprintln!("baa: rule {arg} -> {}", ok as u8);
                    }
                    // clear <history|downloads|site|cache|all> <since_ms>
                    "clear" => {
                        let (what, since) = arg.split_once(' ').unwrap();
                        let bits = match what {
                            "history" => 1,
                            "downloads" => 2,
                            "site" => 4,
                            "cache" => 8,
                            "all" => 15,
                            other => panic!("clear {other:?}"),
                        };
                        clears += 1;
                        (core.profile_clear_data)(profile_for(which), clears, since.parse().unwrap(), bits);
                        eprintln!("baa: clear {arg} id={clears}");
                    }
                    "unpacked" => {
                        unpacked += 1;
                        (core.profile_load_unpacked)(profile_for(which), unpacked, c(arg).as_ptr());
                        eprintln!("baa: unpacked {arg} id={unpacked}");
                    }
                    "reload-ext" => {
                        let ok = (core.profile_extension_change)(profile_for(which), c(arg).as_ptr(), 3, 0);
                        eprintln!("baa: reload-ext {arg} -> {ok}");
                    }
                    // shell <cmd>: lets a test change files the engine reads (an unpacked extension's manifest).
                    "shell" => {
                        let ok = std::process::Command::new("sh").args(["-c", arg]).status().is_ok_and(|s| s.success());
                        eprintln!("baa: shell -> {}", ok as u8);
                    }
                    "initscript" => (core.tab_set_init_script)(tab, c(arg).as_ptr()),
                    // schemes <website> <webui>: 0 system, 1 light, 2 dark
                    "schemes" => {
                        let (website, webui) = arg.split_once(' ').unwrap();
                        (core.profile_set_color_schemes)(profile_for(which), website.parse().unwrap(), webui.parse().unwrap());
                    }
                    // profile <1|2|e>: the profile the settings steps (prefs, rules, engine, zoom, ...) act on
                    "profile" => which = Box::leak(arg.to_string().into_boxed_str()),
                    "prefclear" => {
                        let (w, name) = arg.split_once(' ').unwrap();
                        let profile = if w == "local" { std::ptr::null_mut() } else { profile_for(w) };
                        let ok = (core.pref_clear)(profile, c(name).as_ptr());
                        eprintln!("baa: prefclear {arg} -> {}", ok as u8);
                    }
                    "defaultbrowser" => eprintln!("baa: defaultbrowser -> {}", (core.is_default_browser)() as u8),
                    "close" => {
                        (core.tab_close)(tab);
                        if current == 0 {
                            PAGE_CLOSED.with(|c| c.set(true));
                        }
                    }
                    "escape" => {
                        let mut s = empty_state();
                        if (core.tab_get_state)(tab, &mut s) {
                            harness::post_key(harness::window_number(s.view.cast()), 53, "\u{1b}", false);
                        }
                    }
                    "screenshot" => {
                        shots += 1;
                        let file = format!("/tmp/baa-shot-{shots}.png");
                        let _ = std::process::Command::new("screencapture").args(["-x", &file]).status();
                        eprintln!("script: screenshot {file}");
                    }
                    "click" | "cmdclick" | "rclick" => {
                        let mut s = empty_state();
                        if (core.tab_get_state)(tab, &mut s) {
                            let (x, y) = arg.split_once(' ').unwrap();
                            harness::post_click(s.view.cast(), x.parse().unwrap(), y.parse().unwrap(), verb == "cmdclick", verb == "rclick");
                        }
                    }
                    "tab" => {
                        let (which, url) = arg.split_once(' ').unwrap_or((arg, "about:blank"));
                        let new = (core.tab_create)(profile_for(which), c(url).as_ptr());
                        let index = TABS.with(|t| {
                            let mut t = t.borrow_mut();
                            t.push(new);
                            t.len() - 1
                        });
                        eprintln!("script: tab {index} = {new:?}");
                    }
                    "closeprofile" => {
                        (core.profile_close)(profile_for(arg));
                        // The handle is dead; the next `tab` opens the profile again.
                        PROFILES.with(|p| p.borrow_mut()[slot_of(arg)] = std::ptr::null_mut());
                    }
                    // Close the current tab and make one in the same profile at once.
                    "reopen" => {
                        let (which, url) = arg.split_once(' ').unwrap();
                        (core.tab_close)(tab);
                        let new = (core.tab_create)(profile_for(which), c(url).as_ptr());
                        let index = TABS.with(|t| {
                            let mut t = t.borrow_mut();
                            t.push(new);
                            t.len() - 1
                        });
                        eprintln!("script: tab {index} = {new:?}");
                    }
                    "answer" => {
                        let mut parts = arg.splitn(5, ' ');
                        let id = match parts.next().unwrap() {
                            "last" => LAST_DIALOG.with(Cell::get),
                            id => id.parse().unwrap(),
                        };
                        let ok = parts.next() == Some("1");
                        let (user, password) = (c(parts.next().unwrap_or("")), c(parts.next().unwrap_or("")));
                        let checked = parts.next() == Some("1");
                        (core.answer)(id, &ffi::Answer { ok, text: user.as_ptr(), password: password.as_ptr(), checked });
                    }
                    "sleep" => executor.timer(std::time::Duration::from_millis(arg.parse().unwrap())).await,
                    "quit" => {
                        eprintln!("spike: quitting");
                        cx.update(|cx| cx.quit());
                    }
                    other => eprintln!("script: unknown step {other:?}"),
                }
                executor.timer(std::time::Duration::from_secs(1)).await;
            }
            eprintln!("script: done");
        })
        .detach();
    }

    fn slot_of(which: &str) -> usize {
        match which {
            "1" => 0,
            "2" => 1,
            "e" => 2,
            other => panic!("profile {other:?}"),
        }
    }

    /// Opens (once) space-1, space-2 or the ephemeral profile.
    fn profile_for(which: &str) -> *mut ffi::Profile {
        let core = core();
        let slot = slot_of(which);
        PROFILES.with(|p| {
            let mut p = p.borrow_mut();
            if p[slot].is_null() {
                p[slot] = match slot {
                    // BAA_SPIKE_PROFILE=Default puts the page where --load-extension
                    // puts extensions (the start-up profile).
                    0 => {
                        let name = std::env::var("BAA_SPIKE_PROFILE").unwrap_or_else(|_| "space-1".into());
                        (core.profile_open)(CString::new(name).unwrap().as_ptr())
                    }
                    1 => (core.profile_open)(c"space-2".as_ptr()),
                    _ => (core.profile_open_ephemeral)(),
                };
                eprintln!("script: profile {which} = {:?}", p[slot]);
            }
            p[slot]
        })
    }

    struct Spike {
        bar_focus: FocusHandle,
        page_focus: FocusHandle,
        typed: String,
        status: SharedString,
        page: Rc<Page>,
    }

    impl Spike {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            let gpui_view = native::gpui_view(window);
            GPUI_VIEW.with(|v| v.set(gpui_view));
            native::install_mouse_forwarding(gpui_view);
            let url = std::env::var("BAA_SPIKE_URL").unwrap_or_else(|_| "https://en.wikipedia.org/wiki/Web_browser".into());
            let profile = profile_for("1");
            let url = CString::new(url).unwrap();
            let tab = (core().tab_create)(profile, url.as_ptr());
            assert!(tab != 0, "baa_tab_create failed");
            TABS.with(|t| t.borrow_mut().push(tab));
            let mut state = empty_state();
            assert!((core().tab_get_state)(tab, &mut state));
            let view = state.view.cast::<AnyObject>();
            eprintln!("spike: page created, view {view:?}");
            let page = Rc::new(Page {
                tab,
                view,
                container: native::Container::new(gpui_view, view, 10.),
                drag: Cell::new(None),
                inside: Cell::new(false),
            });
            Self {
                bar_focus: cx.focus_handle(),
                page_focus: cx.focus_handle(),
                typed: String::new(),
                status: "".into(),
                page,
            }
        }

        fn focus_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            native::make_first_responder(GPUI_VIEW.with(Cell::get));
            window.focus(&self.bar_focus, cx);
            cx.notify();
        }
    }

    impl Render for Spike {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let bar_focused = self.bar_focus.is_focused(window);
            div()
                .key_context("Spike")
                .size_full()
                .flex()
                .flex_col()
                .bg(rgb(0x2b2d31))
                .p_2()
                .gap_2()
                .on_action(cx.listener(|this, _: &FocusBar, window, cx| {
                    eprintln!("spike: gpui action FocusBar");
                    this.status = "⌘L reached gpui".into();
                    this.focus_bar(window, cx);
                }))
                .on_action(cx.listener(|this, _: &NewTab, _, cx| {
                    eprintln!("spike: gpui action NewTab");
                    this.status = "⌘T reached gpui".into();
                    cx.notify();
                }))
                .child(
                    div()
                        .id("bar")
                        .track_focus(&self.bar_focus)
                        .h(px(32.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_4()
                        .rounded_md()
                        .bg(rgb(if bar_focused { 0x45474d } else { 0x383a40 }))
                        .text_color(rgb(0xdddddd))
                        .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| this.focus_bar(window, cx)))
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                            let keystroke = &event.keystroke;
                            if keystroke.key == "backspace" {
                                this.typed.pop();
                            } else if let Some(text) = keystroke.key_char.as_ref().filter(|_| !keystroke.modifiers.platform) {
                                this.typed.push_str(text);
                            }
                            cx.notify();
                        }))
                        .child(SharedString::from(format!("bar: {}{}", self.typed, if bar_focused { "▏" } else { "" })))
                        .child(div().text_color(rgb(0x8be9a0)).child(self.status.clone())),
                )
                .child(
                    div()
                        .flex_1()
                        .track_focus(&self.page_focus)
                        .child(PageSlot { page: self.page.clone(), focus: self.page_focus.clone() }),
                )
        }
    }

    struct Page {
        tab: ffi::TabId,
        view: *mut AnyObject,
        container: native::Container,
        /// The view a drag started on; the drag stays with it.
        drag: Cell<Option<Retained<AnyObject>>>,
        inside: Cell<bool>,
    }

    /// The hole the page shows through; forwards the mouse to it.
    struct PageSlot {
        page: Rc<Page>,
        focus: FocusHandle,
    }

    impl IntoElement for PageSlot {
        type Element = Self;
        fn into_element(self) -> Self {
            self
        }
    }

    impl Element for PageSlot {
        type RequestLayoutState = ();
        type PrepaintState = Hitbox;

        fn id(&self) -> Option<ElementId> {
            None
        }

        fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
            None
        }

        fn request_layout(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            window: &mut Window,
            cx: &mut App,
        ) -> (LayoutId, ()) {
            let mut style = Style::default();
            style.size.width = relative(1.).into();
            style.size.height = relative(1.).into();
            (window.request_layout(style, [], cx), ())
        }

        fn prepaint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            bounds: Bounds<Pixels>,
            _: &mut (),
            window: &mut Window,
            _: &mut App,
        ) -> Hitbox {
            let visible = bounds.intersect(&window.content_mask().bounds);
            if !PAGE_CLOSED.with(Cell::get) && self.page.container.place(bounds, visible) {
                window.request_animation_frame();
            }
            window.insert_hitbox(bounds, HitboxBehavior::Normal)
        }

        fn paint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            bounds: Bounds<Pixels>,
            _: &mut (),
            hitbox: &mut Hitbox,
            window: &mut Window,
            _: &mut App,
        ) {
            let radius = (10. * window.scale_factor()).round() as usize;
            if let Some(hole) = native::punch_buffer(radius, 0) {
                window.paint_surface(bounds, hole);
            }

            let (page, hit, focus) = (self.page.clone(), hitbox.clone(), self.focus.clone());
            window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && hit.is_hovered(window) {
                    window.focus(&focus, cx);
                    (core().tab_focus)(page.tab);
                    page.drag.set(native::forward_current_event(page.view, None));
                }
            });
            let (page, hit) = (self.page.clone(), hitbox.clone());
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if let Some(view) = page.drag.take().filter(|_| event.pressed_button.is_some()) {
                    page.drag.set(native::forward_current_event(page.view, Some(&view)));
                    return;
                }
                // Moves over the page, and the first one off it so hover ends.
                let over = hit.is_hovered(window);
                if over || page.inside.get() {
                    page.inside.set(over);
                    native::forward_current_event(page.view, None);
                }
            });
            let page = self.page.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, _| {
                if phase == DispatchPhase::Bubble {
                    if let Some(view) = page.drag.take() {
                        native::forward_current_event(page.view, Some(&view));
                    }
                }
            });
            let (page, hit) = (self.page.clone(), hitbox.clone());
            window.on_mouse_event(move |_: &ScrollWheelEvent, phase, window, _| {
                if phase == DispatchPhase::Bubble && hit.should_handle_scroll(window) {
                    native::forward_current_event(page.view, None);
                }
            });
        }
    }
}

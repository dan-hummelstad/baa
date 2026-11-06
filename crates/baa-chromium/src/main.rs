//! Phase 2 spike: a gpui window with a bar and one Chromium page from
//! BaaCore.framework. Run it with `crates/baa-chromium/spike.sh`.
//! Start-up order and why: PLAN-chromium-phase2.md.

#[cfg(target_os = "macos")]
mod ffi;
#[cfg(target_os = "macos")]
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
    use objc2::runtime::AnyObject;

    use crate::{ffi, native};

    actions!(spike, [Quit, FocusBar, NewTab]);

    thread_local! {
        static GPUI_VIEW: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) };    }

    pub fn main() {
        // Before anything can start a thread (see ffi::load).
        let core = ffi::load();

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
            if let Ok(path) = std::env::var("BAA_SPIKE_SCRIPT") {
                run_script(path, cx);
            }
            // BAA_SPIKE_QUIT_AFTER=<s>: quit the way ⌘Q does, for smoke.sh.
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
                    (ffi::core().tab_focus)(spike.page.tab);
                    native::window_number(spike.page.view)
                })
                .unwrap();
            wait(500).await;
            // Into the textarea the way a user gets there: a click.
            let view = spike.update(cx, |spike, _, _| spike.page.view).unwrap();
            native::post_click(view, 200., 140., false, false);
            wait(500).await;
            for &(code, chars, command) in PAGE {
                native::post_key(window_number, code, chars, command);
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
            return native::offer_key_equivalent(GPUI_VIEW.with(Cell::get), event);
        }
        match native::edit_command(event) {
            Some(command) => {
                native::send_edit_action(command);
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
        dialog_closed: on_dialog_closed,
        tab_closed: on_tab_closed,
    };

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
        static TABS: RefCell<Vec<ffi::Tab>> = const { RefCell::new(Vec::new()) };
        /// space-1, space-2, ephemeral.
        static PROFILES: RefCell<[ffi::Profile; 3]> = const { RefCell::new([std::ptr::null_mut(); 3]) };
    }

    fn tab_index(tab: ffi::Tab) -> String {
        TABS.with(|t| t.borrow().iter().position(|x| *x == tab)).map_or("?".into(), |i| i.to_string())
    }

    extern "C" fn on_tab_created(
        _: *mut c_void,
        tab: ffi::Tab,
        opener: ffi::Tab,
        profile: ffi::Profile,
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
            if opener.is_null() { "NULL".into() } else { tab_index(opener) },
            if profile.is_null() { "NULL".into() } else { format!("{which:?}") },
            text(url),
        );
    }

    extern "C" fn on_tab_state(_: *mut c_void, tab: ffi::Tab, s: *const ffi::TabState) {
        let s = unsafe { &*s };
        eprintln!(
            "baa: tab_state tab={} url={} title={:?} icon={} loading={} back={} forward={} audible={} secure={}",
            tab_index(tab),
            text(s.url),
            text(s.title),
            text(s.favicon_url),
            s.loading,
            s.can_go_back,
            s.can_go_forward,
            s.audible,
            s.secure
        );
    }

    extern "C" fn on_tab_favicon(_: *mut c_void, tab: ffi::Tab, url: *const c_char, png: *const u8, len: usize) {
        let head = unsafe { std::slice::from_raw_parts(png, len.min(8)) };
        eprintln!("baa: tab_favicon tab={} url={} bytes={len} magic={head:02x?}", tab_index(tab), text(url));
    }

    extern "C" fn on_dialog(_: *mut c_void, tab: ffi::Tab, id: u64, kind: c_int, message: *const c_char, default: *const c_char) {
        eprintln!("baa: dialog tab={} id={id} kind={kind} message={:?} default={:?}", tab_index(tab), text(message), text(default));
    }

    extern "C" fn on_dialog_closed(_: *mut c_void, tab: ffi::Tab, id: u64) {
        eprintln!("baa: dialog_closed tab={} id={id}", tab_index(tab));
    }

    extern "C" fn on_tab_closed(_: *mut c_void, tab: ffi::Tab) {
        eprintln!("baa: tab_closed tab={}", tab_index(tab));
        if tab_index(tab) == "0" {
            PAGE_CLOSED.with(|c| c.set(true));
        }
    }

    extern "C" fn visit_history(_: *mut c_void, title: *const c_char, url: *const c_char) {
        eprintln!("baa: history entry title={:?} url={}", text(title), text(url));
    }

    /// BAA_SPIKE_SCRIPT=<file>: one step per line, 1 s apart (`sleep <ms>`
    /// adds more). `use <n>` picks the tab later steps act on; 0 is the page.
    ///   load <url> | back | forward | reload | stop | history | state
    ///   eval <js> | geval <js> (with a user gesture) | find <text> | zoom <f>
    ///   mute | unmute | print | activate | focus | close | escape | screenshot | sleep <ms> | quit
    ///   click | cmdclick | rclick <x> <y> (real NSEvents at points from the tab's top left)
    ///   tab <1|2|e> <url> (new tab in space-1, space-2, an ephemeral profile)
    ///   reopen <1|2|e> <url> (close the current tab, make one at once) | closeprofile <1|2|e> | answer <id> <0|1> [text]
    fn run_script(path: String, cx: &mut App) {
        let script = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        cx.spawn(async move |cx| {
            let executor = cx.background_executor().clone();
            let mut current = 0usize;
            let mut shots = 0;
            // The page's own tab exists once the window does.
            executor.timer(std::time::Duration::from_secs(3)).await;
            for line in script.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                let (verb, arg) = line.split_once(' ').unwrap_or((line, ""));
                eprintln!("script: {line}");
                let core = ffi::core();
                let tab = TABS.with(|t| t.borrow().get(current).copied()).unwrap_or(std::ptr::null_mut());
                let c = |s: &str| CString::new(s).unwrap();
                match verb {
                    "use" => current = arg.parse().unwrap(),
                    "load" => (core.tab_load_url)(tab, c(arg).as_ptr()),
                    "back" => (core.tab_go_to_offset)(tab, -1),
                    "forward" => (core.tab_go_to_offset)(tab, 1),
                    "reload" => (core.tab_reload)(tab),
                    "stop" => (core.tab_stop)(tab),
                    "history" => {
                        let at = (core.tab_history)(tab, visit_history, std::ptr::null_mut());
                        eprintln!("baa: history current={at}");
                    }
                    "state" => {
                        let mut s = ffi::TabState::empty();
                        let ok = (core.tab_get_state)(tab, &mut s);
                        let (url, title) = if ok { (text(s.url), text(s.title)) } else { Default::default() };
                        eprintln!("baa: get_state ok={ok} url={url} title={title:?} loading={}", s.loading);
                    }
                    "eval" => (core.tab_eval)(tab, c(arg).as_ptr(), false),
                    "geval" => (core.tab_eval)(tab, c(arg).as_ptr(), true),
                    "find" => (core.tab_find)(tab, c(arg).as_ptr(), true),
                    "zoom" => (core.tab_set_zoom)(tab, arg.parse().unwrap()),
                    "mute" => (core.tab_set_muted)(tab, true),
                    "unmute" => (core.tab_set_muted)(tab, false),
                    "print" => (core.tab_print)(tab),
                    "activate" => (core.tab_activate)(tab),
                    "focus" => (core.tab_focus)(tab),
                    "close" => {
                        (core.tab_close)(tab);
                        if current == 0 {
                            PAGE_CLOSED.with(|c| c.set(true));
                        }
                    }
                    "escape" => {
                        let mut s = ffi::TabState::empty();
                        if (core.tab_get_state)(tab, &mut s) {
                            native::post_key(native::window_number(s.view.cast()), 53, "\u{1b}", false);
                        }
                    }
                    "screenshot" => {
                        shots += 1;
                        let file = format!("/tmp/baa-shot-{shots}.png");
                        let _ = std::process::Command::new("screencapture").args(["-x", &file]).status();
                        eprintln!("script: screenshot {file}");
                    }
                    "click" | "cmdclick" | "rclick" => {
                        let mut s = ffi::TabState::empty();
                        if (core.tab_get_state)(tab, &mut s) {
                            let (x, y) = arg.split_once(' ').unwrap();
                            native::post_click(s.view.cast(), x.parse().unwrap(), y.parse().unwrap(), verb == "cmdclick", verb == "rclick");
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
                        let mut parts = arg.splitn(3, ' ');
                        let id: u64 = parts.next().unwrap().parse().unwrap();
                        let ok = parts.next() == Some("1");
                        (core.dialog_answer)(id, ok, c(parts.next().unwrap_or("")).as_ptr());
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
    fn profile_for(which: &str) -> ffi::Profile {
        let core = ffi::core();
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
            let tab = (ffi::core().tab_create)(profile, url.as_ptr());
            assert!(!tab.is_null(), "baa_tab_create failed");
            TABS.with(|t| t.borrow_mut().push(tab));
            let mut state = ffi::TabState::empty();
            assert!((ffi::core().tab_get_state)(tab, &mut state));
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
        tab: ffi::Tab,
        view: *mut AnyObject,
        container: native::Container,
        /// The view a drag started on; the drag stays with it.
        drag: Cell<Option<*mut AnyObject>>,
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
            if !PAGE_CLOSED.with(Cell::get) && self.page.container.place(self.page.view, bounds, visible) {
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
            if let Some(hole) = native::punch_buffer(radius) {
                window.paint_surface(bounds, hole);
            }

            let (page, hit, focus) = (self.page.clone(), hitbox.clone(), self.focus.clone());
            window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && hit.is_hovered(window) {
                    window.focus(&focus, cx);
                    (ffi::core().tab_focus)(page.tab);
                    page.drag.set(native::forward_current_event(page.view, None));
                }
            });
            let (page, hit) = (self.page.clone(), hitbox.clone());
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if let Some(view) = page.drag.get().filter(|_| event.pressed_button.is_some()) {
                    native::forward_current_event(page.view, Some(view));
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
                        native::forward_current_event(page.view, Some(view));
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

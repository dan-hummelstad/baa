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
        cell::Cell,
        ffi::{c_char, c_void, CString},
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
        (core.install_app)(argv.len() as _, argv.as_ptr(), on_key, std::ptr::null_mut());
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
                    (ffi::core().web_contents_focus)(spike.page.contents);
                    native::window_number(spike.page.view)
                })
                .unwrap();
            wait(500).await;
            // Into the textarea the way a user gets there: a click.
            let view = spike.update(cx, |spike, _, _| spike.page.view).unwrap();
            native::post_click(view, 200., 140.);
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
    /// alone go back to the page as edit commands (nothing else would run them:
    /// there is no Chrome window or menu behind the page).
    extern "C" fn on_key(_: *mut c_void, page: ffi::WebContents, event: *mut c_void, after_page: bool) -> bool {
        let event = event.cast::<AnyObject>();
        if !after_page {
            return native::offer_key_equivalent(GPUI_VIEW.with(Cell::get), event);
        }
        match native::edit_command(event) {
            Some(command) => {
                (ffi::core().web_contents_edit)(page, command);
                true
            }
            None => false,
        }
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
            let core = ffi::core();
            let contents = (core.web_contents_create)(CString::new(url).unwrap().as_ptr());
            let view = (core.web_contents_view)(contents).cast::<AnyObject>();
            eprintln!("spike: page created, view {view:?}");
            let page = Rc::new(Page {
                contents,
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
        contents: ffi::WebContents,
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
            if self.page.container.place(self.page.view, bounds, visible) {
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
                    (ffi::core().web_contents_focus)(page.contents);
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

//! Arc's Space editors: the theme colour (a dotted canvas, angle hue and radius
//! vividness; the swatches; a lightness slider; a dial for how far the top stop
//! fades toward white) and the icon (every emoji macOS has a name for).
//! Each is its own view so the browser window and the settings window can both
//! hang one; they change the Space through the browser, live.

use std::{cell::Cell, ops::Range, rc::Rc, sync::LazyLock};

use baa_model::SpaceId;
use baa_web::WebHost;
use gpui::{
    anchored, canvas, deferred, div, prelude::*, px, AnyView, Bounds, Context,
    DismissEvent, Entity, EventEmitter, FocusHandle, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Rgba, ScrollStrategy, SharedString, Stateful, Subscription, UniformListScrollHandle,
    WeakEntity, Window,
};

use crate::{
    browser::{drop_in, shadow, Browser, PALETTE},
    icons::icon,
    input::{Key, TextField},
    theme::{from_oklch, r, surface, to_oklch},
};

/// An open picker and where it hangs.
pub(crate) struct Shown {
    view: AnyView,
    at: Point<Pixels>,
    _dismiss: Subscription,
}

impl Shown {
    /// The emoji picker (`icon`) or the colour picker for `space`, at `at` in
    /// `cx`'s window; `closed` runs once it goes. `space` is passed in, not read
    /// through `browser`: the browser may be mid-update (a menu item opening this).
    pub(crate) fn open<H: WebHost, T: 'static>(
        icon: bool,
        browser: &Entity<Browser<H>>,
        space: &baa_model::Space,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<T>,
        closed: fn(&mut T, &mut Window, &mut Context<T>),
    ) -> Self {
        let browser = browser.downgrade();
        fn watch<T: 'static, V: EventEmitter<DismissEvent>>(
            p: &Entity<V>,
            window: &mut Window,
            cx: &mut Context<T>,
            closed: fn(&mut T, &mut Window, &mut Context<T>),
        ) -> Subscription {
            cx.subscribe_in(p, window, move |this, _, _: &DismissEvent, window, cx| {
                closed(this, window, cx);
                cx.notify();
            })
        }
        let (view, focus, sub): (AnyView, FocusHandle, Subscription) = if icon {
            let p = cx.new(|cx| EmojiPicker::new(browser, space.id, space.icon.clone(), cx));
            (p.clone().into(), p.read(cx).focus.clone(), watch(&p, window, cx, closed))
        } else {
            let bottom = space.accent_to.unwrap_or(space.accent);
            let p = cx.new(|cx| ColorPicker::new(browser, space.id, space.accent, bottom, cx));
            (p.clone().into(), p.read(cx).focus.clone(), watch(&p, window, cx, closed))
        };
        window.focus(&focus, cx);
        Self { view, at, _dismiss: sub }
    }

    pub(crate) fn render(&self) -> impl IntoElement {
        // The backdrop keeps the click that closes it from landing underneath.
        deferred(
            div().absolute().inset_0().occlude().child(
                anchored()
                    .position(self.at)
                    .snap_to_window_with_margin(px(8.))
                    .child(drop_in(div().child(self.view.clone()), "space-picker")),
            ),
        )
    }
}

fn card(id: &'static str) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .occlude()
        .p(px(12.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .rounded(r(14.))
        .bg(surface::bg())
        .border_1()
        .border_color(surface::border())
        .shadow(vec![shadow(gpui::rgba(0x18122C29).into(), 8., 24.)])
        .text_size(px(13.))
        .text_color(surface::ink())
}

/// Saves what the picker changed and asks its owner to drop it.
fn dismiss<H: WebHost, V: EventEmitter<DismissEvent>>(browser: &WeakEntity<Browser<H>>, cx: &mut Context<V>) {
    if let Some(b) = browser.upgrade() {
        b.update(cx, |b, _| b.save());
    }
    cx.emit(DismissEvent);
}

/// The canvas: hue is the angle around its centre, vividness the distance out.
const CANVAS: (f32, f32) = (340., 336.);
/// Distance from the centre at full vividness.
const RING: f32 = 140.;
/// Vividness at the ring: about what sRGB holds at mid lightness.
const MAX_C: f32 = 0.16;
/// The slider's ends, dark to light (OKLCH L).
const LIGHT: (f32, f32) = (0.30, 0.95);
const TRACK: (f32, f32) = (224., 27.5);
const THUMB: (f32, f32) = (20., 42.);
const DIAL: f32 = 88.;
const TICKS: usize = 28;

#[derive(Clone, Copy)]
enum Drag {
    Canvas,
    Light,
    Spread,
}

pub(crate) struct ColorPicker<H: WebHost> {
    browser: WeakEntity<Browser<H>>,
    space: SpaceId,
    /// OKLCH of the Space's bottom stop, the big stop on the canvas.
    lch: [f32; 3],
    /// How far the top stop is mixed toward white: the dial, 0..=1.
    spread: f32,
    drag: Option<Drag>,
    canvas: Rc<Cell<Bounds<Pixels>>>,
    slider: Rc<Cell<Bounds<Pixels>>>,
    dial: Rc<Cell<Bounds<Pixels>>>,
    focus: FocusHandle,
}

impl<H: WebHost> EventEmitter<DismissEvent> for ColorPicker<H> {}

/// The gradient's top stop: the bottom one `spread` of the way to white.
fn top_of(bottom: Rgba, spread: f32) -> Rgba {
    let up = |v: f32| v + (1. - v) * spread;
    Rgba { r: up(bottom.r), g: up(bottom.g), b: up(bottom.b), a: 1. }
}

/// `top_of` undone, read off the channel with the most headroom (the least rounding).
fn spread_of(top: Rgba, bottom: Rgba) -> f32 {
    let (t, b) = [(top.r, bottom.r), (top.g, bottom.g), (top.b, bottom.b)]
        .into_iter()
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .unwrap_or_default();
    if b > 0.98 { 0. } else { ((t - b) / (1. - b)).clamp(0., 1.) }
}

fn hex(c: Rgba) -> u32 {
    let b = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    b(c.r) << 16 | b(c.g) << 8 | b(c.b)
}

/// `at` within `b`, each axis 0..=1.
fn fraction(at: Point<Pixels>, b: Bounds<Pixels>) -> (f32, f32) {
    let f = |p: Pixels, o: Pixels, len: Pixels| (f32::from(p - o) / f32::from(len).max(1.)).clamp(0., 1.);
    (f(at.x, b.origin.x, b.size.width), f(at.y, b.origin.y, b.size.height))
}

/// Offset from the canvas centre (y down) of a colour's vividness and hue.
fn on_canvas(c: f32, h: f32) -> (f32, f32) {
    let d = (c / MAX_C).min(1.) * RING;
    let a = h.to_radians();
    (a.cos() * d, -a.sin() * d)
}

/// `on_canvas` undone: (vividness, hue) at an offset from the centre.
fn from_canvas(dx: f32, dy: f32) -> (f32, f32) {
    (((dx * dx + dy * dy).sqrt() / RING).min(1.) * MAX_C, (-dy).atan2(dx).to_degrees().rem_euclid(360.))
}

fn ring(size: f32, border: f32) -> gpui::Div {
    div()
        .absolute()
        .size(px(size))
        .rounded_full()
        .border(px(border))
        .border_color(gpui::white())
        .shadow(vec![shadow(gpui::rgba(0x2A253A2B).into(), 2., 8.)])
}

fn glow(color: Hsla, blur: f32, spread: f32) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color,
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    }]
}

impl<H: WebHost> ColorPicker<H> {
    fn new(browser: WeakEntity<Browser<H>>, space: SpaceId, top: u32, bottom: u32, cx: &mut Context<Self>) -> Self {
        Self {
            browser,
            space,
            lch: to_oklch(gpui::rgb(bottom)),
            spread: spread_of(gpui::rgb(top), gpui::rgb(bottom)),
            drag: None,
            canvas: Rc::default(),
            slider: Rc::default(),
            dial: Rc::default(),
            focus: cx.focus_handle(),
        }
    }

    fn set(&mut self, top: u32, bottom: u32, cx: &mut Context<Self>) {
        let space = self.space;
        if let Some(b) = self.browser.upgrade() {
            b.update(cx, |b, cx| b.set_space_gradient(space, top, bottom, cx));
        }
        cx.notify();
    }

    fn swatch(&mut self, i: usize, cx: &mut Context<Self>) {
        let (top, bottom) = PALETTE[i];
        self.lch = to_oklch(gpui::rgb(bottom));
        self.spread = spread_of(gpui::rgb(top), gpui::rgb(bottom));
        self.set(top, bottom, cx);
    }

    fn drag_to(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        let off = |b: Bounds<Pixels>| (f32::from(at.x - b.center().x), f32::from(at.y - b.center().y));
        match self.drag {
            Some(Drag::Canvas) => {
                let (dx, dy) = off(self.canvas.get());
                (self.lch[1], self.lch[2]) = from_canvas(dx, dy);
            }
            Some(Drag::Light) => self.lch[0] = LIGHT.0 + fraction(at, self.slider.get()).0 * (LIGHT.1 - LIGHT.0),
            Some(Drag::Spread) => {
                let (dx, dy) = off(self.dial.get());
                self.spread = dx.atan2(-dy).to_degrees().rem_euclid(360.) / 360.;
            }
            None => return,
        }
        let bottom = from_oklch(self.lch);
        self.set(hex(top_of(bottom, self.spread)), hex(bottom), cx);
    }

    fn start(drag: Drag) -> impl Fn(&mut Self, &MouseDownEvent, &mut Window, &mut Context<Self>) {
        move |this, e, _, cx| {
            this.drag = Some(drag);
            this.drag_to(e.position, cx);
        }
    }
}

/// Records an element's bounds each frame, for turning drags into colours.
fn measure(cell: &Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    let cell = cell.clone();
    canvas(move |b, _, _| cell.set(b), |_, _, _, _| {}).absolute().inset_0()
}

impl<H: WebHost> Render for ColorPicker<H> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bottom = from_oklch(self.lch);
        let top = top_of(bottom, self.spread);
        let current = PALETTE.iter().position(|&(_, b)| b == hex(bottom));
        let (mid_x, mid_y) = (CANVAS.0 / 2., CANVAS.1 / 2.);
        let stop = |[_, c, h]: [f32; 3], size: f32, border: f32, fill: Rgba| {
            let (dx, dy) = on_canvas(c, h);
            ring(size, border).left(px(mid_x + dx - size / 2.)).top(px(mid_y + dy - size / 2.)).bg(fill)
        };
        // Soft hue glows past the ring: a legend for the angle.
        let auras = (0..6).map(|i| {
            let hue = 30. + i as f32 * 60.;
            let (dx, dy) = on_canvas(MAX_C * 1.1, hue);
            let tint = Hsla { a: 0.22, ..Hsla::from(from_oklch([0.75, 0.12, hue])) };
            div()
                .absolute()
                .left(px(mid_x + dx * 1.1 - 25.))
                .top(px(mid_y + dy * 1.1 - 25.))
                .size(px(50.))
                .rounded_full()
                .shadow(glow(tint, 24., 8.))
        });
        let dots = canvas(|_, _, _| {}, |b, _, window, _| {
            const STEP: f32 = 7.2;
            let dot = Hsla::from(surface::border());
            let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
            let mut y = STEP / 2.;
            while y < h {
                let mut x = STEP / 2.;
                while x < w {
                    let at = b.origin + gpui::point(px(x), px(y));
                    window.paint_quad(gpui::fill(Bounds::new(at, gpui::size(px(1.4), px(1.4))), dot));
                    x += STEP;
                }
                y += STEP;
            }
        })
        .absolute()
        .inset_0();
        let field = div()
            .id("theme-field")
            .relative()
            .w(px(CANVAS.0))
            .h(px(CANVAS.1))
            .rounded(r(8.))
            .overflow_hidden()
            .bg(surface::key())
            .cursor_crosshair()
            .children(auras)
            .child(dots)
            .child(measure(&self.canvas))
            .child(stop(to_oklch(top), 23.5, 2., top))
            .child(stop(self.lch, 41., 2.5, bottom))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::start(Drag::Canvas)));

        let chevron = |id: &'static str, name: &'static str, step: usize| {
            div()
                .id(id)
                .cursor_pointer()
                .child(icon(name, 14., surface::muted().into()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let i = current.map_or(0, |i| (i + step) % PALETTE.len());
                    this.swatch(i, cx);
                }))
        };
        let palette = div()
            .mt(px(25.))
            .px(px(10.))
            .flex()
            .items_center()
            .justify_between()
            .child(chevron("theme-prev", "chevron-left", PALETTE.len() - 1))
            .children(PALETTE.iter().enumerate().map(|(i, &(_, b))| {
                div()
                    .id(("theme-swatch", i))
                    .size(px(22.))
                    .rounded_full()
                    .cursor_pointer()
                    .bg(gpui::rgb(b))
                    .border_1()
                    .border_color(gpui::rgba(0x00000014))
                    .when(current == Some(i), |d| {
                        d.border_2().border_color(gpui::white()).shadow(glow(surface::ink().into(), 0., 1.5))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.swatch(i, cx)))
            }))
            .child(chevron("theme-next", "chevron-right", 1));

        let t = ((self.lch[0] - LIGHT.0) / (LIGHT.1 - LIGHT.0)).clamp(0., 1.);
        let wave = canvas(|_, _, _| {}, |b, _, window, _| {
            const CYCLES: f32 = 7.;
            let (w, mid) = (f32::from(b.size.width), f32::from(b.size.height) / 2.);
            let mut p = gpui::PathBuilder::stroke(px(3.));
            p.move_to(b.origin + gpui::point(px(0.), px(mid)));
            let mut x = 1.;
            while x <= w {
                let y = mid + (x / w * CYCLES * std::f32::consts::TAU).sin() * 5.;
                p.line_to(b.origin + gpui::point(px(x), px(y)));
                x += 1.;
            }
            if let Ok(path) = p.build() {
                window.paint_path(path, surface::muted());
            }
        })
        .absolute()
        .top(px(5.))
        .bottom(px(5.))
        .left(px(12.))
        .right(px(12.));
        let slider = div()
            .id("theme-light")
            .relative()
            .w(px(TRACK.0))
            .h(px(TRACK.1))
            .rounded_full()
            .bg(surface::key())
            .cursor_pointer()
            .child(wave)
            .child(measure(&self.slider))
            .child(
                div()
                    .absolute()
                    .left(px(t * (TRACK.0 - THUMB.0)))
                    .top(px((TRACK.1 - THUMB.1) / 2.))
                    .w(px(THUMB.0))
                    .h(px(THUMB.1))
                    .rounded_full()
                    .bg(surface::raised())
                    .shadow(vec![shadow(gpui::rgba(0x2A253A2E).into(), 2., 7.)]),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(Self::start(Drag::Light)));

        // Ticks up to the handle take the colour, so the dial reads at a glance.
        let lit = (self.spread * TICKS as f32).round() as usize;
        let around = |deg: f32, size: f32| {
            let a = deg.to_radians();
            (DIAL / 2. + a.sin() * 34. - size / 2., DIAL / 2. - a.cos() * 34. - size / 2.)
        };
        let ticks = (0..TICKS).map(|i| {
            let (x, y) = around(i as f32 * 360. / TICKS as f32, 3.5);
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .size(px(3.5))
                .rounded_full()
                .bg(if i < lit { Hsla::from(bottom) } else { Hsla::from(surface::border()) })
        });
        let (hx, hy) = around(self.spread * 360., 11.);
        let dial = div()
            .id("theme-spread")
            .relative()
            .size(px(DIAL))
            .cursor_pointer()
            .children(ticks)
            .child(
                div()
                    .absolute()
                    .left(px(22.))
                    .top(px(22.))
                    .size(px(44.))
                    .rounded_full()
                    .bg(surface::raised())
                    .border_1()
                    .border_color(surface::line()),
            )
            .child(ring(11., 0.).left(px(hx)).top(px(hy)).bg(surface::raised()))
            .child(measure(&self.dial))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::start(Drag::Spread)));

        card("theme-picker")
            .w(px(CANVAS.0 + 12.))
            .p(px(6.))
            .gap(px(0.))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if matches!(e.keystroke.key.as_str(), "escape" | "enter") {
                    dismiss(&this.browser, cx);
                }
                cx.stop_propagation();
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| dismiss(&this.browser, cx)))
            .child(field)
            .child(palette)
            .child(div().mt(px(4.)).px(px(10.)).pb(px(6.)).flex().items_center().justify_between().child(slider).child(dial))
            // While dragging, every move in the window steers, even off the card.
            .when(self.drag.is_some(), |d| {
                let this = cx.entity().downgrade();
                d.child(
                    canvas(|_, _, _| {}, move |_, _, window, _| {
                        let up = this.clone();
                        window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                            if phase == gpui::DispatchPhase::Bubble {
                                let _ = this.update(cx, |p, cx| p.drag_to(e.position, cx));
                            }
                        });
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                            if phase == gpui::DispatchPhase::Bubble {
                                let _ = up.update(cx, |p, cx| {
                                    p.drag = None;
                                    cx.notify();
                                });
                            }
                        });
                    })
                    .absolute()
                    .size_0(),
                )
            })
    }
}

struct Emoji {
    glyph: &'static str,
    name: &'static str,
    index: usize,
}

/// Unicode's emoji-test.txt (fully qualified, no skin tones), kept to the
/// emoji macOS names in CoreEmoji's en.lproj/AppleName.strings, with Apple's
/// names: `# Group` lines, then `glyph name` lines.
// ponytail: a snapshot of macOS 27's set; regenerate from those two files when Apple adds emoji.
static EMOJI: LazyLock<Vec<(&'static str, Vec<Emoji>)>> = LazyLock::new(|| {
    let mut groups: Vec<(&'static str, Vec<Emoji>)> = Vec::new();
    let mut index = 0;
    for line in include_str!("emoji.txt").lines() {
        if let Some(group) = line.strip_prefix("# ") {
            groups.push((group, Vec::new()));
        } else if let (Some((glyph, name)), Some((_, list))) = (line.split_once(' '), groups.last_mut()) {
            list.push(Emoji { glyph, name, index });
            index += 1;
        }
    }
    groups
});

const COLS: usize = 9;
const CELL: f32 = 32.;

enum Row {
    Heading(&'static str),
    Cells(Vec<&'static Emoji>),
}

/// The grid: every group under its heading, or for a search, the matches in one run.
fn rows(query: &str) -> Vec<Row> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let all = |list: &'static [Emoji]| list.iter().collect::<Vec<_>>();
    if words.is_empty() {
        return EMOJI
            .iter()
            .flat_map(|(group, list)| {
                std::iter::once(Row::Heading(group)).chain(all(list).chunks(COLS).map(|c| Row::Cells(c.to_vec())).collect::<Vec<_>>())
            })
            .collect();
    }
    let hits: Vec<&'static Emoji> = EMOJI
        .iter()
        .flat_map(|(_, list)| list.iter())
        .filter(|e| {
            let name = e.name.to_lowercase();
            words.iter().all(|w| name.contains(w.as_str()))
        })
        .collect();
    hits.chunks(COLS).map(|c| Row::Cells(c.to_vec())).collect()
}

pub(crate) struct EmojiPicker<H: WebHost> {
    browser: WeakEntity<Browser<H>>,
    space: SpaceId,
    current: String,
    query: TextField,
    rows: Vec<Row>,
    hovered: Option<&'static Emoji>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl<H: WebHost> EventEmitter<DismissEvent> for EmojiPicker<H> {}

impl<H: WebHost> EmojiPicker<H> {
    fn new(browser: WeakEntity<Browser<H>>, space: SpaceId, current: String, cx: &mut Context<Self>) -> Self {
        Self {
            browser,
            space,
            current,
            query: TextField::default(),
            rows: rows(""),
            hovered: None,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    fn pick(&mut self, glyph: &'static str, cx: &mut Context<Self>) {
        let space = self.space;
        if let Some(b) = self.browser.upgrade() {
            b.update(cx, |b, cx| {
                b.set_icon_of(space, glyph);
                cx.notify();
            });
        }
        dismiss(&self.browser, cx);
    }

    fn render_row(&self, i: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        match &self.rows[i] {
            Row::Heading(group) => div()
                .h(px(CELL))
                .px(px(6.))
                .pb(px(4.))
                .flex()
                .items_end()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(surface::muted())
                .child(*group)
                .into_any_element(),
            Row::Cells(list) => div()
                .h(px(CELL))
                .flex()
                .children(list.iter().map(|&e| {
                    div()
                        .id(("emoji", e.index))
                        .size(px(CELL))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(r(8.))
                        .text_size(px(20.))
                        .cursor_pointer()
                        .when(e.glyph == self.current, |d| d.bg(surface::key()))
                        .hover(|s| s.bg(surface::key()))
                        .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                            if *on {
                                this.hovered = Some(e);
                            } else if this.hovered.is_some_and(|h| h.index == e.index) {
                                this.hovered = None;
                            }
                            cx.notify();
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| this.pick(e.glyph, cx)))
                        .child(e.glyph)
                }))
                .into_any_element(),
        }
    }
}

impl<H: WebHost> Render for EmojiPicker<H> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let search = div()
            .h(px(30.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(r(8.))
            .bg(surface::key())
            .child(self.query.render("Search emoji", surface::ink().into(), surface::muted().into(), gpui::rgba(0x6C9CFF55).into()));
        let grid: gpui::AnyElement = if self.rows.is_empty() {
            div()
                .h(px(CELL * 9.))
                .flex()
                .items_center()
                .justify_center()
                .text_color(surface::muted())
                .child("No emoji found")
                .into_any_element()
        } else {
            gpui::uniform_list(
                "emoji-grid",
                self.rows.len(),
                cx.processor(|this, range: Range<usize>, _, cx| range.map(|i| this.render_row(i, cx)).collect::<Vec<_>>()),
            )
            .h(px(CELL * 9.))
            .track_scroll(&self.scroll)
            .into_any_element()
        };
        let (glyph, name) = match self.hovered {
            Some(e) => (e.glyph.to_string(), e.name.to_string()),
            None => (self.current.clone(), "Space icon".to_string()),
        };
        let footer = div()
            .h(px(24.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().text_size(px(18.)).child(SharedString::from(glyph)))
            .child(div().truncate().text_color(surface::muted()).child(SharedString::from(name)));
        card("emoji-picker")
            .w(px(COLS as f32 * CELL + 24.))
            .gap(px(8.))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                match this.query.handle(&e.keystroke, cx) {
                    Key::Edited => {
                        this.rows = rows(&this.query.text);
                        this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    }
                    Key::Submit => {
                        let first = this.rows.iter().find_map(|r| match r {
                            Row::Cells(list) => list.first().map(|e| e.glyph),
                            Row::Heading(_) => None,
                        });
                        if let Some(glyph) = first {
                            this.pick(glyph, cx);
                        }
                    }
                    Key::Cancel => dismiss(&this.browser, cx),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| dismiss(&this.browser, cx)))
            .child(search)
            .child(grid)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_come_grouped_and_search_by_name() {
        assert_eq!(EMOJI.len(), 9);
        assert!(EMOJI.iter().map(|(_, l)| l.len()).sum::<usize>() > 1800);
        assert!(matches!(rows("").first(), Some(Row::Heading("Smileys & Emotion"))));
        let found = |q: &str| -> Vec<&str> {
            rows(q).into_iter().flat_map(|r| match r {
                Row::Cells(l) => l.into_iter().map(|e| e.glyph).collect(),
                Row::Heading(_) => vec![],
            }).collect()
        };
        assert!(found("Sheep").contains(&"🐑"));
        assert!(found("red heart").contains(&"❤️"));
        assert!(found("zzqq").is_empty());
    }

    #[test]
    fn a_swatch_survives_the_round_trip_through_the_canvas() {
        for (_, bottom) in PALETTE {
            let [l, c, h] = to_oklch(gpui::rgb(bottom));
            let (dx, dy) = on_canvas(c, h);
            let (c, h) = from_canvas(dx, dy);
            let back = hex(from_oklch([l, c, h]));
            let d = |s: u32| ((back >> s & 0xFF) as i32 - (bottom >> s & 0xFF) as i32).abs();
            assert!(d(16) <= 1 && d(8) <= 1 && d(0) <= 1, "{bottom:06X} came back {back:06X}");
            for spread in [0., 0.25, 0.4, 0.9] {
                let top = gpui::rgb(hex(top_of(gpui::rgb(bottom), spread)));
                let back = spread_of(top, gpui::rgb(bottom));
                assert!((back - spread).abs() < 0.02, "{bottom:06X} at {spread} read as {back}");
            }
        }
    }
}

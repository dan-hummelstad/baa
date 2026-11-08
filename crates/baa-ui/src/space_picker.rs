//! Arc's Space editors: the theme colour (a hue × vividness field, a lightness
//! slider, the nine swatches) and the icon (every emoji macOS has a name for).
//! Each is its own view so the browser window and the settings window can both
//! hang one; they change the Space through the browser, live.

use std::{cell::Cell, ops::Range, rc::Rc, sync::LazyLock};

use baa_model::SpaceId;
use baa_web::WebHost;
use gpui::{
    anchored, canvas, deferred, div, linear_color_stop, linear_gradient, prelude::*, px, AnyView, Bounds, Context,
    DismissEvent, Entity, EventEmitter, FocusHandle, Hsla, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Rgba, ScrollStrategy, SharedString, Stateful, Subscription, UniformListScrollHandle,
    WeakEntity, Window,
};

use crate::{
    browser::{drop_in, shadow, Browser, PALETTE},
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
            let p = cx.new(|cx| ColorPicker::new(browser, space.id, bottom, cx));
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

const FIELD_W: f32 = 216.;
const FIELD_H: f32 = 132.;
/// Vividness at the field's top edge: about what sRGB holds at mid lightness.
const MAX_C: f32 = 0.16;
/// The slider's ends, dark to light (OKLCH L).
const LIGHT: (f32, f32) = (0.30, 0.95);
const KNOB: f32 = 18.;

#[derive(Clone, Copy)]
enum Drag {
    Field,
    Light,
}

pub(crate) struct ColorPicker<H: WebHost> {
    browser: WeakEntity<Browser<H>>,
    space: SpaceId,
    /// OKLCH of the Space's bottom stop, the colour the dot stands for.
    lch: [f32; 3],
    drag: Option<Drag>,
    field: Rc<Cell<Bounds<Pixels>>>,
    slider: Rc<Cell<Bounds<Pixels>>>,
    focus: FocusHandle,
}

impl<H: WebHost> EventEmitter<DismissEvent> for ColorPicker<H> {}

/// The gradient's top stop: the bottom one 40% toward white, as the swatches are.
fn top_of(bottom: Rgba) -> Rgba {
    let up = |v: f32| v + (1. - v) * 0.4;
    Rgba { r: up(bottom.r), g: up(bottom.g), b: up(bottom.b), a: 1. }
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

impl<H: WebHost> ColorPicker<H> {
    fn new(browser: WeakEntity<Browser<H>>, space: SpaceId, bottom: u32, cx: &mut Context<Self>) -> Self {
        Self {
            browser,
            space,
            lch: to_oklch(gpui::rgb(bottom)),
            drag: None,
            field: Rc::default(),
            slider: Rc::default(),
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

    fn drag_to(&mut self, at: Point<Pixels>, cx: &mut Context<Self>) {
        match self.drag {
            Some(Drag::Field) => {
                let (x, y) = fraction(at, self.field.get());
                self.lch[1] = (1. - y) * MAX_C;
                self.lch[2] = x * 360.;
            }
            Some(Drag::Light) => self.lch[0] = LIGHT.0 + fraction(at, self.slider.get()).0 * (LIGHT.1 - LIGHT.0),
            None => return,
        }
        let bottom = from_oklch(self.lch);
        self.set(hex(top_of(bottom)), hex(bottom), cx);
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
        let [l, c, h] = self.lch;
        let current: Option<u32> = self
            .browser
            .upgrade()
            .and_then(|b| b.read(cx).session.space(self.space).and_then(|s| s.accent_to));
        const STEPS: usize = 24;
        let hue = |i: usize| Hsla::from(from_oklch([l, MAX_C, i as f32 * 360. / STEPS as f32]));
        let grey = Hsla::from(from_oklch([l, 0., 0.]));
        let field = div()
            .id("theme-field")
            .relative()
            .w(px(FIELD_W))
            .h(px(FIELD_H))
            .flex()
            .cursor_crosshair()
            .children((0..STEPS).map(|i| {
                div()
                    .flex_1()
                    .h_full()
                    .bg(linear_gradient(90., linear_color_stop(hue(i), 0.), linear_color_stop(hue(i + 1), 1.)))
                    .when(i == 0, |d| d.rounded_l(r(10.)))
                    .when(i == STEPS - 1, |d| d.rounded_r(r(10.)))
            }))
            // Vivid at the top, fading to the grey of the same lightness at the bottom.
            .child(div().absolute().inset_0().rounded(r(10.)).bg(linear_gradient(
                180.,
                linear_color_stop(Hsla { a: 0., ..grey }, 0.),
                linear_color_stop(grey, 1.),
            )))
            .child(measure(&self.field))
            .child(
                div()
                    .absolute()
                    .left(px(h / 360. * FIELD_W - KNOB / 2.))
                    .top(px((1. - (c / MAX_C).min(1.)) * FIELD_H - KNOB / 2.))
                    .size(px(KNOB))
                    .rounded_full()
                    .border_2()
                    .border_color(gpui::white())
                    .bg(from_oklch(self.lch))
                    .shadow(vec![shadow(gpui::rgba(0x0000004D).into(), 1., 4.)]),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(Self::start(Drag::Field)));
        let t = ((l - LIGHT.0) / (LIGHT.1 - LIGHT.0)).clamp(0., 1.);
        let slider = div()
            .id("theme-light")
            .relative()
            .w(px(FIELD_W))
            .h(px(12.))
            .rounded_full()
            .bg(linear_gradient(
                90.,
                linear_color_stop(from_oklch([LIGHT.0, c, h]), 0.),
                linear_color_stop(from_oklch([LIGHT.1, c, h]), 1.),
            ))
            .child(measure(&self.slider))
            .child(
                div()
                    .absolute()
                    .top(px(-3.))
                    .left(px(t * FIELD_W - KNOB / 2.))
                    .size(px(KNOB))
                    .rounded_full()
                    .border_2()
                    .border_color(gpui::white())
                    .bg(from_oklch(self.lch))
                    .shadow(vec![shadow(gpui::rgba(0x0000004D).into(), 1., 4.)]),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(Self::start(Drag::Light)));
        let swatches = div().flex().justify_between().children(PALETTE.iter().enumerate().map(|(i, &(top, bottom))| {
            let on = current == Some(bottom);
            div()
                .id(("theme-swatch", i))
                .size(px(22.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .border_2()
                .border_color(if on { Hsla::from(surface::ink()) } else { gpui::transparent_black() })
                .cursor_pointer()
                .child(div().size(px(14.)).rounded_full().bg(gpui::rgb(bottom)).border_1().border_color(surface::border()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.lch = to_oklch(gpui::rgb(bottom));
                    this.set(top, bottom, cx);
                }))
        }));
        card("theme-picker")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if matches!(e.keystroke.key.as_str(), "escape" | "enter") {
                    dismiss(&this.browser, cx);
                }
                cx.stop_propagation();
            }))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| dismiss(&this.browser, cx)))
            .child(field)
            .child(slider)
            .child(swatches)
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
    fn a_swatch_survives_the_round_trip_through_the_field() {
        for (_, bottom) in PALETTE {
            let back = hex(from_oklch(to_oklch(gpui::rgb(bottom))));
            let d = |s: u32| ((back >> s & 0xFF) as i32 - (bottom >> s & 0xFF) as i32).abs();
            assert!(d(16) <= 1 && d(8) <= 1 && d(0) <= 1, "{bottom:06X} came back {back:06X}");
        }
    }
}

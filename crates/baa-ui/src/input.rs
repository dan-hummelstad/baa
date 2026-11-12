//! Single-line text field for every text box in the UI. gpui ships no text
//! input, so keys are applied by hand.
// ponytail: no IME or undo here. Upgrade to an `EntityInputHandler` when CJK input matters.

use std::{cell::Cell, ops::Range, rc::Rc, time::Duration};

use gpui::{
    canvas, div, fill, prelude::*, pulsating_between, px, size, Animation, AnimationExt, App, Bounds,
    DispatchPhase, HighlightStyle, Hsla, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, SharedString, StyledText, TextLayout, Window,
};

#[derive(Debug, PartialEq)]
pub enum Key {
    Edited,
    Submit,
    Cancel,
    Up,
    Down,
    /// Tab, or → at the end: take the suggestion.
    Complete,
    Ignored,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Sel {
    /// The caret and the selection's other end, in bytes from the end of `len` bytes of text.
    caret: usize,
    anchor: usize,
    len: usize,
    /// How far a long single line is shifted left to keep the caret in view.
    scroll: Pixels,
    dragging: bool,
}

/// (caret, anchor) as byte indices. Text its owner replaced puts the caret at the end.
fn ends(text: &str, s: Sel) -> (usize, usize) {
    let len = text.len();
    let at = |back: usize| {
        let i = len.saturating_sub(back);
        if text.is_char_boundary(i) { i } else { len }
    };
    if s.len == len { (at(s.caret), at(s.anchor)) } else { (len, len) }
}

fn put(state: &Cell<Sel>, len: usize, caret: usize, anchor: usize) {
    state.set(Sel { caret: len - caret, anchor: len - anchor, len, ..state.get() });
}

#[derive(Default)]
pub struct TextField {
    pub text: String,
    /// Shared with the drawn field, so a click or drag moves it whoever owns the field.
    pub(crate) sel: Rc<Cell<Sel>>,
    /// Draws bullets for the text (passwords).
    pub masked: bool,
}

impl TextField {
    /// Prefilled text starts selected, so the first keystroke replaces it, like an omnibox.
    pub fn selected(text: String) -> Self {
        let f = Self { text, ..Default::default() };
        f.set(f.text.len(), 0);
        f
    }

    fn ends(&self) -> (usize, usize) {
        ends(&self.text, self.sel.get())
    }

    fn set(&self, caret: usize, anchor: usize) {
        put(&self.sel, self.text.len(), caret, anchor);
    }

    fn replace(&mut self, r: Range<usize>, with: &str) {
        self.text.replace_range(r.clone(), with);
        let at = r.start + with.len();
        self.set(at, at);
    }

    pub fn handle(&mut self, ks: &Keystroke, cx: &App) -> Key {
        let m = &ks.modifiers;
        let (caret, anchor) = self.ends();
        let sel = caret.min(anchor)..caret.max(anchor);
        let (s, len, k) = (self.text.as_str(), self.text.len(), ks.key.as_str());
        match k {
            "enter" => Key::Submit,
            "escape" => Key::Cancel,
            "up" => Key::Up,
            "down" => Key::Down,
            "tab" if !m.control && !m.platform && !m.alt => Key::Complete,
            // → at the end takes the suggestion, like an omnibox.
            "right" if !m.control && !m.platform && !m.alt && !m.shift && sel.is_empty() && caret == len => {
                Key::Complete
            }
            "left" | "right" | "home" | "end" => {
                let left = matches!(k, "left" | "home");
                let arrow = matches!(k, "left" | "right");
                // A plain arrow collapses a selection to its edge; ⌘ jumps to the ends, ⌥ by word.
                let to = if arrow && !m.shift && !m.platform && !m.alt && !sel.is_empty() {
                    if left { sel.start } else { sel.end }
                } else if m.platform || !arrow {
                    if left { 0 } else { len }
                } else if m.alt {
                    if left { word_left(s, caret) } else { word_right(s, caret) }
                } else if left {
                    prev(s, caret)
                } else {
                    next(s, caret)
                };
                self.set(to, if m.shift { anchor } else { to });
                Key::Edited
            }
            "backspace" | "delete" => {
                let r = if !sel.is_empty() {
                    sel
                } else if k == "backspace" {
                    (if m.secondary() { 0 } else if m.alt { word_left(s, caret) } else { prev(s, caret) })..caret
                } else {
                    caret..if m.alt { word_right(s, caret) } else { next(s, caret) }
                };
                self.replace(r, "");
                Key::Edited
            }
            "a" if m.secondary() => {
                self.set(len, 0);
                Key::Edited
            }
            "c" | "x" if m.secondary() && !sel.is_empty() && !self.masked => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(s[sel.clone()].to_string()));
                if k == "c" {
                    return Key::Ignored;
                }
                self.replace(sel, "");
                Key::Edited
            }
            "v" if m.secondary() => {
                let Some(paste) = cx.read_from_clipboard().and_then(|c| c.text()) else {
                    return Key::Ignored;
                };
                self.replace(sel, paste.lines().next().unwrap_or_default());
                Key::Edited
            }
            _ if m.platform || m.control => Key::Ignored,
            _ => match ks.key_char.clone() {
                Some(c) => {
                    self.replace(sel, &c);
                    Key::Edited
                }
                None => Key::Ignored,
            },
        }
    }

    /// `focused`: only the field that gets the keys shows its (pulsing) caret.
    pub fn render(
        &self,
        focused: bool,
        placeholder: &'static str,
        text: Hsla,
        dim: Hsla,
        selection: Hsla,
    ) -> impl IntoElement {
        self.render_with(false, focused, placeholder, text, dim, selection)
    }

    /// Long text flows onto more lines instead of being cut at the edge.
    pub fn render_wrapped(
        &self,
        focused: bool,
        placeholder: &'static str,
        text: Hsla,
        dim: Hsla,
        selection: Hsla,
    ) -> impl IntoElement {
        self.render_with(true, focused, placeholder, text, dim, selection)
    }

    fn render_with(
        &self,
        wrap: bool,
        focused: bool,
        placeholder: &'static str,
        text: Hsla,
        dim: Hsla,
        selection: Hsla,
    ) -> impl IntoElement {
        let empty = self.text.is_empty();
        let (caret, anchor) = self.ends();
        // A bullet is 3 bytes in the drawn string.
        let shown_ix = |i: usize| if self.masked { self.text[..i].chars().count() * 3 } else { i };
        let shown: SharedString = if empty {
            placeholder.into()
        } else if self.masked {
            "\u{2022}".repeat(self.text.chars().count()).into()
        } else {
            self.text.clone().into()
        };
        let (c, a) = (shown_ix(caret), shown_ix(anchor));
        let pick = HighlightStyle { background_color: Some(selection), ..Default::default() };
        let styled = StyledText::new(shown).with_highlights((c != a).then(|| (c.min(a)..c.max(a), pick)));
        let hit = Hit {
            layout: styled.layout().clone(),
            state: self.sel.clone(),
            src: self.text.clone().into(),
            masked: self.masked,
        };
        let on_down = hit.clone();
        let layer = div().absolute().inset_0();
        let layer = if focused {
            layer.with_animation("caret", pulse(), move |d, t| d.child(hit.overlay(c, wrap, Some((text, t))))).into_any_element()
        } else {
            layer.child(hit.overlay(c, wrap, None)).into_any_element()
        };
        div()
            .relative()
            .flex()
            .items_center()
            // The whole box, not just the glyphs, takes the click.
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .line_height(gpui::relative(1.25))
            .text_color(if empty { dim } else { text })
            .on_mouse_down(MouseButton::Left, move |e: &MouseDownEvent, window, cx| {
                on_down.click(e, window);
                // Focused, the press is the field's: a row around it must not toggle or start a drag.
                // Unfocused, it goes on up so the owner can focus the field.
                if focused {
                    cx.stop_propagation();
                }
            })
            .child(if wrap {
                div().flex_1().min_w_0().child(styled)
            } else {
                div().flex_shrink_0().whitespace_nowrap().ml(-self.sel.get().scroll).child(styled)
            })
            .child(layer)
    }
}

/// What a click, a drag and the caret need from the drawn text.
#[derive(Clone)]
struct Hit {
    layout: TextLayout,
    state: Rc<Cell<Sel>>,
    src: SharedString,
    masked: bool,
}

impl Hit {
    /// The text's byte index nearest a window position.
    fn index(&self, pos: Point<Pixels>) -> usize {
        let Some(line) = self.layout.line_layouts().into_iter().next() else { return 0 };
        let lh = self.layout.line_height();
        let mut p = pos - self.layout.bounds().origin;
        p.y = p.y.max(px(0.)).min(line.size(lh).height - px(1.));
        let i = line.closest_index_for_position(p, lh).unwrap_or_else(|i| i);
        if self.masked {
            self.src.char_indices().nth(i / 3).map_or(self.src.len(), |(b, _)| b)
        } else {
            i.min(self.src.len())
        }
    }

    /// Click places the caret, shift-click extends, double picks a word, triple all.
    fn click(&self, e: &MouseDownEvent, window: &mut Window) {
        let (len, i) = (self.src.len(), self.index(e.position));
        let (_, anchor) = ends(&self.src, self.state.get());
        let (caret, anchor) = match e.click_count {
            0 | 1 if e.modifiers.shift => (i, anchor),
            0 | 1 => (i, i),
            2 => word_at(&self.src, i),
            _ => (len, 0),
        };
        put(&self.state, len, caret, anchor);
        self.state.set(Sel { dragging: e.click_count <= 1, ..self.state.get() });
        window.refresh();
    }

    /// Paints the caret (`color`: base colour and pulse), keeps it in view, and tracks drags.
    fn overlay(&self, caret: usize, wrap: bool, color: Option<(Hsla, f32)>) -> impl IntoElement {
        let hit = self.clone();
        canvas(|_, _, _| {}, move |bounds: Bounds<Pixels>, _, window, cx| {
            let layout = &hit.layout;
            let mut p = layout.position_for_index(caret);
            if let (false, Some(p)) = (wrap, p.as_mut()) {
                // ponytail: the text follows the caret a frame late; the caret itself is on time.
                let s = hit.state.get();
                let right = bounds.right() - px(2.);
                let shift = if p.x > right { p.x - right } else { (p.x - bounds.left()).min(px(0.)) };
                let mut scroll = s.scroll + shift;
                // Once the text fits again, give back the room at its end.
                let room = right - (layout.bounds().right() - shift);
                if room > px(0.) {
                    scroll -= room.min(scroll);
                }
                let scroll = scroll.max(px(0.));
                if scroll != s.scroll {
                    p.x -= scroll - s.scroll;
                    hit.state.set(Sel { scroll, ..s });
                    window.refresh();
                }
            }
            if let (Some((base, t)), Some(p)) = (color, p) {
                let a = if cx.reduce_motion() { 1. } else { t };
                let lh = layout.line_height();
                window.paint_quad(fill(Bounds::new(p, size(px(1.5), lh)), Hsla { a: base.a * a, ..base }));
            }
            let drag = hit.clone();
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, _| {
                let s = drag.state.get();
                if phase != DispatchPhase::Bubble || !s.dragging {
                    return;
                }
                if e.pressed_button != Some(MouseButton::Left) {
                    return drag.state.set(Sel { dragging: false, ..s });
                }
                let (_, anchor) = ends(&drag.src, s);
                put(&drag.state, drag.src.len(), drag.index(e.position), anchor);
                window.refresh();
            });
            let up = hit.state.clone();
            window.on_mouse_event(move |_: &MouseUpEvent, _, _, _| up.set(Sel { dragging: false, ..up.get() }));
        })
        .size_full()
    }
}

fn prev(s: &str, i: usize) -> usize {
    s[..i].char_indices().next_back().map_or(0, |(j, _)| j)
}

fn next(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(i, |c| i + c.len_utf8())
}

/// Words are letters and digits, so ⌥← in a URL stops at each `/`, `.` and `-`.
fn word_left(s: &str, i: usize) -> usize {
    s[..i].trim_end_matches(|c: char| !c.is_alphanumeric()).trim_end_matches(char::is_alphanumeric).len()
}

fn word_right(s: &str, i: usize) -> usize {
    let rest = s[i..].trim_start_matches(|c: char| !c.is_alphanumeric()).trim_start_matches(char::is_alphanumeric);
    s.len() - rest.len()
}

/// The word around `i`, as (end, start): a double-click leaves the caret at its end.
fn word_at(s: &str, i: usize) -> (usize, usize) {
    let start = s[..i].trim_end_matches(char::is_alphanumeric).len();
    (s.len() - s[i..].trim_start_matches(char::is_alphanumeric).len(), start)
}

/// A slow breath, so a focused field reads as focused; under reduce motion it holds steady.
fn pulse() -> Animation {
    Animation::new(Duration::from_millis(1000)).repeat().with_easing(pulsating_between(0.15, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(field: &mut TextField, s: &str, cx: &App) -> Key {
        field.handle(&Keystroke::parse(s).unwrap(), cx)
    }

    fn typed(field: &mut TextField, c: &str, cx: &App) {
        let mut ks = Keystroke::parse("b").unwrap();
        ks.key_char = Some(c.into());
        field.handle(&ks, cx);
    }

    #[gpui::test]
    fn typing_replaces_selection_then_appends(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut f = TextField::selected("https://old".into());
            typed(&mut f, "a", cx);
            typed(&mut f, "a", cx);
            assert_eq!(f.text, "aa");
            assert_eq!(key(&mut f, "backspace", cx), Key::Edited);
            assert_eq!(f.text, "a");
            assert_eq!(key(&mut f, "enter", cx), Key::Submit);
            assert_eq!(key(&mut f, "cmd-w", cx), Key::Ignored);
        });
    }

    #[gpui::test]
    fn arrows_collapse_the_selection_instead_of_completing(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut f = TextField::selected("https://old".into());
            assert_eq!(key(&mut f, "right", cx), Key::Edited);
            assert_eq!(f.ends(), (11, 11));
            assert_eq!(key(&mut f, "right", cx), Key::Complete);
            assert_eq!(key(&mut f, "cmd-a", cx), Key::Edited);
            assert_eq!(key(&mut f, "left", cx), Key::Edited);
            assert_eq!(f.ends(), (0, 0), "← collapses a selection to its start");
            assert_eq!(f.text, "https://old");
        });
    }

    #[gpui::test]
    fn the_caret_moves_and_edits_in_the_middle(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut f = TextField { text: "https://a.com/x".into(), ..Default::default() };
            assert_eq!(f.ends().0, f.text.len(), "set text leaves the caret at the end");
            key(&mut f, "alt-left", cx);
            key(&mut f, "alt-left", cx);
            assert_eq!(f.ends().0, "https://a.".len());
            typed(&mut f, "é", cx);
            assert_eq!(f.text, "https://a.écom/x");
            key(&mut f, "left", cx);
            key(&mut f, "delete", cx);
            assert_eq!(f.text, "https://a.com/x", "forward delete takes the multibyte char");
            key(&mut f, "backspace", cx);
            assert_eq!(f.text, "https://acom/x");
            key(&mut f, "cmd-backspace", cx);
            assert_eq!(f.text, "com/x");
            assert_eq!(key(&mut f, "right", cx), Key::Edited, "→ moves before the end");
            key(&mut f, "end", cx);
            assert_eq!(key(&mut f, "right", cx), Key::Complete, "→ at the end completes");
            key(&mut f, "home", cx);
            key(&mut f, "alt-delete", cx);
            assert_eq!(f.text, "/x");
        });
    }

    #[gpui::test]
    fn shift_extends_and_edits_replace_the_selection(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut f = TextField { text: "https://a.com/path".into(), ..Default::default() };
            key(&mut f, "shift-alt-left", cx);
            key(&mut f, "shift-left", cx);
            assert_eq!(f.ends(), ("https://a.com".len(), f.text.len()), "anchor stays put");
            key(&mut f, "shift-right", cx);
            typed(&mut f, "x", cx);
            assert_eq!(f.text, "https://a.com/x");
            key(&mut f, "shift-cmd-left", cx);
            assert_eq!(key(&mut f, "right", cx), Key::Edited, "→ collapses a selection, never completes");
            assert_eq!(f.ends(), (f.text.len(), f.text.len()));
            key(&mut f, "shift-home", cx);
            key(&mut f, "backspace", cx);
            assert_eq!(f.text, "");
        });
    }

    #[test]
    fn a_double_click_picks_the_word_around_it() {
        let s = "https://example.com/a";
        assert_eq!(word_at(s, 10), (15, 8));
        assert_eq!(word_at(s, 8), (15, 8));
        assert_eq!(word_at(s, 7), (7, 7), "between separators: nothing to pick");
    }
}

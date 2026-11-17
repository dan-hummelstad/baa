//! What macOS's input methods type into a `TextField`: Chinese, Japanese and Korean
//! composition, dead keys, the press-and-hold accents, the emoji picker, dictation.
//! The platform counts in UTF-16; the field keeps bytes.

use std::ops::Range;

use gpui::{
    canvas, px, size, App, Bounds, Context, FocusHandle, InputHandler, InteractiveElement, KeyDownEvent, Keystroke,
    ParentElement, Pixels, Point, Styled, UTF16Selection, WeakEntity, Window,
};

use super::{Edit, TextField};
use crate::actions::{Redo, Undo};

pub(crate) type FieldFn<V> = fn(&mut V) -> Option<&mut TextField>;
pub(crate) type KeyFn<V> = fn(&mut V, &KeyDownEvent, &mut Window, &mut Context<V>);

fn to16(s: &str, i: usize) -> usize {
    s[..i].encode_utf16().count()
}

fn from16(s: &str, i: usize) -> usize {
    let mut n = 0;
    for (b, c) in s.char_indices() {
        if n >= i {
            return b;
        }
        n += c.len_utf16();
    }
    s.len()
}

/// The fields are one line: a pasted or dictated newline ends the text, and the Tab
/// or Return a key also reports as text never types.
fn one_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().chars().filter(|c| !c.is_control()).collect()
}

impl TextField {
    fn range8(&self, r: Range<usize>) -> Range<usize> {
        let (a, b) = (from16(&self.text, r.start), from16(&self.text, r.end));
        a.min(b)..a.max(b)
    }

    fn target(&self, r16: Option<Range<usize>>) -> Range<usize> {
        r16.map(|r| self.range8(r)).or_else(|| self.marked()).unwrap_or_else(|| self.selection())
    }

    pub(crate) fn text_for_range(&self, r16: Range<usize>, adjusted: &mut Option<Range<usize>>) -> Option<String> {
        if self.masked {
            return None;
        }
        let r = self.range8(r16);
        *adjusted = Some(to16(&self.text, r.start)..to16(&self.text, r.end));
        Some(self.text[r].to_string())
    }

    pub(crate) fn selected_text_range(&self) -> UTF16Selection {
        let (caret, anchor) = self.ends();
        let r = self.selection();
        UTF16Selection { range: to16(&self.text, r.start)..to16(&self.text, r.end), reversed: caret < anchor }
    }

    pub(crate) fn marked_text_range(&self) -> Option<Range<usize>> {
        self.marked().map(|r| to16(&self.text, r.start)..to16(&self.text, r.end))
    }

    pub(crate) fn unmark_text(&mut self) {
        self.marked = None;
    }

    pub(crate) fn replace_text_in_range(&mut self, r16: Option<Range<usize>>, text: &str) {
        let line = one_line(text);
        let composing = self.marked().is_some();
        if line.is_empty() && (!composing || !text.is_empty()) {
            return;
        }
        let r = self.target(r16);
        if composing {
            self.end_composition(r, &line);
        } else {
            self.marked = None;
            self.edit(Edit::Typing, r, &line);
        }
    }

    /// Composition text, shown underlined until committed; `sel16` is within `text`.
    /// A masked field ignores it and takes only the commit, so nothing secret is ever
    /// marked and an interim string (a dead key's `¨`, dictation's guesses) never types.
    pub(crate) fn replace_and_mark_text_in_range(&mut self, r16: Option<Range<usize>>, text: &str, sel16: Option<Range<usize>>) {
        if self.masked {
            return;
        }
        let text = one_line(text);
        let r = self.target(r16);
        if self.marked().is_none() {
            self.checkpoint();
        }
        if text.is_empty() {
            return self.end_composition(r, "");
        }
        self.text.replace_range(r.clone(), &text);
        self.marked = Some(r.start..r.start + text.len());
        let (a, b) = sel16.map_or((text.len(), text.len()), |s| (from16(&text, s.start), from16(&text, s.end)));
        self.set(r.start + b, r.start + a);
        self.history.run = None;
    }

    /// From the last painted text, which an edit the input method just made has not
    /// reached yet: then the end of the text stands in.
    pub(crate) fn bounds_for_range(&self, r16: Range<usize>) -> Option<Bounds<Pixels>> {
        let layout = self.drawn.borrow().clone()?;
        let r = self.range8(r16);
        let start = layout.position_for_index(self.shown(r.start)).or_else(|| layout.position_for_index(layout.len()))?;
        let end = layout.position_for_index(self.shown(r.end)).filter(|p| p.y == start.y).unwrap_or(start);
        Some(Bounds::new(start, size((end.x - start.x).max(px(0.)), layout.line_height())))
    }

    pub(crate) fn character_index_for_point(&self, p: Point<Pixels>) -> Option<usize> {
        if self.masked {
            return None;
        }
        let layout = self.drawn.borrow().clone()?;
        let mut i = layout.index_for_position(p).ok()?.min(self.text.len());
        while !self.text.is_char_boundary(i) {
            i -= 1;
        }
        Some(to16(&self.text, i))
    }
}

/// The key the owner's handler gets after the input method changed the text, which
/// `TextField::handle` answers with `Key::Edited`. A `-` in a key name never comes from
/// a keyboard: `Keystroke::parse` reads it as a modifier separator.
pub(super) const EDITED: &str = "ime-edited";

fn edited() -> KeyDownEvent {
    let keystroke = Keystroke { key: EDITED.into(), ..Default::default() };
    KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }
}

struct Ime<V: 'static> {
    owner: WeakEntity<V>,
    field: FieldFn<V>,
    on_key: KeyFn<V>,
}

impl<V: 'static> Ime<V> {
    fn read<R>(&self, cx: &mut App, f: impl FnOnce(&mut TextField) -> R) -> Option<R> {
        let field = self.field;
        self.owner.update(cx, |v, _| field(v).map(f)).ok().flatten()
    }

    fn write(&self, window: &mut Window, cx: &mut App, f: impl FnOnce(&mut TextField)) {
        let (field, on_key) = (self.field, self.on_key);
        let _ = self.owner.update(cx, |v, cx| {
            let Some(t) = field(v) else { return };
            let before = t.text.clone();
            f(t);
            if t.text != before {
                on_key(v, &edited(), window, cx);
            }
            cx.notify();
        });
    }
}

impl<V: 'static> InputHandler for Ime<V> {
    fn selected_text_range(&mut self, _: bool, _: &mut Window, cx: &mut App) -> Option<UTF16Selection> {
        self.read(cx, |f| f.selected_text_range())
    }

    fn marked_text_range(&mut self, _: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.read(cx, |f| f.marked_text_range()).flatten()
    }

    fn text_for_range(&mut self, r: Range<usize>, adjusted: &mut Option<Range<usize>>, _: &mut Window, cx: &mut App) -> Option<String> {
        self.read(cx, |f| f.text_for_range(r, adjusted)).flatten()
    }

    fn replace_text_in_range(&mut self, r: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut App) {
        self.write(window, cx, |f| f.replace_text_in_range(r, text));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        r: Option<Range<usize>>,
        text: &str,
        sel: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.write(window, cx, |f| f.replace_and_mark_text_in_range(r, text, sel));
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        self.write(window, cx, TextField::unmark_text);
    }

    fn bounds_for_range(&mut self, r: Range<usize>, _: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        self.read(cx, |f| f.bounds_for_range(r)).flatten()
    }

    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, cx: &mut App) -> Option<usize> {
        self.read(cx, |f| f.character_index_for_point(p)).flatten()
    }

    fn accepts_text_input(&mut self, _: &mut Window, cx: &mut App) -> bool {
        self.read(cx, |f| !f.masked).unwrap_or(false)
    }

    fn prefers_ime_for_printable_keys(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.accepts_text_input(window, cx)
    }
}

/// While `focus` is focused, the input method types into the field `field` picks and
/// the Edit menu's Undo and Redo reach it. `field` must pick the field `on_key` routes
/// keys to, and `on_key` hears of every edit the input method makes (`edited`).
/// Without a field (`has_field` false, as under an alert) Undo and Redo stay disabled.
pub(crate) fn wire<V: 'static, E: ParentElement + InteractiveElement>(
    el: E,
    focus: &FocusHandle,
    has_field: bool,
    window: &Window,
    cx: &mut Context<V>,
    field: FieldFn<V>,
    on_key: KeyFn<V>,
) -> E {
    if !focus.is_focused(window) {
        return el;
    }
    let ime = Ime { owner: cx.entity().downgrade(), field, on_key };
    let focus = focus.clone();
    let el = el.child(canvas(|_, _, _| {}, move |_, _, window, cx| window.handle_input(&focus, ime, cx)).absolute().size_0());
    if !has_field {
        return el;
    }
    let step = move |v: &mut V, back: bool, window: &mut Window, cx: &mut Context<V>| {
        if field(v).is_some_and(|f| f.step(back)) {
            on_key(v, &edited(), window, cx);
            cx.notify();
        }
    };
    el.on_action(cx.listener(move |v, _: &Undo, window, cx| step(v, true, window, cx)))
        .on_action(cx.listener(move |v, _: &Redo, window, cx| step(v, false, window, cx)))
}

#[cfg(test)]
pub(crate) fn handler<V: 'static>(owner: &gpui::Entity<V>, field: FieldFn<V>, on_key: KeyFn<V>) -> impl InputHandler {
    Ime { owner: owner.downgrade(), field, on_key }
}

#[cfg(test)]
mod tests {
    use super::super::Key;
    use super::*;

    fn key(f: &mut TextField, s: &str, cx: &App) -> Key {
        f.handle(&Keystroke::parse(s).unwrap().with_simulated_ime(), cx)
    }

    #[gpui::test]
    fn composition_shows_marked_then_commits_as_one_undo_step(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut f = TextField::default();
            f.replace_and_mark_text_in_range(None, "n", None);
            f.replace_and_mark_text_in_range(None, "に", Some(1..1));
            assert_eq!((f.text.as_str(), f.marked_text_range()), ("に", Some(0..1)));
            assert_eq!(f.selected_text_range().range, 1..1);
            f.replace_text_in_range(None, "日本");
            assert_eq!((f.text.as_str(), f.marked_text_range()), ("日本", None));
            assert_eq!(f.selected_text_range().range, 2..2);
            assert_eq!(key(&mut f, "cmd-z", cx), Key::Edited);
            assert_eq!(f.text, "");
        });
    }

    #[gpui::test]
    fn undo_waits_for_the_composition_to_end(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut f = TextField { text: "a".into(), ..Default::default() };
            f.replace_and_mark_text_in_range(None, "に", None);
            assert_eq!(key(&mut f, "cmd-z", cx), Key::Ignored);
            assert_eq!(key(&mut f, "shift-cmd-z", cx), Key::Ignored);
            assert_eq!((f.text.as_str(), f.marked_text_range()), ("aに", Some(1..2)));
        });
    }

    #[test]
    fn a_dead_key_and_its_letter_make_one_character() {
        let mut f = TextField { text: "caf".into(), ..Default::default() };
        f.replace_and_mark_text_in_range(None, "´", Some(1..1));
        assert_eq!(f.text, "caf´");
        f.replace_text_in_range(None, "é");
        assert_eq!(f.text, "café");
        assert_eq!(f.text.chars().count(), 4);
        assert_eq!(f.marked_text_range(), None);
    }

    #[test]
    fn ranges_count_utf16_past_emoji_and_cjk() {
        let mut f = TextField { text: "😀中a".into(), ..Default::default() };
        assert_eq!(f.selected_text_range().range, 4..4);
        let mut adjusted = None;
        assert_eq!(f.text_for_range(2..3, &mut adjusted).as_deref(), Some("中"));
        assert_eq!(adjusted, Some(2..3));
        f.replace_and_mark_text_in_range(None, "k", None);
        assert_eq!(f.marked_text_range(), Some(4..5));
        f.replace_text_in_range(None, "か");
        assert_eq!(f.text, "😀中aか");
        f.replace_text_in_range(Some(3..4), "á");
        assert_eq!(f.text, "😀中áか");
        assert_eq!(f.selected_text_range().range, 4..4);
    }

    #[gpui::test]
    fn enter_and_escape_end_a_composition_before_they_submit_or_cancel(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut f = TextField::default();
            f.replace_and_mark_text_in_range(None, "に", None);
            assert_eq!(key(&mut f, "enter", cx), Key::Edited);
            assert_eq!((f.text.as_str(), f.marked_text_range()), ("に", None));
            assert_eq!(key(&mut f, "enter", cx), Key::Submit);
            f.replace_and_mark_text_in_range(None, "ほ", None);
            assert_eq!(key(&mut f, "escape", cx), Key::Edited);
            assert_eq!(f.text, "に");
            assert_eq!(key(&mut f, "escape", cx), Key::Cancel);
            assert_eq!(key(&mut f, "cmd-z", cx), Key::Edited);
            assert_eq!(f.text, "", "a composition taken back with Esc is no undo step");
        });
    }

    #[gpui::test]
    fn a_masked_field_types_directly_and_never_gives_its_text_away(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut f = TextField { masked: true, ..Default::default() };
            f.replace_and_mark_text_in_range(None, "¨", Some(1..1));
            f.replace_text_in_range(None, "ä");
            assert_eq!((f.text.as_str(), f.marked_text_range()), ("ä", None));
            assert_eq!(key(&mut f, "b", cx), Key::Edited);
            assert_eq!(f.text, "äb");
            assert_eq!(f.text_for_range(0..2, &mut None), None);
            let mut plain = TextField::default();
            assert_eq!(key(&mut plain, "b", cx), Key::Text, "a plain field waits for the input method");
            assert_eq!(plain.text, "");
        });
    }

    #[test]
    fn control_text_from_a_key_never_types() {
        let mut f = TextField::selected("keep".into());
        f.replace_text_in_range(None, "\t");
        f.replace_text_in_range(None, "\n");
        assert_eq!(f.text, "keep");
        f.replace_text_in_range(None, "one\ntwo");
        assert_eq!(f.text, "one");
    }
}

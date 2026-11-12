//! Single-line text field for every text box in the UI. gpui ships no text
//! input, so keys are applied by hand.
// ponytail: no shift-selections, click-to-place or IME here; the caret moves by key only.
// Upgrade to an `EntityInputHandler` when CJK input matters.

use std::{cell::Cell, rc::Rc, time::Duration};

use crate::theme::r;
use gpui::{
    div, prelude::*, pulsating_between, px, Animation, AnimationExt, App, HighlightStyle, Hsla, Keystroke, MouseButton,
    SharedString, StyledText,
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

#[derive(Default)]
pub struct TextField {
    pub text: String,
    /// Prefilled text is selected so the first keystroke replaces it, like an omnibox.
    /// Shared with the drawn field, so a click on it can drop the selection whoever owns it.
    pub(crate) all_selected: Rc<Cell<bool>>,
    /// The caret, in bytes from the end: code that sets `text` leaves it at the end.
    pub(crate) back: usize,
    /// Draws bullets for the text (passwords).
    pub masked: bool,
}

impl TextField {
    pub fn selected(text: String) -> Self {
        Self {
            text,
            all_selected: Rc::new(Cell::new(true)),
            ..Default::default()
        }
    }

    /// Byte index of the caret.
    fn caret(&self) -> usize {
        let at = self.text.len().saturating_sub(self.back);
        if self.text.is_char_boundary(at) { at } else { self.text.len() }
    }

    fn move_to(&mut self, at: usize) {
        self.back = self.text.len() - at;
    }

    pub fn handle(&mut self, ks: &Keystroke, cx: &App) -> Key {
        let m = &ks.modifiers;
        let (s, at, sel) = (self.text.as_str(), self.caret(), self.all_selected.get());
        let len = s.len();
        let key = match ks.key.as_str() {
            "enter" => return Key::Submit,
            "escape" => return Key::Cancel,
            "up" => return Key::Up,
            "down" => return Key::Down,
            "tab" if !m.control && !m.platform && !m.alt => return Key::Complete,
            // → at the end takes the suggestion, like an omnibox.
            "right" if !m.control && !m.platform && !m.alt && !m.shift && !sel && at == len => {
                return Key::Complete
            }
            // ponytail: no shift-extended selections; the only selection is select-all.
            "left" | "right" | "home" | "end" if !m.shift => {
                let left = matches!(ks.key.as_str(), "left" | "home");
                // A selection collapses to its edge; ⌘ jumps to the ends, ⌥ by word.
                let to = if sel || m.platform || !matches!(ks.key.as_str(), "left" | "right") {
                    if left { 0 } else { len }
                } else if m.alt {
                    if left { word_left(s, at) } else { word_right(s, at) }
                } else if left {
                    prev(s, at)
                } else {
                    next(s, at)
                };
                self.move_to(to);
                Key::Edited
            }
            "backspace" | "delete" if sel => {
                self.text.clear();
                self.back = 0;
                Key::Edited
            }
            "backspace" => {
                let from = if m.secondary() { 0 } else if m.alt { word_left(s, at) } else { prev(s, at) };
                self.text.replace_range(from..at, "");
                Key::Edited
            }
            "delete" => {
                let to = if m.alt { word_right(s, at) } else { next(s, at) };
                self.text.replace_range(at..to, "");
                self.move_to(at);
                Key::Edited
            }
            "a" if m.secondary() => {
                self.all_selected.set(true);
                self.back = 0;
                return Key::Edited;
            }
            "c" | "x" if m.secondary() && sel && !self.masked => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(self.text.clone()));
                if ks.key == "c" {
                    return Key::Ignored;
                }
                self.text.clear();
                self.back = 0;
                Key::Edited
            }
            "v" if m.secondary() => {
                let Some(paste) = cx.read_from_clipboard().and_then(|c| c.text()) else {
                    return Key::Ignored;
                };
                self.insert(paste.lines().next().unwrap_or_default());
                Key::Edited
            }
            _ if m.platform || m.control => return Key::Ignored,
            _ => match ks.key_char.clone() {
                Some(c) => {
                    self.insert(&c);
                    Key::Edited
                }
                None => return Key::Ignored,
            },
        };
        self.all_selected.set(false);
        key
    }

    /// Types at the caret, replacing a selection.
    fn insert(&mut self, s: &str) {
        if self.all_selected.get() {
            self.text.clear();
            self.back = 0;
        }
        let at = self.caret();
        self.text.insert_str(at, s);
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
        let sel = self.all_selected.get() && !empty;
        let at = if sel { self.text.len() } else { self.caret() };
        let show = |s: &str| if self.masked { "\u{2022}".repeat(s.chars().count()) } else { s.to_string() };
        // Empty, the caret sits where typing starts, before the hint.
        let (before, after) =
            if empty { (String::new(), placeholder.to_string()) } else { (show(&self.text[..at]), show(&self.text[at..])) };
        let all_selected = self.all_selected.clone();
        let root = div()
            .flex()
            .items_center()
            // The whole box, not just the glyphs, takes the click.
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            // Tight to the glyphs, so the selection hugs the text.
            .line_height(gpui::relative(1.25))
            .text_color(if empty { dim } else { text })
            // ponytail: a click drops the select-all but does not place the caret (needs the
            // shaped line; `closest_index_for_x` when that matters).
            .on_mouse_down(MouseButton::Left, move |_, window, _| {
                if all_selected.replace(false) {
                    window.refresh();
                }
            });
        if wrap && !empty {
            // Wrapped, the caret is a glyph in the text, so it sits after the right word
            // rather than at the block's edge; only its colour pulses.
            let body = div().flex_1().min_w_0().when(sel, |d| d.bg(selection).rounded(r(3.)));
            if !focused {
                return root.child(body.child(before + &after)).into_any_element();
            }
            let caret_at = before.len();
            let shown = SharedString::from(format!("{before}\u{258f}{after}"));
            return root
                .child(body.with_animation("caret", pulse(), move |d, t| {
                    let caret = HighlightStyle { color: Some(Hsla { a: text.a * t, ..text }), ..Default::default() };
                    d.child(StyledText::new(shown.clone()).with_highlights([(caret_at..caret_at + 3, caret)]))
                }))
                .into_any_element();
        }
        root
            // Long text is cut away from the caret: on the left before it, on the right after it.
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis_start()
                    .when(sel, |d| d.bg(selection).rounded(r(3.)))
                    .child(before),
            )
            .when(focused, |d| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .w(px(1.5))
                        .h(px(16.))
                        .bg(text)
                        .with_animation("caret", pulse(), |d, t| d.opacity(t)),
                )
            })
            .child(div().min_w_0().truncate().child(after))
            .into_any_element()
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

    #[gpui::test]
    fn typing_replaces_selection_then_appends(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut f = TextField::selected("https://old".into());
            let mut ks = Keystroke::parse("a").unwrap();
            ks.key_char = Some("a".into());
            assert_eq!(f.handle(&ks, cx), Key::Edited);
            f.handle(&ks, cx);
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
            assert!(!f.all_selected.get());
            assert_eq!(f.text, "https://old");
            assert_eq!(key(&mut f, "right", cx), Key::Complete);
            assert_eq!(key(&mut f, "cmd-a", cx), Key::Edited);
            assert_eq!(key(&mut f, "left", cx), Key::Edited);
            assert_eq!(f.caret(), 0, "← collapses a selection to its start");
            assert_eq!(f.text, "https://old");
        });
    }

    #[gpui::test]
    fn the_caret_moves_and_edits_in_the_middle(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let mut f = TextField { text: "https://a.com/x".into(), ..Default::default() };
            assert_eq!(f.caret(), f.text.len(), "set text leaves the caret at the end");
            key(&mut f, "alt-left", cx);
            key(&mut f, "alt-left", cx);
            assert_eq!(f.caret(), "https://a.".len());
            let mut ks = Keystroke::parse("b").unwrap();
            ks.key_char = Some("é".into());
            f.handle(&ks, cx);
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
}

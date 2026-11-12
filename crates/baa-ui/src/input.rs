//! Single-line text field for every text box in the UI. gpui ships no text
//! input, so keys are applied by hand.
// ponytail: no caret movement, partial selections or IME here; the field is append/
// backspace/paste only. Upgrade to an `EntityInputHandler` when CJK input matters.

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
    /// Draws bullets for the text (passwords).
    pub masked: bool,
}

impl TextField {
    pub fn selected(text: String) -> Self {
        Self {
            text,
            all_selected: Rc::new(Cell::new(true)),
            masked: false,
        }
    }

    pub fn handle(&mut self, ks: &Keystroke, cx: &App) -> Key {
        let m = &ks.modifiers;
        let key = match ks.key.as_str() {
            "enter" => return Key::Submit,
            "escape" => return Key::Cancel,
            "up" => return Key::Up,
            "down" => return Key::Down,
            // An arrow on a selection collapses it (to the end: the field has no caret position).
            "left" | "right" | "home" | "end" if self.all_selected.get() && !m.shift => {
                self.all_selected.set(false);
                return Key::Edited;
            }
            "tab" if !m.control && !m.platform && !m.alt => return Key::Complete,
            "right" if !m.control && !m.platform && !m.alt && !m.shift => return Key::Complete,
            "backspace" if self.all_selected.get() || m.secondary() || m.alt => {
                self.text.clear();
                Key::Edited
            }
            "backspace" => {
                self.text.pop();
                Key::Edited
            }
            "a" if m.secondary() => {
                self.all_selected.set(true);
                return Key::Edited;
            }
            "v" if m.secondary() => {
                let Some(paste) = cx.read_from_clipboard().and_then(|c| c.text()) else {
                    return Key::Ignored;
                };
                self.replace_selection();
                self.text.push_str(paste.lines().next().unwrap_or_default());
                Key::Edited
            }
            _ if m.platform || m.control => return Key::Ignored,
            _ => match &ks.key_char {
                Some(c) => {
                    self.replace_selection();
                    self.text.push_str(c);
                    Key::Edited
                }
                None => return Key::Ignored,
            },
        };
        self.all_selected.set(false);
        key
    }

    fn replace_selection(&mut self) {
        if self.all_selected.get() {
            self.text.clear();
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
        // Wrapped, a caret beside the block would sit at its right edge, not after the last word.
        let inline_caret = focused && wrap && !empty && !self.masked;
        let caret = || {
            div().flex_shrink_0().w(px(1.5)).h(px(16.)).bg(text).with_animation("caret", pulse(), |d, t| d.opacity(t))
        };
        let shown = if empty {
            placeholder.to_string()
        } else if self.masked {
            "\u{2022}".repeat(self.text.chars().count())
        } else {
            self.text.clone()
        };
        let body = div()
            .when(!wrap, |d| d.truncate())
            .when(wrap, |d| d.flex_1().min_w_0())
            // Tight to the glyphs, so the selection hugs the text.
            .line_height(gpui::relative(1.25))
            .when(self.all_selected.get() && !empty, |d| d.bg(selection).rounded(r(3.)))
            .text_color(if empty { dim } else { text });
        let body = if inline_caret {
            // The caret is a glyph after the last word; only its colour pulses.
            let at = shown.len();
            let shown = SharedString::from(format!("{shown}\u{258f}"));
            body.with_animation("caret", pulse(), move |d, t| {
                let caret = HighlightStyle { color: Some(Hsla { a: text.a * t, ..text }), ..Default::default() };
                d.child(StyledText::new(shown.clone()).with_highlights([(at..shown.len(), caret)]))
            })
            .into_any_element()
        } else {
            body.child(SharedString::from(shown)).into_any_element()
        };
        let all_selected = self.all_selected.clone();
        div()
            .flex()
            .items_center()
            .min_w_0()
            .overflow_hidden()
            // A click in the field drops the select-all, as in any text box.
            .on_mouse_down(MouseButton::Left, move |_, window, _| {
                if all_selected.replace(false) {
                    window.refresh();
                }
            })
            // Empty: the caret sits where typing starts, before the hint.
            .when(focused && empty, |d| d.child(caret()))
            .child(body)
            .when(focused && !empty && !inline_caret, |d| d.child(caret()))
    }
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
            assert_eq!(key(&mut f, "backspace", cx), Key::Edited);
            assert_eq!(f.text, "https://ol", "after deselecting, backspace removes one char");
        });
    }
}

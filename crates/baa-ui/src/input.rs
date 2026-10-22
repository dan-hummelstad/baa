//! Single-line text field for the command bar and find bar. gpui ships no text
//! input, so keys are applied by hand.
// ponytail: no caret movement, selection ranges or IME here; the field is append/
// backspace/paste only. Upgrade to an `EntityInputHandler` when CJK input matters.

use gpui::{div, prelude::*, px, App, Hsla, Keystroke, SharedString};

#[derive(Debug, PartialEq)]
pub enum Key {
    Edited,
    Submit,
    Cancel,
    Up,
    Down,
    Ignored,
}

#[derive(Default)]
pub struct TextField {
    pub text: String,
    /// Prefilled text is selected so the first keystroke replaces it, like an omnibox.
    pub all_selected: bool,
}

impl TextField {
    pub fn selected(text: String) -> Self {
        Self {
            text,
            all_selected: true,
        }
    }

    pub fn handle(&mut self, ks: &Keystroke, cx: &App) -> Key {
        let m = &ks.modifiers;
        let key = match ks.key.as_str() {
            "enter" => return Key::Submit,
            "escape" => return Key::Cancel,
            "up" => return Key::Up,
            "down" => return Key::Down,
            "backspace" if self.all_selected || m.secondary() || m.alt => {
                self.text.clear();
                Key::Edited
            }
            "backspace" => {
                self.text.pop();
                Key::Edited
            }
            "a" if m.secondary() => {
                self.all_selected = true;
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
        self.all_selected = false;
        key
    }

    fn replace_selection(&mut self) {
        if self.all_selected {
            self.text.clear();
        }
    }

    pub fn render(
        &self,
        placeholder: &'static str,
        text: Hsla,
        dim: Hsla,
        selection: Hsla,
    ) -> impl IntoElement {
        let empty = self.text.is_empty();
        let caret = || div().flex_shrink_0().w(px(1.5)).h(px(16.)).bg(text);
        div()
            .flex()
            .items_center()
            .min_w_0()
            .overflow_hidden()
            // Empty: the caret sits where typing starts, before the hint.
            .when(empty, |d| d.child(caret()))
            .child(
                div()
                    .truncate()
                    .when(self.all_selected && !empty, |d| {
                        d.bg(selection).rounded_sm()
                    })
                    .text_color(if empty { dim } else { text })
                    .child(SharedString::from(if empty {
                        placeholder.to_string()
                    } else {
                        self.text.clone()
                    })),
            )
            .when(!empty, |d| d.child(caret()))
    }
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
}

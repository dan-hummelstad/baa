//! What the page card shows with no tab open (pen.dev "Web View" start page).

use std::rc::Rc;

use baa_model::TabId;
use crate::theme::r;
use gpui::{div, img, prelude::*, px, rgb, App, FontWeight, SharedString, Window};

use crate::theme::Theme;

#[cfg(target_os = "macos")]
const TIPS: [(&str, &str, &[&str]); 4] = [
    ("Open the Command Bar", "Search, open a site, or run any command", &["⌘", "T"]),
    ("Hide the sidebar", "Hover the left edge to bring it back", &["⌘", "S"]),
    ("Switch spaces", "Or click the dots under the sidebar", &["⌃", "⌥", "→"]),
    ("Peek a link", "Links that open a tab preview first", &["⇧", "click"]),
];
#[cfg(not(target_os = "macos"))]
const TIPS: [(&str, &str, &[&str]); 4] = [
    ("Open the Command Bar", "Search, open a site, or run any command", &["Ctrl", "T"]),
    ("Hide the sidebar", "Hover the left edge to bring it back", &["Ctrl", "S"]),
    ("Switch spaces", "Or click the dots under the sidebar", &["Ctrl", "Alt", "→"]),
    ("Peek a link", "Links that open a tab preview first", &["Shift", "click"]),
];

/// Activates a tab of the start page's list.
pub type OpenTab = Rc<dyn Fn(TabId, &mut Window, &mut App)>;

/// `dev_servers`: the "Dev servers" folder's tabs, listed above the tips while any listen.
pub fn render(theme: &Theme, dev_servers: &[(TabId, String)], open: OpenTab) -> impl IntoElement {
    let label = |text: &'static str| {
        div().pt(px(44.)).pb(px(8.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(0x6E6A7A)).child(text)
    };
    let servers = dev_servers.iter().map(|(id, title)| {
        let (id, open) = (*id, open.clone());
        div()
            .id(("dev-server", id.0))
            .debug_selector(move || format!("dev-server:{}", id.0))
            .h(px(40.))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(0xECEAF0))
            .cursor_pointer()
            .text_size(px(15.))
            .text_color(rgb(0x17151D))
            .hover(|d| d.text_color(theme.accent))
            .on_click(move |_, window, cx| open(id, window, cx))
            .child(SharedString::from(title.clone()))
    });
    let servers = (!dev_servers.is_empty()).then(|| div().flex().flex_col().child(label("DEV SERVERS")).children(servers));
    let rows = TIPS.iter().map(|(action, hint, keys)| {
        div()
            .h(px(58.))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(rgb(0xECEAF0))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(0x17151D))
                            .child(*action),
                    )
                    .child(div().text_size(px(13.)).text_color(rgb(0x6E6A7A)).child(*hint)),
            )
            .child(div().flex().gap(px(4.)).children(keys.iter().map(|key| {
                div()
                    .h(px(26.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(r(6.))
                    .bg(rgb(0xF6F5F9))
                    .border_1()
                    .border_color(rgb(0xE3E1EA))
                    .text_size(px(12.))
                    .text_color(rgb(0x17151D))
                    .child(SharedString::from(*key))
            })))
    });
    div().size_full().flex().justify_center().pt(px(84.)).child(
        div()
            .w(px(560.))
            .flex()
            .flex_col()
            .child(img(crate::icons::MARK).size(px(40.)).mb(px(20.)))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.accent)
                    .child("WELCOME TO BAA"),
            )
            .child(
                div()
                    .pt(px(18.))
                    .font_family("Georgia")
                    .text_size(px(56.))
                    .line_height(px(62.))
                    .text_color(rgb(0x17151D))
                    .child("The browser that gets out of your way."),
            )
            .children(servers)
            .child(label("TRY IT"))
            .children(rows),
    )
}

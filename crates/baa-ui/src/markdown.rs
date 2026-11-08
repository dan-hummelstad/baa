//! Enough Markdown for Claude's chat replies: headings, lists, quotes, code, bold, italic, links.
//! Links are shown as their label only; a reply's URL is never something baa loads.

use std::ops::Range;

use gpui::{div, prelude::*, px, AnyElement, FontStyle, FontWeight, HighlightStyle, SharedString, StyledText};

use crate::theme::{fade, r, Theme};

#[derive(Debug, PartialEq)]
pub(crate) enum Block {
    Para(Inline),
    Heading(usize, Inline),
    /// `marker` is "•" or "3.".
    Item { depth: usize, marker: String, text: Inline },
    Quote(Vec<Block>),
    Code(String),
    Rule,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum Mark {
    Bold,
    Italic,
    Code,
    Link,
}

#[derive(Debug, PartialEq, Default)]
pub(crate) struct Inline {
    pub text: String,
    pub marks: Vec<(Range<usize>, Mark)>,
}

pub(crate) fn parse(src: &str) -> Vec<Block> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(inline(&para.join("\n"))));
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        i += 1;
        if t.starts_with("```") {
            flush(&mut para, &mut out);
            let start = i;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                i += 1;
            }
            out.push(Block::Code(lines[start..i].join("\n")));
            i += 1;
        } else if t.is_empty() {
            flush(&mut para, &mut out);
        } else if t.starts_with('>') {
            flush(&mut para, &mut out);
            let mut inner = vec![quoted(t)];
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                inner.push(quoted(lines[i].trim_start()));
                i += 1;
            }
            out.push(Block::Quote(parse(&inner.join("\n"))));
        } else if let Some(h) = heading(t) {
            flush(&mut para, &mut out);
            out.push(Block::Heading(h, inline(t[h..].trim())));
        } else if is_rule(t) {
            flush(&mut para, &mut out);
            out.push(Block::Rule);
        } else if let Some((marker, rest)) = item(t) {
            flush(&mut para, &mut out);
            let depth = (line.len() - t.len()) / 2;
            out.push(Block::Item { depth, marker, text: inline(rest) });
        } else {
            para.push(t);
        }
    }
    flush(&mut para, &mut out);
    out
}

fn is_rule(t: &str) -> bool {
    let s: Vec<char> = t.chars().filter(|c| *c != ' ').collect();
    s.len() >= 3 && ['-', '*', '_'].iter().any(|m| s.iter().all(|c| c == m))
}

fn quoted(t: &str) -> &str {
    let t = &t[1..];
    t.strip_prefix(' ').unwrap_or(t)
}

/// The number of `#`s when `t` is an ATX heading.
fn heading(t: &str) -> Option<usize> {
    let n = t.bytes().take_while(|b| *b == b'#').count();
    ((1..=6).contains(&n) && t[n..].starts_with(' ')).then_some(n)
}

fn item(t: &str) -> Option<(String, &str)> {
    for b in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(b) {
            return Some(("•".into(), rest));
        }
    }
    let n = t.bytes().take_while(u8::is_ascii_digit).count();
    let rest = t[n..].strip_prefix(". ").or_else(|| t[n..].strip_prefix(") "));
    (n > 0 && n < 4).then_some(())?;
    rest.map(|rest| (format!("{}.", &t[..n]), rest))
}

pub(crate) fn inline(s: &str) -> Inline {
    let mut out = Inline::default();
    push_inline(s, &mut out);
    out
}

/// `open inner close` at the start of `r`, with a non-empty inner: (inner, bytes used).
fn wrapped<'a>(r: &'a str, open: &str, close: &str) -> Option<(&'a str, usize)> {
    let body = r.strip_prefix(open)?;
    let end = body.find(close).filter(|e| *e > 0)?;
    Some((&body[..end], open.len() + end + close.len()))
}

fn push_inline(s: &str, out: &mut Inline) {
    let mut i = 0;
    while i < s.len() {
        let r = &s[i..];
        if let Some((code, n)) = wrapped(r, "`", "`") {
            let at = out.text.len();
            out.text.push_str(code);
            out.marks.push((at..out.text.len(), Mark::Code));
            i += n;
            continue;
        }
        let styled = wrapped(r, "**", "**")
            .or_else(|| wrapped(r, "__", "__"))
            .map(|(inner, n)| (inner, n, Mark::Bold))
            .or_else(|| {
                wrapped(r, "*", "*")
                    .filter(|(inner, _)| !r.starts_with("**") && !inner.starts_with(' ') && !inner.ends_with(' '))
                    .map(|(inner, n)| (inner, n, Mark::Italic))
            })
            .or_else(|| {
                let (label, n) = wrapped(r, "[", "]")?;
                let (_, m) = wrapped(&r[n..], "(", ")")?;
                Some((label, n + m, Mark::Link))
            });
        if let Some((inner, n, mark)) = styled {
            let at = out.text.len();
            push_inline(inner, out);
            out.marks.push((at..out.text.len(), mark));
            i += n;
            continue;
        }
        let c = r.chars().next().unwrap_or(' ');
        out.text.push(c);
        i += c.len_utf8();
    }
}

fn text(t: &Inline, theme: &Theme) -> StyledText {
    let marks = t.marks.iter().map(|(range, mark)| {
        let style = match mark {
            Mark::Bold => HighlightStyle { font_weight: Some(FontWeight::SEMIBOLD), ..Default::default() },
            Mark::Italic => HighlightStyle { font_style: Some(FontStyle::Italic), ..Default::default() },
            Mark::Code => HighlightStyle { background_color: Some(fade(theme.ink, 0.08)), ..Default::default() },
            Mark::Link => HighlightStyle {
                color: Some(theme.accent),
                underline: Some(gpui::UnderlineStyle { thickness: px(1.), color: Some(fade(theme.accent, 0.5)), wavy: false }),
                ..Default::default()
            },
        };
        (range.clone(), style)
    });
    let mono = t.marks.iter().filter(|(_, m)| *m == Mark::Code).map(|(range, _)| (range.clone(), SharedString::from("Menlo")));
    StyledText::new(t.text.clone()).with_highlights(marks.collect::<Vec<_>>()).with_font_family_overrides(mono.collect::<Vec<_>>())
}

pub(crate) fn render(blocks: &[Block], theme: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .children(blocks.iter().map(|b| match b {
            Block::Para(t) => div().child(text(t, theme)).into_any_element(),
            Block::Heading(level, t) => div()
                .pt(px(4.))
                .text_size(px(if *level <= 2 { 14.5 } else { 13. }))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.ink)
                .child(text(t, theme))
                .into_any_element(),
            Block::Item { depth, marker, text: t } => div()
                .pl(px(*depth as f32 * 14.))
                .flex()
                .gap(px(6.))
                .child(div().flex_none().min_w(px(12.)).text_color(theme.ink_muted).child(SharedString::from(marker.clone())))
                .child(div().flex_1().min_w_0().child(text(t, theme)))
                .into_any_element(),
            Block::Quote(inner) => div()
                .pl(px(10.))
                .border_l_2()
                .border_color(fade(theme.ink, 0.18))
                .text_color(theme.ink_muted)
                .child(render(inner, theme))
                .into_any_element(),
            Block::Code(code) => div()
                .px(px(10.))
                .py(px(8.))
                .rounded(r(8.))
                .bg(fade(theme.ink, 0.06))
                .font_family("Menlo")
                .text_size(px(11.))
                .child(SharedString::from(code.clone()))
                .into_any_element(),
            Block::Rule => div().my(px(2.)).h(px(1.)).bg(theme.divider).into_any_element(),
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screenshot_reply_parses() {
        let src = "From **Christian (@CF)**, 1 hour ago:\n\n> \"I think\n>\n> Typescript 7\"\n\n- one *two*\n  - `three`\n2. [label](https://x.com)\n\n```\nlet a = 1;\n```\n---";
        let b = parse(src);
        let Block::Para(p) = &b[0] else { panic!("{b:?}") };
        assert_eq!(p.text, "From Christian (@CF), 1 hour ago:");
        assert_eq!(p.marks, vec![(5..20, Mark::Bold)]);
        let Block::Quote(q) = &b[1] else { panic!("{b:?}") };
        assert_eq!(q.len(), 2, "the bare > is a paragraph break inside the quote");
        assert_eq!(b[2], Block::Item { depth: 0, marker: "•".into(), text: Inline { text: "one two".into(), marks: vec![(4..7, Mark::Italic)] } });
        assert_eq!(b[3], Block::Item { depth: 1, marker: "•".into(), text: Inline { text: "three".into(), marks: vec![(0..5, Mark::Code)] } });
        assert_eq!(b[4], Block::Item { depth: 0, marker: "2.".into(), text: Inline { text: "label".into(), marks: vec![(0..5, Mark::Link)] } });
        assert_eq!(b[5], Block::Code("let a = 1;".into()));
        assert_eq!(b[6], Block::Rule);
    }

    #[test]
    fn stray_markers_stay_literal() {
        assert_eq!(inline("2 * 3 * 4 and **open").text, "2 * 3 * 4 and **open");
        assert_eq!(inline("snake_case_name").text, "snake_case_name");
        assert_eq!(parse("# Title\n#nope")[0], Block::Heading(1, inline("Title")));
        assert_eq!(parse("2024. was a year")[0], Block::Para(inline("2024. was a year")));
    }
}

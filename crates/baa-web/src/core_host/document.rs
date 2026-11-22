//! The pages baa writes itself (PLAN-mail.md §6): the whole policy for untrusted mail, in one place.
//!
//! A document is a tab at `about:blank` (an opaque origin) that never navigates. Its page is a
//! content security policy and one sandboxed frame; the mail is the frame's `srcdoc`, so it parses
//! in a document of its own and never as part of the page. The browser's sandbox and policy are the
//! control: nothing here cleans HTML beyond dropping `<link>`, which no policy directive covers.

use crate::DocumentKind;

/// The message frame: links open as new tabs through the engine (and escape the sandbox, so the
/// site works there); no scripts, forms, same origin, top navigation, or automatic features
/// (`<meta http-equiv=refresh>` among them).
pub(super) const MESSAGE_SANDBOX: &str = "allow-popups allow-popups-to-escape-sandbox";
/// The editor frame shares the page's opaque origin, so the shell's script can edit and read it;
/// without `allow-scripts` that grants the mail nothing, and quoted mail cannot refresh the page
/// away or open anything.
pub(super) const EDITOR_SANDBOX: &str = "allow-same-origin";

/// The script expression for the editor's document; its `body` is the editable content.
pub const EDITOR_DOCUMENT: &str = "frames[0].document";

pub(super) fn policy(kind: DocumentKind) -> String {
    let images = match kind {
        DocumentKind::Message { remote_images: false } => "data:",
        DocumentKind::Message { remote_images: true } => "data: https:",
        DocumentKind::Editor => "data: blob:",
    };
    format!("default-src 'none'; img-src {images}; style-src 'unsafe-inline'; frame-src 'none'; form-action 'none'")
}

/// The page around the frame. The policy is in its head before the frame exists, and a `srcdoc`
/// frame inherits it.
pub(super) fn page_html(kind: DocumentKind) -> String {
    let sandbox = match kind {
        DocumentKind::Message { .. } => MESSAGE_SANDBOX,
        DocumentKind::Editor => EDITOR_SANDBOX,
    };
    format!(
        "<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"{}\">\
         <style>html,body{{margin:0;height:100%}}iframe{{display:block;border:0;width:100%;height:100%;background:#fff}}</style>\
         </head><body><iframe sandbox=\"{sandbox}\"></iframe></body></html>",
        policy(kind)
    )
}

/// Run in the page's main world, from the browser side. A meta policy can only be added to, never
/// loosened, so a document keeps the policy it was made with; remote images need a new document.
pub(super) fn write_script(kind: DocumentKind, html: &str) -> String {
    let (head, tail) = script_parts(kind);
    format!("{head}{}{tail}", serde_json::Value::from(html))
}

/// The script around the content: the content goes between the two halves as one string literal.
fn script_parts(kind: DocumentKind) -> (String, String) {
    let head = format!(
        "(()=>{{if(location.href!==\"about:blank\"||origin!==\"null\")return;\
         let f=document.querySelector(\"iframe\");\
         if(!f){{document.open();document.write({});document.close();f=document.querySelector(\"iframe\")}}\
         const d=new DOMParser().parseFromString(",
        serde_json::Value::from(page_html(kind))
    );
    // `<link>` goes because preconnect and DNS prefetch are outside every directive. For the same
    // reason link hosts are never prefetched: Blink lets a mail's own `x-dns-prefetch-control: on`
    // turn that on in a frame, but not after an "off" (Document::ParseDNSPrefetchControlHeader), so
    // this one comes first. A message's links leave as new tabs: a `<base>` the mail brings comes
    // after this one and does not win.
    let finish = match kind {
        DocumentKind::Message { .. } => "const b=d.createElement(\"base\");b.target=\"_blank\";d.head.prepend(m,b);",
        DocumentKind::Editor => "d.head.prepend(m);d.body.contentEditable=\"true\";",
    };
    let tail = format!(
        ",\"text/html\");d.querySelectorAll(\"link\").forEach(e=>e.remove());\
         const m=d.createElement(\"meta\");m.httpEquiv=\"x-dns-prefetch-control\";m.content=\"off\";\
         {finish}f.srcdoc=d.documentElement.outerHTML}})()"
    );
    (head, tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSAGE: DocumentKind = DocumentKind::Message { remote_images: false };

    #[test]
    fn each_kind_has_its_policy() {
        assert_eq!(
            policy(MESSAGE),
            "default-src 'none'; img-src data:; style-src 'unsafe-inline'; frame-src 'none'; form-action 'none'"
        );
        assert_eq!(
            policy(DocumentKind::Message { remote_images: true }),
            "default-src 'none'; img-src data: https:; style-src 'unsafe-inline'; frame-src 'none'; form-action 'none'"
        );
        assert_eq!(
            policy(DocumentKind::Editor),
            "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; frame-src 'none'; form-action 'none'"
        );
    }

    #[test]
    fn the_policy_comes_before_the_frame() {
        for kind in [MESSAGE, DocumentKind::Message { remote_images: true }, DocumentKind::Editor] {
            let page = page_html(kind);
            let meta = page.find(&format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">", policy(kind))).unwrap();
            assert!(meta < page.find("<iframe").unwrap(), "{page}");
            assert_eq!(page.matches("<iframe").count(), 1);
            assert!(!page.contains("srcdoc"), "the content is never part of the page's markup");
        }
    }

    #[test]
    fn no_sandbox_lets_the_mail_run_or_leave() {
        for sandbox in [MESSAGE_SANDBOX, EDITOR_SANDBOX] {
            for banned in ["allow-scripts", "allow-forms", "allow-top-navigation", "allow-modals", "allow-downloads"] {
                assert!(!sandbox.contains(banned), "{sandbox}");
            }
        }
        assert!(!MESSAGE_SANDBOX.contains("allow-same-origin"));
        assert!(!EDITOR_SANDBOX.contains("allow-popups"));
        assert!(page_html(MESSAGE).contains(&format!("<iframe sandbox=\"{MESSAGE_SANDBOX}\">")));
        assert!(page_html(DocumentKind::Editor).contains(&format!("<iframe sandbox=\"{EDITOR_SANDBOX}\">")));
    }

    #[test]
    fn hostile_content_stays_one_string() {
        let lone_surrogate = String::from_utf16_lossy(&[0xD800]);
        let html = format!(
            "</iframe><script>alert(1)</script>\"'`${{x}}\\\u{2028}\u{2029}</script>\0{lone_surrogate}<!--"
        );
        for kind in [MESSAGE, DocumentKind::Editor] {
            let script = write_script(kind, &html);
            let (head, tail) = script_parts(kind);
            let literal = script.strip_prefix(&head).and_then(|s| s.strip_suffix(&tail)).unwrap();
            assert_eq!(serde_json::from_str::<String>(literal).unwrap(), html);
            assert!(!literal.contains('\0'), "a NUL would end the C string");
            assert_eq!(script.lines().count(), 1, "one line, as the harness reads it");
        }
    }

    #[test]
    fn the_script_writes_only_into_a_blank_opaque_page() {
        let script = write_script(MESSAGE, "<p>hi</p>");
        assert!(script.starts_with("(()=>{if(location.href!==\"about:blank\"||origin!==\"null\")return;"));
        let off = "const m=d.createElement(\"meta\");m.httpEquiv=\"x-dns-prefetch-control\";m.content=\"off\";";
        assert!(script.contains(&format!("{off}const b=d.createElement(\"base\");b.target=\"_blank\";d.head.prepend(m,b);f.srcdoc=")));
        let editor = write_script(DocumentKind::Editor, "");
        assert!(editor.contains(&format!("{off}d.head.prepend(m);d.body.contentEditable=\"true\";f.srcdoc=")));
    }

    /// `crates/baa-chromium/tests/mail-document.txt` runs the exact scripts this module writes.
    #[test]
    fn the_harness_runs_these_scripts() {
        let harness = include_str!("../../../baa-chromium/tests/mail-document.txt");
        let message = include_str!("../../../baa-chromium/tests/mail-message.html").trim_end();
        let editor = include_str!("../../../baa-chromium/tests/mail-editor.html").trim_end();
        assert!(harness.lines().any(|l| l == format!("eval {}", write_script(MESSAGE, message))), "regenerate the message line");
        assert!(harness.lines().any(|l| l == format!("eval {}", write_script(DocumentKind::Editor, editor))), "regenerate the editor line");
    }
}

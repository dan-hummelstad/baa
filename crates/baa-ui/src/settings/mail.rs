//! Settings › Mail: the switch, the accounts and the form that adds one (`mail/setup.rs`).

use baa_mail::{provider::Provider, Account, Error, SignIn};
use baa_model::{RemoteImages, Settings};
use baa_web::WebHost;
use gpui::{div, prelude::*, px, AnyElement, Context, SharedString};

use super::{Field, Item, SettingsView};
use crate::{mail::setup::MailField, theme::surface};

pub(crate) fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::ICloud => "iCloud",
        Provider::Gmail => "Gmail",
        Provider::Fastmail => "Fastmail",
        Provider::Yahoo => "Yahoo",
        Provider::Other => "Other IMAP",
    }
}

/// The mail actions of the Keyboard tab, under their own heading.
pub(super) fn shortcuts() -> Vec<(&'static str, Box<dyn gpui::Action>)> {
    use crate::actions::*;
    vec![
        ("New Message", Box::new(NewMessage)),
        ("Reply", Box::new(Reply)),
        ("Reply All", Box::new(ReplyAll)),
        ("Forward Message", Box::new(ForwardMessage)),
        ("Send", Box::new(SendMessage)),
        ("Delete Message", Box::new(DeleteMessage)),
        ("Search Mail", Box::new(SearchMail)),
        ("Back to the List", Box::new(BackToList)),
        ("Browser ⇄ Mail", Box::new(ToggleMail)),
    ]
}

impl<H: WebHost> SettingsView<H> {
    pub(super) fn mail_tab(&self, s: &Settings, cx: &mut Context<Self>) -> Vec<Item> {
        let (private, running) = {
            let b = self.browser.read(cx);
            (b.private, b.mail.is_some())
        };
        let detail = match (running, private, s.mail.enabled) {
            (true, ..) => "",
            (_, true, _) => "Mail is not available in a private window.",
            (_, _, true) => "Mail is not running in this window.",
            _ => "Turn on Mail to add an account.",
        };
        let add = running.then(|| {
            let label = if self.mail_setup.open { "Cancel" } else { "Add an account…" };
            self.button("mail-add", label, false, cx).on_click(cx.listener(|this, _, _, cx| this.mail_toggle_form(cx))).into_any_element()
        });
        let mut out = vec![
            self.row(
                "mail_enabled",
                "Off by default. No data leaves this Mac except to your own mail servers.",
                self.toggle("pref-mail", s.mail.enabled, |s, v| s.mail.enabled = v, cx),
            ),
            self.row(
                "mail_codes",
                "Offers a code from a message to the page waiting for one, from the accounts switched on below.",
                self.toggle("pref-mail-codes", s.mail.codes, |s, v| s.mail.codes = v, cx),
            ),
            self.row("mail_accounts", detail, add.unwrap_or_else(|| div().into_any_element())),
        ];
        if running {
            out.extend(self.mail_accounts(s.mail.codes, cx));
            if self.mail_setup.open {
                out.extend(self.mail_form(cx));
            }
        }
        out.push(self.row(
            "mail_images",
            "Loading one tells the sender the message was opened.",
            self.segments(
                "pref-mail-images",
                vec![
                    (RemoteImages::Ask, "Ask for each message".into()),
                    (RemoteImages::Always, "Always load".into()),
                    (RemoteImages::Never, "Never load".into()),
                ],
                s.mail.remote_images,
                |v, s| s.mail.remote_images = v,
                cx,
            ),
        ));
        out
    }

    /// The accounts of each stored Space, then those of every Space once.
    fn mail_accounts(&self, codes: bool, cx: &mut Context<Self>) -> Vec<Item> {
        let groups: Vec<(SharedString, Vec<Account>)> = {
            let b = self.browser.read(cx);
            let mut shared: Vec<Account> = Vec::new();
            let mut groups = Vec::new();
            for space in b.session.spaces().iter().filter(|s| !s.ephemeral) {
                let Some((mail, mail_space)) = b.mail_of(space.id) else { continue };
                let (own, every): (Vec<_>, Vec<_>) = mail.accounts(mail_space).into_iter().partition(|a| a.space.is_some());
                for account in every {
                    if !shared.iter().any(|a| a.id == account.id) {
                        shared.push(account);
                    }
                }
                groups.push((SharedString::from(format!("{} {}", space.icon, space.name)), own));
            }
            groups.push(("Every Space".into(), shared));
            groups
        };
        let mut out = Vec::new();
        if groups.iter().all(|(_, accounts)| accounts.is_empty()) {
            out.push(self.note("No accounts yet."));
        }
        for (title, accounts) in groups.into_iter().filter(|(_, a)| !a.is_empty()) {
            out.push((None, div().pt(px(10.)).px(px(26.)).text_size(px(11.)).text_color(surface::muted()).child(title).into_any_element()));
            out.extend(accounts.iter().map(|a| self.mail_account_row(a, codes, cx)));
        }
        out
    }

    /// `codes`: the global switch; off, the account's own is greyed.
    fn mail_account_row(&self, account: &Account, codes: bool, cx: &mut Context<Self>) -> Item {
        let id = account.id;
        let confirming = self.mail_setup.confirm_remove == Some(id);
        let detail = if confirming {
            format!("Remove {}? Its mail here and its sign-in go; the mail stays on the server.", account.address)
        } else {
            format!("{} · {}", account.address, provider_name(account.provider))
        };
        let buttons = if confirming {
            div()
                .flex()
                .gap(px(6.))
                .child(self.button("mail-remove-cancel", "Cancel", false, cx).on_click(cx.listener(|this, _, _, cx| this.mail_ask_remove(None, cx))))
                .child(self.button("mail-remove-yes", "Remove", true, cx).on_click(cx.listener(move |this, _, _, cx| this.mail_remove(id, cx))))
                .into_any_element()
        } else {
            self.button("mail-remove", "Remove", false, cx).on_click(cx.listener(move |this, _, _, cx| this.mail_ask_remove(Some(id), cx))).into_any_element()
        };
        let el = div()
            .id(("mail-account", id.0))
            .debug_selector(move || format!("mail-account:{}", id.0))
            .py(px(5.))
            .pl(px(26.))
            .pr(px(10.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().truncate().text_size(px(12.5)).child(SharedString::from(account.name.clone())))
                    .child(div().text_size(px(11.5)).text_color(surface::muted()).child(SharedString::from(detail))),
            )
            .child(self.mail_codes_switch(account, codes, cx))
            .child(div().flex_none().child(buttons));
        (None, el.into_any_element())
    }

    fn mail_codes_switch(&self, account: &Account, enabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let (id, on) = (account.id, account.codes);
        let el = div()
            .id(("mail-account-codes", id.0))
            .debug_selector(move || format!("mail-account-codes:{}:{}", id.0, if on { "on" } else { "off" }))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .text_size(px(11.5))
            .text_color(surface::muted())
            .child("Codes")
            .child(self.switch("mail-account-codes", on));
        if !enabled {
            return el.opacity(0.4).into_any_element();
        }
        el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.mail_set_codes(id, !on, cx))).into_any_element()
    }

    fn form_row(&self, selector: &'static str, label: &'static str, control: AnyElement) -> Item {
        self.sub_row(selector.into(), SharedString::from(label).into_any_element(), control)
    }

    fn mail_form(&self, cx: &mut Context<Self>) -> Vec<Item> {
        let space_name = {
            let b = self.browser.read(cx);
            b.mail_space().and_then(|(_, space)| b.session.space(space.id())).map(|s| s.name.clone())
        };
        let Some(space_name) = space_name else {
            return vec![self.note("Mail is not available in an incognito Space. Switch to another Space to add an account.")];
        };
        let setup = &self.mail_setup;
        let provider = setup.provider;
        let providers = [Provider::ICloud, Provider::Gmail, Provider::Fastmail, Provider::Yahoo, Provider::Other];
        let mut out = vec![
            self.form_row(
                "mail-form:provider",
                "Provider",
                self.segments_with("mail-provider", providers.map(|p| (p, provider_name(p).into())).to_vec(), provider, |this, p, cx| this.mail_pick_provider(p, cx), cx),
            ),
            self.form_row(
                "mail-form:where",
                "Where",
                self.segments_with(
                    "mail-where",
                    vec![(false, format!("This Space ({space_name})").into()), (true, "Every Space".into())],
                    setup.every_space,
                    |this, every, cx| this.mail_set_every_space(every, cx),
                    cx,
                ),
            ),
            self.form_row("mail-form:name", "Name", self.field(Field::Mail(MailField::Name), "mail-name", "Your name", 260., cx)),
            self.form_row("mail-form:address", "Address", self.field(Field::Mail(MailField::Address), "mail-address", "you@example.com", 260., cx)),
        ];
        let busy = setup.busy;
        let go = match provider.sign_in() {
            SignIn::Password => {
                out.push(self.form_row("mail-form:password", "App password", self.field(Field::Mail(MailField::Password), "mail-password", "App password", 260., cx)));
                if provider == Provider::Other {
                    out.push(self.form_row("mail-form:imap", "IMAP server", self.server_fields((Field::Mail(MailField::ImapHost), Field::Mail(MailField::ImapPort)), ("mail-imap-host", "mail-imap-port"), "imap.example.com", cx)));
                    out.push(self.form_row("mail-form:smtp", "SMTP server", self.server_fields((Field::Mail(MailField::SmtpHost), Field::Mail(MailField::SmtpPort)), ("mail-smtp-host", "mail-smtp-port"), "smtp.example.com", cx)));
                    out.push(self.note("baa connects only on the standard ports: 993 or 143 for IMAP, 465 or 587 for SMTP."));
                }
                (if busy { "Signing in…" } else { "Add account" }, "mail-add-account", true)
            }
            SignIn::Google => match &setup.google {
                Some(Err(Error::NotSetUp)) => ("Google sign-in is not set up in this build yet", "mail-google", false),
                Some(Err(_)) => ("Sign in with Google", "mail-google", false),
                _ => (if busy { "Signing in…" } else { "Sign in with Google" }, "mail-google", true),
            },
        };
        let (label, id, enabled) = go;
        let button = self.button(id, label, enabled, cx);
        let button = if enabled && !busy {
            button.on_click(cx.listener(|this, _, _, cx| this.mail_add_account(cx)))
        } else {
            button.opacity(0.5)
        };
        let mut row = div().flex().items_center().gap(px(8.)).child(button);
        if let Some(error) = setup.error.clone().or_else(|| match &setup.google {
            Some(Err(e)) if *e != Error::NotSetUp => Some(e.to_string()),
            _ => None,
        }) {
            row = row.child(div().debug_selector(|| "mail-error".into()).text_size(px(12.)).text_color(surface::warn()).child(SharedString::from(error)));
        }
        out.push((None, div().py(px(6.)).pl(px(26.)).child(row).into_any_element()));
        out
    }

    fn server_fields(&self, fields: (Field, Field), ids: (&'static str, &'static str), placeholder: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let ((host, port), (host_id, port_id)) = (fields, ids);
        div()
            .flex()
            .gap(px(6.))
            .child(self.field(host, host_id, placeholder, 190., cx))
            .child(self.field(port, port_id, "Port", 56., cx))
            .into_any_element()
    }
}

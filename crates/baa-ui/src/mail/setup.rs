//! Adding and removing a mail account (WP-K): the state and steps of Settings › Mail's form.
//! `settings/mail.rs` draws it.

use baa_mail::{oauth::GoogleSignIn, provider::Provider, AccountId, MailSpace, NewAccount, NewSignIn, Server, Servers, SignIn};
use baa_web::WebHost;
use gpui::{AppContext, Context};

use crate::{
    browser::Browser,
    input::TextField,
    settings::{Field, SettingsView, Tab},
};

pub(crate) struct Setup {
    pub(crate) open: bool,
    pub(crate) provider: Provider,
    /// The account belongs to every Space, not the active one.
    pub(crate) every_space: bool,
    /// A sign-in call is under way: the form takes no second one.
    pub(crate) busy: bool,
    pub(crate) error: Option<String>,
    /// Remove was asked for this account and waits for its confirmation.
    pub(crate) confirm_remove: Option<AccountId>,
    /// Google's sign-in, made when Gmail is chosen so that "not set up" shows before a click.
    pub(crate) google: Option<baa_mail::Result<GoogleSignIn>>,
}

impl Default for Setup {
    fn default() -> Self {
        Self { open: false, provider: Provider::ICloud, every_space: false, busy: false, error: None, confirm_remove: None, google: None }
    }
}

/// The add form's fields, in Settings' `Field::Mail`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum MailField {
    Name,
    Address,
    Password,
    ImapHost,
    ImapPort,
    SmtpHost,
    SmtpPort,
}

const MAIL_FIELDS: [MailField; 7] =
    [MailField::Name, MailField::Address, MailField::Password, MailField::ImapHost, MailField::ImapPort, MailField::SmtpHost, MailField::SmtpPort];

fn masked() -> TextField {
    TextField { masked: true, ..Default::default() }
}

fn text(text: &str) -> TextField {
    TextField { text: text.into(), ..Default::default() }
}

impl<H: WebHost> Browser<H> {
    /// Settings › Mail, where accounts are added: the door's "Add an account".
    pub(crate) fn open_mail_settings(&mut self, cx: &mut Context<Self>) {
        self.open_settings_at(Some(Tab::Mail), Some("mail_accounts"), cx);
    }
}

impl<H: WebHost> SettingsView<H> {
    pub(crate) fn mail_toggle_form(&mut self, cx: &mut Context<Self>) {
        if self.mail_setup.busy {
            return;
        }
        let open = !self.mail_setup.open;
        self.mail_close_form();
        if open {
            self.mail_setup.open = true;
            self.mail_refresh_google();
        }
        cx.notify();
    }

    /// Back to a closed, empty form, unless a sign-in is under way.
    pub(crate) fn mail_close_form(&mut self) {
        if self.mail_setup.busy {
            return;
        }
        self.mail_setup = Setup::default();
        for field in MAIL_FIELDS {
            self.fields.remove(&Field::Mail(field));
        }
        self.fields.insert(Field::Mail(MailField::Password), masked());
        self.fields.insert(Field::Mail(MailField::ImapPort), text("993"));
        self.fields.insert(Field::Mail(MailField::SmtpPort), text("465"));
    }

    pub(crate) fn mail_pick_provider(&mut self, provider: Provider, cx: &mut Context<Self>) {
        self.mail_setup.provider = provider;
        self.mail_setup.error = None;
        self.mail_refresh_google();
        cx.notify();
    }

    /// A typed address that belongs to a preset selects it; any other leaves the choice alone.
    pub(crate) fn mail_address_edited(&mut self) {
        let provider = Provider::of(self.text(Field::Mail(MailField::Address)).trim());
        if provider != Provider::Other && provider != self.mail_setup.provider {
            self.mail_setup.provider = provider;
            self.mail_refresh_google();
        }
    }

    // ponytail: starting a sign-in binds a loopback port, so it is kept while Gmail is chosen
    // and dropped when it is not; the only way to learn "not set up" is to start one.
    fn mail_refresh_google(&mut self) {
        if self.mail_setup.provider.sign_in() != SignIn::Google {
            self.mail_setup.google = None;
        } else if self.mail_setup.google.is_none() {
            self.mail_setup.google = Some(GoogleSignIn::start());
        }
    }

    pub(crate) fn mail_set_every_space(&mut self, every: bool, cx: &mut Context<Self>) {
        self.mail_setup.every_space = every;
        cx.notify();
    }

    fn mail_fail(&mut self, why: impl Into<String>, cx: &mut Context<Self>) {
        self.mail_setup.error = Some(why.into());
        cx.notify();
    }

    /// The servers of the chosen provider: its preset, or what the form says for Other.
    fn mail_servers(&self, address: &str) -> Result<Servers, String> {
        if let Some(servers) = self.mail_setup.provider.servers(address) {
            return Ok(servers);
        }
        let host = |field| self.text(field).trim().to_owned();
        let port = |field| self.text(field).trim().parse::<u16>().map_err(|_| "A port is a number".to_owned());
        Ok(Servers {
            imap: Server { host: host(Field::Mail(MailField::ImapHost)), port: port(Field::Mail(MailField::ImapPort))? },
            smtp: Server { host: host(Field::Mail(MailField::SmtpHost)), port: port(Field::Mail(MailField::SmtpPort))? },
            login: address.to_owned(),
        })
    }

    /// Add account, or Sign in with Google: the checks, then the sign-in off the UI thread.
    pub(crate) fn mail_add_account(&mut self, cx: &mut Context<Self>) {
        if self.mail_setup.busy {
            return;
        }
        let Some((mail, space)) = self.browser.read(cx).mail_space().map(|(mail, space)| (mail.clone(), space)) else { return };
        let address = self.text(Field::Mail(MailField::Address)).trim().to_owned();
        if !address.contains('@') {
            return self.mail_fail("Enter the address of the account", cx);
        }
        let servers = match self.mail_servers(&address) {
            Ok(servers) => servers,
            Err(why) => return self.mail_fail(why, cx),
        };
        let name = match self.text(Field::Mail(MailField::Name)).trim() {
            "" => address.clone(),
            name => name.to_owned(),
        };
        let provider = self.mail_setup.provider;
        let owner: Option<MailSpace> = (!self.mail_setup.every_space).then_some(space);
        let new = move |sign_in| NewAccount { space: owner, name, address, provider, servers, sign_in };
        match provider.sign_in() {
            SignIn::Password => {
                if self.text(Field::Mail(MailField::Password)).is_empty() {
                    return self.mail_fail("Enter the app password", cx);
                }
                let sign_in = NewSignIn::Password(self.take_mail_password());
                let new = new(sign_in);
                self.mail_run(cx, move || mail.add_account(new));
            }
            SignIn::Google => {
                if !matches!(self.mail_setup.google, Some(Ok(_))) {
                    return;
                }
                let Some(Ok(google)) = self.mail_setup.google.take() else { return };
                let url = google.url().to_owned();
                let space = space.id();
                // The URL is baa's own, built by `GoogleSignIn`: a user-initiated load.
                self.in_browser(cx, true, move |b, window, cx| b.open_url_in(space, &url, window, cx));
                // ponytail: `finish` cannot be cancelled, so a form waiting on Google stays
                // busy for up to its 5 minutes.
                self.mail_run(cx, move || {
                    let tokens = google.finish()?;
                    mail.add_account(new(NewSignIn::Google(tokens)))
                });
            }
        }
    }

    /// The password moves out of its field, which is replaced so no copy stays in its undo history.
    fn take_mail_password(&mut self) -> String {
        let field = self.fields.entry(Field::Mail(MailField::Password)).or_default();
        let password = std::mem::take(&mut field.text);
        *field = masked();
        password
    }

    fn mail_run(&mut self, cx: &mut Context<Self>, work: impl FnOnce() -> baa_mail::Result<AccountId> + Send + 'static) {
        self.mail_setup.busy = true;
        self.mail_setup.error = None;
        let task = cx.background_spawn(async move { work() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.mail_added(result, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn mail_added(&mut self, result: baa_mail::Result<AccountId>, cx: &mut Context<Self>) {
        self.mail_setup.busy = false;
        match result {
            Ok(_) => {
                self.mail_close_form();
                self.browser.update(cx, |_, cx| cx.notify());
            }
            // Gmail's sign-in is consumed by the attempt: the next one starts afresh.
            Err(error) => {
                self.mail_refresh_google();
                self.mail_setup.error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    pub(crate) fn mail_ask_remove(&mut self, id: Option<AccountId>, cx: &mut Context<Self>) {
        self.mail_setup.confirm_remove = id;
        cx.notify();
    }

    pub(crate) fn mail_remove(&mut self, id: AccountId, cx: &mut Context<Self>) {
        self.mail_setup.confirm_remove = None;
        if let Some(mail) = self.browser.read(cx).mail.as_ref().map(|m| m.mail.clone()) {
            mail.remove_account(id);
            crate::mail::account_removed(id, cx);
        }
        cx.notify();
    }

    pub(crate) fn mail_set_codes(&mut self, id: AccountId, on: bool, cx: &mut Context<Self>) {
        if let Some(mail) = self.browser.read(cx).mail.as_ref().map(|m| m.mail.clone()) {
            mail.set_codes(id, on);
        }
        cx.notify();
    }
}

use baa_model::SpaceId;
use serde::{Deserialize, Serialize};

use crate::{oauth::Tokens, provider::Provider, AccountId, Error, MailSpace, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    /// `None`: an account of every Space.
    pub space: Option<SpaceId>,
    /// The person's name, as the From line carries it.
    pub name: String,
    pub address: String,
    /// Verification codes from its mail are offered to pages (PLAN-mail.md §7).
    pub codes: bool,
    pub provider: Provider,
    pub servers: Servers,
    pub sign_in: SignIn,
}

/// Where an account's mail is. [`crate::provider::servers`] fills it for a preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Servers {
    pub imap: Server,
    pub smtp: Server,
    /// The user name both servers take; most take the address.
    pub login: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
}

/// How an account signs in. The secret itself is in [`crate::Secrets`], never here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignIn {
    /// A password or app password ([`crate::Secret::Password`]).
    Password,
    /// Google OAuth ([`crate::Secret::RefreshToken`]); IMAP and SMTP by XOAUTH2.
    Google,
}

/// What [`crate::Mail::add_account`] is given. No `Debug`: it holds the secret.
pub struct NewAccount {
    /// `None`: an account of every Space.
    pub space: Option<MailSpace>,
    pub name: String,
    pub address: String,
    pub provider: Provider,
    pub servers: Servers,
    pub sign_in: NewSignIn,
}

/// No `Debug`: it is the secret.
pub enum NewSignIn {
    Password(String),
    /// From [`crate::oauth::GoogleSignIn::finish`].
    Google(Tokens),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Imap,
    Pop3,
    Smtp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tls {
    /// TLS from the first byte.
    Implicit,
    /// Plain until STARTTLS, which must succeed before anything else is sent.
    StartTls,
}

/// The one place a mail port is allowed (PLAN-mail.md §9, the user's decision of 2026-11-22):
/// the standard ports only, and each with the TLS it must have. There is no plain text.
pub fn check_port(protocol: Protocol, port: u16) -> Result<Tls> {
    match (protocol, port) {
        (Protocol::Imap, 993) | (Protocol::Pop3, 995) | (Protocol::Smtp, 465) => Ok(Tls::Implicit),
        (Protocol::Imap, 143) | (Protocol::Pop3, 110) | (Protocol::Smtp, 587) => Ok(Tls::StartTls),
        _ => Err(Error::Port(port)),
    }
}

impl Server {
    /// The port's TLS, if the port is allowed and the host is a plain host name or address:
    /// the host goes into curl's URL and TLS's server name, so `user@`, `/` or a scheme in it
    /// would change where the connection goes.
    pub fn check(&self, protocol: Protocol) -> Result<Tls> {
        let tls = check_port(protocol, self.port)?;
        let plain = !self.host.is_empty() && self.host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        if !plain || url::Host::parse(&self.host).is_err() {
            return Err(Error::Other("That is not a server name".into()));
        }
        Ok(tls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_standard_mail_ports_are_allowed_each_with_its_tls() {
        use Protocol::*;
        for (protocol, port, tls) in [
            (Imap, 993, Tls::Implicit),
            (Imap, 143, Tls::StartTls),
            (Pop3, 995, Tls::Implicit),
            (Pop3, 110, Tls::StartTls),
            (Smtp, 465, Tls::Implicit),
            (Smtp, 587, Tls::StartTls),
        ] {
            assert_eq!(check_port(protocol, port), Ok(tls), "{protocol:?} {port}");
        }
        for (protocol, port) in [(Imap, 0), (Imap, 25), (Imap, 1143), (Imap, 465), (Pop3, 993), (Smtp, 25), (Smtp, 2525), (Smtp, 1025), (Smtp, 143), (Imap, 443)] {
            assert_eq!(check_port(protocol, port), Err(Error::Port(port)), "{protocol:?} {port}");
        }
    }

    #[test]
    fn a_server_host_is_a_bare_name_or_address() {
        let server = |host: &str| Server { host: host.into(), port: 993 };
        assert_eq!(server("imap.mail.me.com").check(Protocol::Imap), Ok(Tls::Implicit));
        assert_eq!(server("192.0.2.7").check(Protocol::Imap), Ok(Tls::Implicit));
        for bad in ["", "evil.com/x", "user@evil.com", "imaps://evil.com", "a b", "evil.com:25", "[::1]"] {
            assert!(server(bad).check(Protocol::Imap).is_err(), "{bad:?}");
        }
        assert_eq!(Server { host: "imap.example.com".into(), port: 1143 }.check(Protocol::Imap), Err(Error::Port(1143)));
    }
}

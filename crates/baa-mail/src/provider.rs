//! Presets: the servers and sign-in of the providers the setup screen names.

use serde::{Deserialize, Serialize};

use crate::account::{Server, Servers, SignIn};

/// No Proton (its Bridge's ports are not allowed yet) and no Microsoft (Graph, stage 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    ICloud,
    /// Gmail and Google Workspace: label folders, All Mail, `X-GM-THRID`, and it files sent
    /// mail itself.
    Gmail,
    Fastmail,
    Yahoo,
    /// Any IMAP server; the user gives the servers.
    Other,
}

impl Provider {
    /// The preset a consumer address belongs to. A Google Workspace or Fastmail address on
    /// its own domain is `Other` here; the setup screen lets the user pick the preset.
    pub fn of(address: &str) -> Provider {
        let domain = address.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase()).unwrap_or_default();
        match domain.as_str() {
            "icloud.com" | "me.com" | "mac.com" => Provider::ICloud,
            "gmail.com" | "googlemail.com" => Provider::Gmail,
            "fastmail.com" | "fastmail.fm" => Provider::Fastmail,
            "yahoo.com" | "ymail.com" | "rocketmail.com" => Provider::Yahoo,
            _ => Provider::Other,
        }
    }

    /// How the setup screen asks for the sign-in.
    pub fn sign_in(self) -> SignIn {
        match self {
            Provider::Gmail => SignIn::Google,
            _ => SignIn::Password,
        }
    }

    /// The preset's servers for `address`; `None` for [`Provider::Other`].
    pub fn servers(self, address: &str) -> Option<Servers> {
        let (imap, smtp) = match self {
            Provider::ICloud => (("imap.mail.me.com", 993), ("smtp.mail.me.com", 587)),
            Provider::Gmail => (("imap.gmail.com", 993), ("smtp.gmail.com", 465)),
            Provider::Fastmail => (("imap.fastmail.com", 993), ("smtp.fastmail.com", 465)),
            Provider::Yahoo => (("imap.mail.yahoo.com", 993), ("smtp.mail.yahoo.com", 465)),
            Provider::Other => return None,
        };
        let server = |(host, port): (&str, u16)| Server { host: host.into(), port };
        Some(Servers { imap: server(imap), smtp: server(smtp), login: address.into() })
    }

    /// Gmail files a sent message in Sent itself; appending it too would show it twice.
    pub(crate) fn files_sent_mail(self) -> bool {
        self == Provider::Gmail
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Protocol;

    #[test]
    fn every_preset_uses_allowed_ports_and_an_address_finds_its_preset() {
        for provider in [Provider::ICloud, Provider::Gmail, Provider::Fastmail, Provider::Yahoo] {
            let servers = provider.servers("dan@example.com").unwrap();
            assert!(servers.imap.check(Protocol::Imap).is_ok() && servers.smtp.check(Protocol::Smtp).is_ok(), "{provider:?}");
            assert_eq!(servers.login, "dan@example.com");
        }
        assert_eq!(Provider::Other.servers("dan@example.com"), None);
        assert_eq!(Provider::of("Dan@GMail.com"), Provider::Gmail);
        assert_eq!(Provider::of("dan@me.com"), Provider::ICloud);
        assert_eq!(Provider::of("dan@example.com"), Provider::Other);
        assert_eq!(Provider::of("not an address"), Provider::Other);
        assert_eq!((Provider::Gmail.sign_in(), Provider::ICloud.sign_in()), (SignIn::Google, SignIn::Password));
    }
}

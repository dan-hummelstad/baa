use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    account::{Protocol, Server, Tls},
    Error, Result,
};

/// How curl signs in to the SMTP server. No `Debug`: it holds the secret.
pub(crate) enum Login<'a> {
    Password { user: &'a str, password: &'a str },
    /// An OAuth access token, sent by XOAUTH2.
    Bearer { user: &'a str, token: &'a str },
}

/// Sends `raw` (an RFC 822 message) from `from` to every address of `to` with the system curl.
/// The message goes through a new 0600 file in `dir` (the mail directory), removed afterwards
/// whatever happens; the secret goes on curl's stdin, never argv. Blocks.
pub(crate) fn smtp(server: &Server, login: &Login, from: &str, to: &[String], raw: &[u8], dir: &Path) -> Result<()> {
    let file = PrivateFile::create(dir, raw)?;
    curl(server, login, Some((from, to, &file.path)))
}

/// Connects to the SMTP server, starts TLS and signs in as [`smtp`] does, then sends NOOP
/// and leaves: nothing is mailed. Blocks.
pub(crate) fn smtp_check(server: &Server, login: &Login) -> Result<()> {
    curl(server, login, None)
}

/// The sender, the recipients and the message's file.
type Mailing<'a> = (&'a str, &'a [String], &'a Path);

fn curl(server: &Server, login: &Login, mailing: Option<Mailing>) -> Result<()> {
    let config = curl_config(login)?;
    let mut child = Command::new("curl")
        .args(curl_args(server, mailing)?)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| Error::Other("curl is missing".into()))?;
    // A write error means curl already quit; its exit status tells why.
    let _ = child.stdin.take().expect("piped").write_all(config.as_bytes());
    let out = child.wait_with_output().map_err(|e| Error::Other(format!("Sending failed: {e}")))?;
    match out.status.code() {
        _ if out.status.success() => Ok(()),
        Some(67) => Err(Error::SignIn),
        Some(35 | 60 | 64) => Err(Error::NoTls),
        // Could not resolve, could not connect, timed out: the outbox tries again later.
        Some(6 | 7 | 28) => Err(Error::Offline),
        code => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let line = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
            log::debug!("smtp: curl exited {code:?}: {line}");
            Err(Error::Other(format!("Sending failed: {line}")))
        }
    }
}

/// curl's argv: a send, or with no `mailing` a NOOP after the sign-in. Holds no secret.
fn curl_args(server: &Server, mailing: Option<Mailing>) -> Result<Vec<String>> {
    let mut args: Vec<String> = ["-sS", "--ssl-reqd", "--proto", "=smtp,smtps", "--proto-redir", "=smtp,smtps"].map(String::from).into();
    let url = match server.check(Protocol::Smtp)? {
        Tls::Implicit => format!("smtps://{}:{}", server.host, server.port),
        Tls::StartTls => format!("smtp://{}:{}", server.host, server.port),
    };
    let Some((from, to, file)) = mailing else {
        args.extend(["--max-time", "60", "--request", "NOOP", "-K", "-", "--url"].map(String::from));
        args.push(url);
        return Ok(args);
    };
    for addr in std::iter::once(from).chain(to.iter().map(String::as_str)) {
        check_address(addr)?;
    }
    if to.is_empty() {
        return Err(Error::Other("No recipient".into()));
    }
    args.extend(["--max-time", "120", "--mail-from", from].map(String::from));
    for addr in to {
        args.extend(["--mail-rcpt".to_string(), addr.clone()]);
    }
    args.extend(["--upload-file".to_string(), file.to_string_lossy().into_owned()]);
    args.extend(["-K", "-", "--url"].map(String::from));
    args.push(url);
    Ok(args)
}

/// curl's config, read from stdin (`-K -`): the sign-in.
fn curl_config(login: &Login) -> Result<String> {
    let quote = |v: &str| -> Result<String> {
        if v.contains(['\r', '\n', '\0']) {
            return Err(Error::Other("The sign-in holds a character curl cannot take".into()));
        }
        Ok(v.replace('\\', "\\\\").replace('"', "\\\""))
    };
    Ok(match login {
        Login::Password { user, password } => format!("user = \"{}:{}\"\n", quote(user)?, quote(password)?),
        Login::Bearer { user, token } => format!("user = \"{}\"\noauth2-bearer = \"{}\"\n", quote(user)?, quote(token)?),
    })
}

// SMTP commands are lines: anything that could end one or open an angle-bracket path is refused.
fn check_address(addr: &str) -> Result<()> {
    let bad = addr.is_empty()
        || !addr.contains('@')
        || addr.chars().any(|c| c.is_ascii_control() || c.is_whitespace() || matches!(c, '<' | '>' | ',' | '"'));
    if bad {
        return Err(Error::Other(format!("{addr} is not an address")));
    }
    Ok(())
}

struct PrivateFile {
    path: PathBuf,
}

impl PrivateFile {
    fn create(dir: &Path, contents: &[u8]) -> Result<Self> {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("outbox-{}-{n}.eml", std::process::id()));
        let io = |e: std::io::Error| Error::Other(format!("Sending failed: {e}"));
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).map_err(io)?;
        let this = Self { path };
        file.write_all(contents).map_err(io)?;
        Ok(this)
    }
}

impl Drop for PrivateFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(port: u16) -> Server {
        Server { host: "smtp.example.com".into(), port }
    }

    fn to() -> Vec<String> {
        vec!["a@example.com".into(), "b@example.com".into()]
    }

    #[test]
    fn smtp_args_require_tls_name_mail_schemes_only_and_hold_no_secret() {
        let login = Login::Password { user: "dan@example.com", password: "hunter2-secret" };
        for (port, url) in [(465, "smtps://smtp.example.com:465"), (587, "smtp://smtp.example.com:587")] {
            let args = curl_args(&server(port), Some(("dan@example.com", &to(), Path::new("/tmp/m.eml")))).unwrap();
            assert_eq!(args.last().unwrap(), url);
            assert!(args.contains(&"--ssl-reqd".to_string()));
            for flag in ["--proto", "--proto-redir"] {
                let i = args.iter().position(|a| a == flag).unwrap();
                assert_eq!(args[i + 1], "=smtp,smtps");
                assert_eq!(args.iter().filter(|a| *a == flag).count(), 1);
            }
            assert!(!args.iter().any(|a| a.contains("http")));
            for r in to() {
                let i = args.iter().position(|a| *a == r).unwrap();
                assert_eq!(args[i - 1], "--mail-rcpt");
            }
            let k = args.iter().position(|a| a == "-K").unwrap();
            assert_eq!(args[k + 1], "-");
            assert!(!args.iter().any(|a| a.contains("hunter2-secret")));
            assert!(curl_config(&login).unwrap().contains("hunter2-secret"));
        }
    }

    #[test]
    fn a_port_other_than_465_or_587_is_refused() {
        for port in [25, 2525] {
            assert!(matches!(curl_args(&server(port), Some(("a@b.com", &to(), Path::new("/x")))), Err(Error::Port(_))));
            assert!(matches!(curl_args(&server(port), None), Err(Error::Port(_))));
        }
    }

    #[test]
    fn the_check_requires_tls_signs_in_from_stdin_and_mails_nothing() {
        for (port, url) in [(465, "smtps://smtp.example.com:465"), (587, "smtp://smtp.example.com:587")] {
            let args = curl_args(&server(port), None).unwrap();
            assert_eq!(args.last().unwrap(), url);
            assert!(args.contains(&"--ssl-reqd".to_string()));
            for flag in ["--proto", "--proto-redir"] {
                let i = args.iter().position(|a| a == flag).unwrap();
                assert_eq!(args[i + 1], "=smtp,smtps");
            }
            let i = args.iter().position(|a| a == "--request").unwrap();
            assert_eq!(args[i + 1], "NOOP");
            let k = args.iter().position(|a| a == "-K").unwrap();
            assert_eq!(args[k + 1], "-");
            for mailing in ["--mail-from", "--mail-rcpt", "--upload-file", "-T"] {
                assert!(!args.iter().any(|a| a == mailing), "{mailing}");
            }
        }
    }

    #[test]
    fn an_address_that_could_inject_is_refused() {
        for bad in ["a@b.com\r\nRCPT TO:<x@y>", "a b@c.com", "<a@b.com>", "", "nobody"] {
            let as_from = curl_args(&server(587), Some((bad, &to(), Path::new("/x"))));
            let as_to = curl_args(&server(587), Some(("a@b.com", &[bad.to_string()], Path::new("/x"))));
            assert!(matches!(as_from, Err(Error::Other(_))), "{bad:?}");
            assert!(matches!(as_to, Err(Error::Other(_))), "{bad:?}");
        }
        assert!(curl_args(&server(587), Some(("a@b.com", &[], Path::new("/x")))).is_err());
    }

    #[test]
    fn the_config_escapes_quotes_and_refuses_line_breaks() {
        let config = curl_config(&Login::Password { user: "dan@example.com", password: r#"pa"ss\word"# }).unwrap();
        assert_eq!(config, "user = \"dan@example.com:pa\\\"ss\\\\word\"\n");
        let config = curl_config(&Login::Bearer { user: "dan@example.com", token: "tok" }).unwrap();
        assert_eq!(config, "user = \"dan@example.com\"\noauth2-bearer = \"tok\"\n");
        let err = curl_config(&Login::Password { user: "dan@example.com", password: "secret\nmore" }).err().unwrap();
        let Error::Other(text) = err else { panic!("not Other") };
        assert!(!text.contains("secret"));
    }

    #[test]
    fn the_message_file_is_private_and_removed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("baa-mail-send-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = PrivateFile::create(&dir, b"Subject: hi\r\n\r\nbody").unwrap();
        let path = file.path.clone();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::read(&path).unwrap(), b"Subject: hi\r\n\r\nbody");
        drop(file);
        assert!(!path.exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}

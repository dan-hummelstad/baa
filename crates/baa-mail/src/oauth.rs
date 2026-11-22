//! Google sign-in (PLAN-mail.md §4 "OAuth").

use std::io::Write;
use std::net::{Ipv4Addr, TcpListener};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ring::digest::{digest, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use url::form_urlencoded::Serializer;
use url::Url;

use crate::{Error, Result};

// Empty until the user registers baa's client (a Google "Desktop" client). The environment
// overrides them. Google's token endpoint wants the secret of a Desktop client too, and Google
// documents that it is not a secret in an installed app.
const GOOGLE_CLIENT_ID: &str = "";
const GOOGLE_CLIENT_SECRET: &str = "";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://mail.google.com/";

/// What a sign-in gives: the refresh token each connection gets its access token with. No
/// `Debug`: it is a secret.
pub struct Tokens {
    pub(crate) refresh: String,
}

struct Client {
    id: String,
    secret: String,
}

/// One Google sign-in under way. No `Debug`: it holds the PKCE verifier.
pub struct GoogleSignIn {
    url: String,
    verifier: String,
    state: String,
    redirect: String,
    listener: TcpListener,
    client: Client,
}

impl GoogleSignIn {
    /// Binds the loopback listener. [`Error::NotSetUp`] when this build has no Google client.
    pub fn start() -> Result<GoogleSignIn> {
        Self::start_with(client())
    }

    fn start_with(client: Option<Client>) -> Result<GoogleSignIn> {
        let client = client.ok_or(Error::NotSetUp)?;
        // 127.0.0.1 only: nothing else on the network may reach the redirect.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| Error::Other(format!("Could not listen for Google: {e}")))?;
        let port = listener.local_addr().map_err(|e| Error::Other(e.to_string()))?.port();
        let redirect = format!("http://127.0.0.1:{port}/");
        let verifier = random_token::<32>()?;
        let state = random_token::<16>()?;
        let url = Url::parse_with_params(
            AUTH_URL,
            [
                ("client_id", client.id.as_str()),
                ("redirect_uri", redirect.as_str()),
                ("response_type", "code"),
                ("scope", SCOPE),
                ("code_challenge", pkce_challenge(&verifier).as_str()),
                ("code_challenge_method", "S256"),
                ("state", state.as_str()),
                ("access_type", "offline"),
                ("prompt", "consent"),
            ],
        )
        .map_err(|e| Error::Other(e.to_string()))?
        .into();
        Ok(GoogleSignIn { url, verifier, state, redirect, listener, client })
    }

    /// Google's consent page, to open as a tab of the Space.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Blocks until Google sends the browser back, or 5 minutes pass, then exchanges the code
    /// for tokens. Call it off the UI thread.
    pub fn finish(self) -> Result<Tokens> {
        let code = listen(self.listener, &self.state, Instant::now() + Duration::from_secs(300))?;
        exchange(&self.client, &code, &self.verifier, &self.redirect)
    }
}

/// A fresh access token for a stored refresh token. A refresh token Google no longer takes
/// (`invalid_grant`) is [`Error::SignInAgain`].
pub(crate) fn refresh(refresh_token: &str) -> Result<String> {
    let client = client().ok_or(Error::NotSetUp)?;
    let body = Serializer::new(String::new())
        .append_pair("client_id", &client.id)
        .append_pair("client_secret", &client.secret)
        .append_pair("refresh_token", refresh_token)
        .append_pair("grant_type", "refresh_token")
        .finish();
    access_from(&post(TOKEN_URL, &body)?)
}

fn client() -> Option<Client> {
    let pick = |var: &str, constant: &str| match std::env::var(var) {
        Ok(v) if !v.is_empty() => v,
        _ => constant.to_string(),
    };
    let id = pick("BAA_GOOGLE_CLIENT_ID", GOOGLE_CLIENT_ID);
    (!id.is_empty()).then(|| Client { id, secret: pick("BAA_GOOGLE_CLIENT_SECRET", GOOGLE_CLIENT_SECRET) })
}

fn random_token<const N: usize>() -> Result<String> {
    let mut bytes = [0u8; N];
    SystemRandom::new().fill(&mut bytes).map_err(|_| Error::Other("No random numbers for the sign-in".into()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.as_bytes()))
}

/// Takes the first request from a loopback peer and closes the listener, so a second request
/// is refused. A request whose `state` is not this sign-in's ends the sign-in: it did not come
/// from Google's redirect.
fn listen(listener: TcpListener, state: &str, deadline: Instant) -> Result<String> {
    listener.set_nonblocking(true).map_err(|e| Error::Other(e.to_string()))?;
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok((stream, peer)) if loopback(peer) => break (stream, peer),
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(_) => return Err(Error::Other("Google sign-in took too long".into())),
        }
    };
    drop(listener);
    let answer = redirect_code(&mut stream, state);
    let page = if answer.is_ok() { "Signed in. You can close this tab." } else { "baa did not sign in." };
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n{page}");
    answer
}

fn redirect_code(stream: &mut std::net::TcpStream, state: &str) -> Result<String> {
    use std::io::Read;
    let refused = || Error::Other("Google did not send the sign-in back".into());
    stream.set_nonblocking(false).map_err(|_| refused())?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).map_err(|_| refused())?;
    let mut head = Vec::new();
    let mut chunk = [0; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 8192 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => head.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&head);
    let target = head.strip_prefix("GET ").and_then(|rest| rest.split(' ').next()).ok_or_else(refused)?;
    let url = Url::parse("http://127.0.0.1/").and_then(|base| base.join(target)).map_err(|_| refused())?;
    let param = |name: &str| url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned());
    if !same(param("state").unwrap_or_default().as_bytes(), state.as_bytes()) {
        return Err(refused());
    }
    match (param("code"), param("error")) {
        (Some(code), None) => Ok(code),
        (_, Some(error)) if error == "access_denied" => Err(Error::Other("Google sign-in was cancelled".into())),
        _ => Err(refused()),
    }
}

/// Compares in time that does not depend on where the first difference is.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

// A peer outside loopback cannot reach a 127.0.0.1 listener; checked anyway, in case the
// bind ever changes.
fn loopback(peer: std::net::SocketAddr) -> bool {
    peer.ip().is_loopback()
}

fn exchange(client: &Client, code: &str, verifier: &str, redirect: &str) -> Result<Tokens> {
    let body = Serializer::new(String::new())
        .append_pair("client_id", &client.id)
        .append_pair("client_secret", &client.secret)
        .append_pair("code", code)
        .append_pair("code_verifier", verifier)
        .append_pair("grant_type", "authorization_code")
        .append_pair("redirect_uri", redirect)
        .finish();
    tokens_from(&post(TOKEN_URL, &body)?)
}

// The body goes in on stdin so a code or token never shows in the process list. No `-f`:
// Google answers errors as JSON with status 400, and `error_from` reads it.
fn curl_args(url: &str) -> [&str; 13] {
    [
        "-sS", "--max-time", "30", "--proto", "=https", "--proto-redir", "=https", "--data-binary", "@-", "-H",
        "Content-Type: application/x-www-form-urlencoded", "--", url,
    ]
}

fn post(url: &str, body: &str) -> Result<serde_json::Value> {
    let unreachable = || Error::Other("Could not reach Google".into());
    let mut child = Command::new("curl")
        .args(curl_args(url))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| unreachable())?;
    child.stdin.take().expect("piped").write_all(body.as_bytes()).map_err(|_| unreachable())?;
    let out = child.wait_with_output().map_err(|_| unreachable())?;
    if !out.status.success() {
        return Err(unreachable());
    }
    serde_json::from_slice(&out.stdout).map_err(|_| Error::Other("Google gave an answer baa could not read".into()))
}

fn tokens_from(v: &serde_json::Value) -> Result<Tokens> {
    match v["refresh_token"].as_str() {
        Some(refresh) => Ok(Tokens { refresh: refresh.into() }),
        None => Err(error_from(v)),
    }
}

fn access_from(v: &serde_json::Value) -> Result<String> {
    v["access_token"].as_str().map(String::from).ok_or_else(|| error_from(v))
}

/// Only the error code goes into the text: it is not secret, and nothing else of the answer may be.
fn error_from(v: &serde_json::Value) -> Error {
    match v["error"].as_str() {
        Some("invalid_grant") => Error::SignInAgain,
        Some(error) => Error::Other(format!("Google sign-in failed: {error}")),
        None => Error::Other("Google gave no token".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_client() -> Client {
        Client { id: "id.apps.googleusercontent.com".into(), secret: "s".into() }
    }

    fn query(url: &str) -> std::collections::HashMap<String, String> {
        Url::parse(url).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
    }

    #[test]
    fn the_pkce_challenge_is_the_s256_of_the_verifier() {
        // RFC 7636 Appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn without_a_client_google_sign_in_says_it_is_not_set_up() {
        assert_eq!(GoogleSignIn::start_with(None).err(), Some(Error::NotSetUp));
    }

    #[test]
    fn the_consent_url_carries_pkce_state_and_a_loopback_redirect_but_not_the_verifier() {
        let a = GoogleSignIn::start_with(Some(test_client())).ok().unwrap();
        let q = query(a.url());
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], pkce_challenge(&a.verifier));
        assert_eq!(q["state"], a.state);
        assert!(a.state.len() >= 22);
        let addr = a.listener.local_addr().unwrap();
        assert_eq!(addr.ip(), std::net::IpAddr::from(Ipv4Addr::LOCALHOST));
        assert!(q["redirect_uri"].starts_with("http://127.0.0.1:"));
        assert_eq!(q["redirect_uri"], format!("http://127.0.0.1:{}/", addr.port()));
        assert_eq!(q["scope"], SCOPE);
        assert_eq!(q["client_id"], "id.apps.googleusercontent.com");
        assert!(!a.url().contains(&a.verifier));
        assert!(!a.url().contains("client_secret"));
        let b = GoogleSignIn::start_with(Some(test_client())).ok().unwrap();
        assert_ne!(a.state, b.state);
        assert_ne!(a.verifier, b.verifier);
    }

    #[test]
    fn token_answers_map_to_tokens_or_the_right_error() {
        let both = tokens_from(&json!({"access_token": "a", "refresh_token": "r", "expires_in": 3599})).ok().unwrap();
        assert_eq!(both.refresh, "r");
        assert!(tokens_from(&json!({"access_token": "a"})).is_err());
        let revoked = json!({"error": "invalid_grant", "error_description": "Token has been expired or revoked."});
        assert_eq!(access_from(&revoked), Err(Error::SignInAgain));
        match access_from(&json!({"error": "invalid_client"})) {
            Err(Error::Other(text)) => assert!(text.contains("invalid_client")),
            _ => panic!("expected Error::Other"),
        }
    }

    fn listening(state: &str) -> (std::net::SocketAddr, std::thread::JoinHandle<Result<String>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let state = state.to_string();
        (addr, std::thread::spawn(move || listen(listener, &state, Instant::now() + Duration::from_secs(10))))
    }

    fn get(addr: std::net::SocketAddr, target: &str) -> std::io::Result<String> {
        use std::io::Read;
        let mut stream = std::net::TcpStream::connect(addr)?;
        write!(stream, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")?;
        let mut answer = String::new();
        stream.read_to_string(&mut answer)?;
        Ok(answer)
    }

    #[test]
    fn the_listener_takes_one_request_with_the_right_state_and_refuses_the_rest() {
        let (addr, waiting) = listening("s3cr3t-state");
        assert!(get(addr, "/?state=s3cr3t-state&code=4%2F0Ab").unwrap().contains("Signed in"));
        assert_eq!(waiting.join().unwrap().ok().as_deref(), Some("4/0Ab"));
        assert!(get(addr, "/?state=s3cr3t-state&code=again").is_err(), "a second request finds nothing listening");

        let (addr, waiting) = listening("s3cr3t-state");
        assert!(get(addr, "/?state=guess&code=forged").unwrap().contains("did not sign in"));
        assert!(waiting.join().unwrap().is_err(), "a wrong state ends the sign-in");
        assert!(get(addr, "/?state=s3cr3t-state&code=late").is_err(), "and nothing listens after it");

        let (addr, waiting) = listening("s3cr3t-state");
        get(addr, "/?state=s3cr3t-state&error=access_denied").unwrap();
        assert_eq!(waiting.join().unwrap().err(), Some(Error::Other("Google sign-in was cancelled".into())));
    }

    #[test]
    fn the_listener_gives_up_at_its_deadline_and_trusts_only_loopback_peers() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        assert!(listen(listener, "s", Instant::now()).is_err());
        assert!(loopback("127.0.0.1:5000".parse().unwrap()) && loopback("[::1]:5000".parse().unwrap()));
        assert!(!loopback("192.168.1.20:5000".parse().unwrap()) && !loopback("0.0.0.0:5000".parse().unwrap()));
    }

    #[test]
    fn curl_args_hold_https_only_and_no_secret() {
        let args = curl_args("https://oauth2.googleapis.com/token");
        for flag in ["--proto", "--proto-redir"] {
            let at = args.iter().position(|a| *a == flag).unwrap();
            assert_eq!(args[at + 1], "=https");
        }
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(args[args.len() - 1], "https://oauth2.googleapis.com/token");
        assert!(args.contains(&"@-"));
    }
}

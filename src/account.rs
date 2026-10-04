//! Vrspi accounts in the CLI: `vrspi login`, `logout`, `account`, and the
//! sign-in check that runs before Vrspi starts.
//!
//! Sign-in uses the OAuth device flow against the Keycloak realm behind
//! vrspi.com: the terminal shows a short code and opens the browser on a link
//! that already carries it, so it works the same over SSH, on Windows, and on
//! a machine with no browser at all (open the link on a phone).
//!
//! The session is an offline refresh token kept in the config directory. It is
//! confirmed online at most every [`CONFIRM_EVERY_SECS`]; when the network is
//! unreachable Vrspi keeps starting for [`OFFLINE_GRACE_SECS`] after the last
//! confirmation. An account the server rejects, because it was deleted or
//! signed out elsewhere, is forgotten at once.
//!
//! This is an account requirement, not copy protection: the check runs on the
//! user's machine and can be bypassed by anyone who rebuilds the binary.

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// The Keycloak realm that owns Vrspi accounts.
pub(crate) const ISSUER: &str = "https://auth.vrspi.com/realms/vrspi";
/// The public client for this CLI; it has no secret to protect.
const CLIENT_ID: &str = "vrspi-cli";
/// `offline_access` makes the session outlive the browser session, so one
/// sign-in carries across restarts and short trips offline.
const SCOPE: &str = "openid offline_access email profile";
const SIGNUP_URL: &str = "https://vrspi.com/signup";

/// How often a running installation reconfirms its account online.
pub(crate) const CONFIRM_EVERY_SECS: u64 = 12 * 60 * 60;
/// How long Vrspi keeps starting without reaching the account server.
pub(crate) const OFFLINE_GRACE_SECS: u64 = 14 * 24 * 60 * 60;
const ACCOUNT_FILE: &str = "account.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StoredAccount {
    pub(crate) subject: String,
    pub(crate) email: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    pub(crate) refresh_token: String,
    /// Unix seconds of the last time the account server accepted this session.
    pub(crate) confirmed_at: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AccountError {
    /// The account server could not be reached, or answered with a server error.
    Network(String),
    /// The account server refused the session: deleted, revoked, or expired.
    Rejected(String),
    /// Anything else, such as an unreadable response or a local I/O failure.
    Other(String),
}

impl std::fmt::Display for AccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(message) => write!(f, "could not reach the account server: {message}"),
            Self::Rejected(message) => {
                write!(f, "the account server refused the session: {message}")
            }
            Self::Other(message) => f.write_str(message),
        }
    }
}

/// Token endpoint responses this module needs.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    refresh_token: Option<String>,
    id_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

/// A refreshed session token, and who it belongs to when the server says.
pub(crate) type RefreshResult = Result<(String, Option<Identity>), AccountError>;

/// The network side of the account flow, so the decisions can be tested offline.
pub(crate) trait AuthServer {
    fn refresh(&self, refresh_token: &str) -> RefreshResult;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    subject: String,
    email: String,
    name: Option<String>,
}

pub(crate) struct Keycloak;

impl AuthServer for Keycloak {
    fn refresh(&self, refresh_token: &str) -> RefreshResult {
        let body = post_form(
            &format!("{ISSUER}/protocol/openid-connect/token"),
            &[
                ("grant_type", "refresh_token"),
                ("client_id", CLIENT_ID),
                ("refresh_token", refresh_token),
            ],
        )?;
        let tokens: TokenResponse = serde_json::from_value(body)
            .map_err(|err| AccountError::Other(format!("unexpected token response: {err}")))?;
        let refresh = tokens
            .refresh_token
            .unwrap_or_else(|| refresh_token.to_string());
        Ok((
            refresh,
            tokens.id_token.as_deref().and_then(identity_from_id_token),
        ))
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn account_path() -> PathBuf {
    crate::config::config_dir().join(ACCOUNT_FILE)
}

pub(crate) fn load() -> Option<StoredAccount> {
    let text = fs::read_to_string(account_path()).ok()?;
    serde_json::from_str(&text).ok()
}

fn save(account: &StoredAccount) -> io::Result<()> {
    let path = account_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(account).map_err(io::Error::other)?;
    // Write a sibling and rename, so an interrupted save never leaves a
    // half-written session that would sign the user out.
    let tmp = path.with_extension("json.tmp");
    write_private(&tmp, json.as_bytes())?;
    fs::rename(&tmp, &path)
}

/// The refresh token is a credential; on Unix only the owner may read it.
/// Windows keeps the config directory under the user's own profile.
#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(path, bytes)
}

fn forget() {
    let _ = fs::remove_file(account_path());
}

/// What the launch check concluded.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    /// Signed in and confirmed recently enough; start normally.
    Allowed(StoredAccount),
    /// Signed in but the server was unreachable; still inside the grace period.
    AllowedOffline {
        account: StoredAccount,
        days_left: u64,
    },
    /// Nobody is signed in on this machine.
    SignInRequired,
    /// The server refused the session; the stored one has been discarded.
    SignInAgain,
    /// Offline for longer than the grace period.
    ConfirmOnline { days_offline: u64 },
}

/// Decides whether Vrspi may start, refreshing the session when it is due.
pub(crate) fn check(server: &impl AuthServer, stored: Option<StoredAccount>, now: u64) -> Gate {
    let Some(mut account) = stored else {
        return Gate::SignInRequired;
    };
    let age = now.saturating_sub(account.confirmed_at);
    if age < CONFIRM_EVERY_SECS {
        return Gate::Allowed(account);
    }
    match server.refresh(&account.refresh_token) {
        Ok((refresh_token, identity)) => {
            account.refresh_token = refresh_token;
            account.confirmed_at = now;
            if let Some(identity) = identity {
                account.subject = identity.subject;
                account.email = identity.email;
                account.name = identity.name;
            }
            Gate::Allowed(account)
        }
        Err(AccountError::Rejected(_)) => Gate::SignInAgain,
        Err(_) if age < OFFLINE_GRACE_SECS => Gate::AllowedOffline {
            days_left: (OFFLINE_GRACE_SECS - age).div_ceil(24 * 60 * 60),
            account,
        },
        Err(_) => Gate::ConfirmOnline {
            days_offline: age / (24 * 60 * 60),
        },
    }
}

/// Whether this build enforces sign-in. Debug builds skip it so tests and
/// local development never need an account, unless asked to check.
fn enforced() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("VRSPI_REQUIRE_ACCOUNT").is_some()
}

/// Runs before Vrspi starts. Returns only when the user may continue.
pub(crate) fn require_sign_in() {
    if !enforced() {
        return;
    }
    let gate = check(&Keycloak, load(), now_unix());
    match gate {
        Gate::Allowed(account) => persist_confirmation(&account),
        Gate::AllowedOffline { account, days_left } => {
            persist_confirmation(&account);
            if days_left <= 3 {
                eprintln!(
                    "{}: offline — {} can start without reaching auth.vrspi.com for {days_left} more day{}.",
                    crate::EXECUTABLE_NAME,
                    crate::brand::PRODUCT_NAME,
                    if days_left == 1 { "" } else { "s" }
                );
            }
        }
        Gate::SignInRequired | Gate::SignInAgain => {
            if matches!(gate, Gate::SignInAgain) {
                forget();
                eprintln!(
                    "Your {} session ended. Please sign in again.",
                    crate::brand::PRODUCT_NAME
                );
            }
            if !interactive() {
                eprintln!(
                    "{} needs a free account. Run `{} login` in a terminal first.",
                    crate::brand::PRODUCT_NAME,
                    crate::EXECUTABLE_NAME
                );
                std::process::exit(1);
            }
            eprintln!(
                "{} needs a free account to start.",
                crate::brand::PRODUCT_NAME
            );
            if let Err(err) = login() {
                eprintln!("Sign-in did not complete: {err}");
                std::process::exit(1);
            }
        }
        Gate::ConfirmOnline { days_offline } => {
            eprintln!(
                "{} could not confirm your account online for {days_offline} days. Connect to the internet and start it again.",
                crate::brand::PRODUCT_NAME
            );
            std::process::exit(1);
        }
    }
}

fn persist_confirmation(account: &StoredAccount) {
    if load().as_ref() != Some(account) {
        if let Err(err) = save(account) {
            tracing::warn!("could not save account confirmation: {err}");
        }
    }
}

fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

/// `vrspi login`: the device flow, ending with a stored session.
pub(crate) fn login() -> Result<StoredAccount, AccountError> {
    let start = post_form(
        &format!("{ISSUER}/protocol/openid-connect/auth/device"),
        &[("client_id", CLIENT_ID), ("scope", SCOPE)],
    )?;
    let device: DeviceAuthorization = serde_json::from_value(start)
        .map_err(|err| AccountError::Other(format!("unexpected sign-in response: {err}")))?;
    let link = device
        .verification_uri_complete
        .clone()
        .unwrap_or_else(|| device.verification_uri.clone());

    eprintln!();
    eprintln!(
        "  Sign in to {} in your browser:",
        crate::brand::PRODUCT_NAME
    );
    eprintln!("    {link}");
    eprintln!();
    eprintln!(
        "  Or open {} and enter the code  {}",
        device.verification_uri, device.user_code
    );
    eprintln!("  No account yet? Create one free at {SIGNUP_URL}");
    eprintln!();
    if crate::platform::open_url(&link).is_err() {
        tracing::debug!("could not open a browser for sign-in");
    }
    eprint!("  Waiting for you to approve in the browser… (Ctrl+C to cancel)");
    let _ = io::stderr().flush();

    let mut interval = Duration::from_secs(device.interval.unwrap_or(5).max(1));
    let deadline = Instant::now() + Duration::from_secs(device.expires_in);
    loop {
        std::thread::sleep(interval);
        if Instant::now() >= deadline {
            eprintln!();
            return Err(AccountError::Other(
                "the sign-in code expired; run the command again".into(),
            ));
        }
        let result = post_form(
            &format!("{ISSUER}/protocol/openid-connect/token"),
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", CLIENT_ID),
                ("device_code", &device.device_code),
            ],
        );
        match result {
            Ok(body) => {
                eprintln!();
                let tokens: TokenResponse = serde_json::from_value(body).map_err(|err| {
                    AccountError::Other(format!("unexpected token response: {err}"))
                })?;
                let refresh_token = tokens.refresh_token.ok_or_else(|| {
                    AccountError::Other("the account server returned no session".into())
                })?;
                let identity = tokens
                    .id_token
                    .as_deref()
                    .and_then(identity_from_id_token)
                    .ok_or_else(|| {
                        AccountError::Other("the account server returned no identity".into())
                    })?;
                let previous = load();
                let account = StoredAccount {
                    subject: identity.subject,
                    email: identity.email,
                    name: identity.name,
                    refresh_token,
                    confirmed_at: now_unix(),
                };
                save(&account).map_err(|err| {
                    AccountError::Other(format!("could not save the session: {err}"))
                })?;
                if let Some(previous) =
                    previous.filter(|p| p.refresh_token != account.refresh_token)
                {
                    revoke(&previous.refresh_token);
                }
                eprintln!("  ✓ Signed in as {}", account.email);
                eprintln!();
                return Ok(account);
            }
            Err(AccountError::Rejected(code)) if code == "authorization_pending" => {}
            Err(AccountError::Rejected(code)) if code == "slow_down" => {
                interval += Duration::from_secs(5);
            }
            Err(AccountError::Rejected(code)) if code == "access_denied" => {
                eprintln!();
                return Err(AccountError::Rejected(
                    "sign-in was declined in the browser".into(),
                ));
            }
            Err(AccountError::Rejected(code)) if code == "expired_token" => {
                eprintln!();
                return Err(AccountError::Other(
                    "the sign-in code expired; run the command again".into(),
                ));
            }
            // A dropped connection while waiting is not fatal; keep polling.
            Err(AccountError::Network(_)) => {}
            Err(other) => {
                eprintln!();
                return Err(other);
            }
        }
    }
}

/// `vrspi logout`: ends the session on the server too, then forgets it.
pub(crate) fn logout() -> Option<StoredAccount> {
    let account = load()?;
    revoke(&account.refresh_token);
    forget();
    Some(account)
}

fn revoke(refresh_token: &str) {
    if let Err(err) = post_form(
        &format!("{ISSUER}/protocol/openid-connect/revoke"),
        &[
            ("client_id", CLIENT_ID),
            ("token", refresh_token),
            ("token_type_hint", "refresh_token"),
        ],
    ) {
        tracing::debug!("could not revoke the session on the server: {err}");
    }
}

/// Handles `login`, `logout`, and `account`. Returns `None` for anything else.
pub(crate) fn run_command(args: &[String]) -> Option<i32> {
    let command = args.get(1)?.as_str();
    if !matches!(command, "login" | "logout" | "account") {
        return None;
    }
    crate::platform::begin_cli_output();
    if args.iter().skip(2).any(|a| a == "--help" || a == "-h") {
        println!("usage: {} login | logout | account", crate::EXECUTABLE_NAME);
        println!();
        println!("  login    Sign in with your vrspi.com account (opens your browser)");
        println!("  logout   Sign out on this machine and end the session");
        println!("  account  Show who is signed in");
        return Some(0);
    }
    let code = match command {
        "login" => match login() {
            Ok(_) => 0,
            Err(err) => {
                eprintln!("Sign-in did not complete: {err}");
                1
            }
        },
        "logout" => {
            match logout() {
                Some(account) => println!("Signed out {}.", account.email),
                None => println!("Nobody is signed in on this machine."),
            }
            0
        }
        _ => {
            match load() {
                Some(account) => {
                    let name = account.name.as_deref().map(|n| format!("{n} ")).unwrap_or_default();
                    println!("Signed in as {name}<{}>", account.email);
                    println!("Last confirmed {}", describe_age(now_unix().saturating_sub(account.confirmed_at)));
                    println!("Manage your account at https://vrspi.com/account");
                }
                None => println!(
                    "Nobody is signed in. Run `{} login`, or create a free account at {SIGNUP_URL}.",
                    crate::EXECUTABLE_NAME
                ),
            }
            0
        }
    };
    Some(code)
}

fn describe_age(seconds: u64) -> String {
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} minutes ago", seconds / 60),
        3600..=86_399 => format!("{} hours ago", seconds / 3600),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

/// Reads who signed in from the ID token. It came straight from the token
/// endpoint over TLS, so its claims are taken as sent; nothing here grants
/// access on the strength of them.
fn identity_from_id_token(id_token: &str) -> Option<Identity> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Some(Identity {
        subject: claims.get("sub")?.as_str()?.to_string(),
        email: claims
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        name: claims
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

fn form_encode(pairs: &[(&str, &str)]) -> String {
    fn encode(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for byte in value.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(byte as char)
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{byte:02X}")),
            }
        }
        out
    }
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// POSTs a form and returns the JSON body. The body goes through stdin so
/// tokens never appear in the process list.
fn post_form(url: &str, pairs: &[(&str, &str)]) -> Result<serde_json::Value, AccountError> {
    let mut child = crate::noninteractive_process::curl_command()
        .args([
            "-sS",
            "--connect-timeout",
            "8",
            "--max-time",
            "15",
            "-H",
            "Content-Type: application/x-www-form-urlencoded",
            "-H",
            "Accept: application/json",
            "--data-binary",
            "@-",
            "-w",
            "\n%{http_code}",
            url,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| AccountError::Network(format!("curl could not start: {err}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(form_encode(pairs).as_bytes())
            .map_err(|err| AccountError::Network(err.to_string()))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|err| AccountError::Network(err.to_string()))?;
    if !output.status.success() {
        return Err(AccountError::Network(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, status) = text.rsplit_once('\n').unwrap_or(("", text.as_ref()));
    let status: u16 = status.trim().parse().unwrap_or(0);
    let json: serde_json::Value = if body.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(body)
            .map_err(|_| AccountError::Other(format!("unexpected response ({status})")))?
    };
    match status {
        200..=299 => Ok(json),
        400..=499 => Err(AccountError::Rejected(
            json.get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("rejected")
                .to_string(),
        )),
        _ => Err(AccountError::Network(format!(
            "account server answered {status}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeServer {
        answer: RefCell<Option<RefreshResult>>,
        calls: RefCell<u32>,
    }

    impl FakeServer {
        fn answering(answer: RefreshResult) -> Self {
            Self {
                answer: RefCell::new(Some(answer)),
                calls: RefCell::new(0),
            }
        }
    }

    impl AuthServer for FakeServer {
        fn refresh(&self, _: &str) -> RefreshResult {
            *self.calls.borrow_mut() += 1;
            self.answer
                .borrow_mut()
                .take()
                .expect("refresh called once")
        }
    }

    const NOW: u64 = 2_000_000_000;
    const DAY: u64 = 24 * 60 * 60;

    fn account(confirmed_at: u64) -> StoredAccount {
        StoredAccount {
            subject: "user-1".into(),
            email: "ada@example.com".into(),
            name: Some("Ada".into()),
            refresh_token: "old-token".into(),
            confirmed_at,
        }
    }

    #[test]
    fn nobody_signed_in_requires_sign_in_without_calling_the_server() {
        let server = FakeServer::answering(Err(AccountError::Network("unused".into())));
        assert_eq!(check(&server, None, NOW), Gate::SignInRequired);
        assert_eq!(*server.calls.borrow(), 0);
    }

    #[test]
    fn a_recent_confirmation_starts_without_a_network_round_trip() {
        let server = FakeServer::answering(Err(AccountError::Network("unused".into())));
        let stored = account(NOW - 60);
        assert_eq!(
            check(&server, Some(stored.clone()), NOW),
            Gate::Allowed(stored)
        );
        assert_eq!(*server.calls.borrow(), 0);
    }

    #[test]
    fn a_due_confirmation_refreshes_and_keeps_the_rotated_token() {
        let identity = Identity {
            subject: "user-1".into(),
            email: "new@example.com".into(),
            name: None,
        };
        let server = FakeServer::answering(Ok(("rotated".into(), Some(identity))));
        let Gate::Allowed(updated) = check(&server, Some(account(NOW - 2 * DAY)), NOW) else {
            panic!("expected allowed");
        };
        assert_eq!(updated.refresh_token, "rotated");
        assert_eq!(updated.confirmed_at, NOW);
        assert_eq!(updated.email, "new@example.com");
    }

    #[test]
    fn a_rejected_session_must_sign_in_again_even_inside_the_grace_period() {
        let server = FakeServer::answering(Err(AccountError::Rejected("invalid_grant".into())));
        assert_eq!(
            check(&server, Some(account(NOW - DAY)), NOW),
            Gate::SignInAgain
        );
    }

    #[test]
    fn offline_inside_the_grace_period_still_starts_and_counts_down() {
        let server = FakeServer::answering(Err(AccountError::Network("offline".into())));
        let stored = account(NOW - 12 * DAY - 60);
        assert_eq!(
            check(&server, Some(stored.clone()), NOW),
            Gate::AllowedOffline {
                account: stored,
                days_left: 2
            }
        );
    }

    #[test]
    fn offline_past_the_grace_period_must_confirm_online() {
        let server = FakeServer::answering(Err(AccountError::Network("offline".into())));
        assert_eq!(
            check(&server, Some(account(NOW - 15 * DAY)), NOW),
            Gate::ConfirmOnline { days_offline: 15 }
        );
    }

    #[test]
    fn form_encoding_escapes_reserved_characters() {
        assert_eq!(
            form_encode(&[("scope", "openid offline_access"), ("token", "a+b/c=&d")]),
            "scope=openid+offline_access&token=a%2Bb%2Fc%3D%26d"
        );
    }

    #[test]
    fn identity_is_read_from_the_id_token_claims() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(r#"{"sub":"user-1","email":"ada@example.com","name":"Ada Lovelace"}"#);
        let identity =
            identity_from_id_token(&format!("header.{payload}.signature")).expect("identity");
        assert_eq!(identity.subject, "user-1");
        assert_eq!(identity.email, "ada@example.com");
        assert_eq!(identity.name.as_deref(), Some("Ada Lovelace"));
        assert!(identity_from_id_token("not-a-token").is_none());
    }
}

//! `boswell login` — the OAuth 2.0 device authorization grant (RFC 8628).
//!
//! The client half of ADR-022's OIDC story. #68 taught the gateway to *verify*
//! a provider's token; nothing in the repo *obtained* one, so an operator had
//! to paste a token by hand. This runs the grant.
//!
//! Boswell is not in the grant. The exchange is between this CLI and the
//! identity provider the operator already runs — the gateway never sees the
//! device code, and it learns nothing from a login that it does not learn again
//! from the first request carrying the token. That is the whole reason ADR-022
//! declined to put Boswell in the identity business: the provider issues, the
//! gateway verifies, and neither needs the other's cooperation to do it.

use crate::cli::LoginArgs;
use crate::config::Config;
use crate::error::{CliError, Result};
use crate::output::Formatter;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, Instant};

/// The grant type identifier, spelled out by RFC 8628 §3.4.
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Poll interval to use when the provider does not name one (RFC 8628 §3.2).
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;

/// How much a `slow_down` adds to the interval. RFC 8628 §3.5 specifies five
/// seconds; it is not a suggestion, and providers rate-limit against it.
const SLOW_DOWN_INCREMENT_SECS: u64 = 5;

/// The provider metadata this command needs.
///
/// Every field is optional at the serde level so a provider that publishes a
/// discovery document without device-grant support fails with a sentence about
/// the missing endpoint rather than a serde error about a missing field.
#[derive(Debug, Default, Deserialize)]
struct Discovery {
    #[serde(default)]
    device_authorization_endpoint: String,
    #[serde(default)]
    token_endpoint: String,
}

/// The provider's answer to the device authorization request (RFC 8628 §3.2).
#[derive(Debug, Deserialize)]
struct DeviceAuthorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
    /// The verification URI with the user code already in it. Optional, and
    /// worth preferring when present — it saves the operator typing the code.
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
}

/// A successful token response (RFC 6749 §5.1).
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default = "default_token_type")]
    token_type: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
}

fn default_token_type() -> String {
    "Bearer".to_string()
}

/// An error response from the token endpoint (RFC 6749 §5.2).
#[derive(Debug, Deserialize)]
struct TokenErrorBody {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// The token as it is written to disk.
///
/// **No refresh token.** The provider may well issue one, and it is discarded
/// rather than stored: nothing in Boswell refreshes, so keeping it would put a
/// long-lived credential on disk to serve a code path that does not exist. JWT
/// refresh is one of the open questions in `10-security.md`; when it is
/// answered, this struct grows a field and the answer decides its lifetime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredToken {
    /// The issuer the token came from. Recorded so `--status` can say which
    /// provider is logged in, and so a token from a retired issuer is
    /// recognizable rather than merely mysterious.
    pub issuer: String,
    /// The bearer token itself.
    pub access_token: String,
    /// Almost always `Bearer`.
    pub token_type: String,
    /// Unix seconds. Absent when the provider did not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// The scope the provider actually granted, which need not be what was
    /// asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl StoredToken {
    /// Whether the token is past its expiry, as of `now` in unix seconds.
    ///
    /// A token with no stated expiry is never reported as expired — the CLI
    /// does not know better than the provider.
    pub fn is_expired_at(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|exp| now >= exp)
    }
}

/// Current unix time in seconds.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What one poll of the token endpoint means for the loop (RFC 8628 §3.5).
#[derive(Debug)]
enum PollStep {
    /// Nobody has approved it yet. Wait and ask again at the same interval.
    Pending,
    /// We are asking too often. Wait longer, then ask again.
    SlowDown,
    /// The grant completed.
    Done(Box<TokenResponse>),
    /// Terminal. The user declined, the code expired, or the provider refused
    /// the request outright.
    Failed(String),
}

/// Classify a token-endpoint error code.
///
/// `authorization_pending` and `slow_down` are the only two codes that mean
/// "keep going"; RFC 8628 §3.5 is explicit that every other code is terminal.
/// Treating an unrecognized code as terminal is the safe direction — the
/// alternative polls a provider forever over a request it has already refused.
fn classify(code: &str, description: Option<&str>) -> PollStep {
    match code {
        "authorization_pending" => PollStep::Pending,
        "slow_down" => PollStep::SlowDown,
        "access_denied" => PollStep::Failed("the request was declined at the provider".into()),
        "expired_token" => {
            PollStep::Failed("the device code expired before it was approved".into())
        }
        other => {
            let detail = description.unwrap_or("no description given");
            PollStep::Failed(format!("{}: {}", other, detail))
        }
    }
}

/// Drive the polling loop to a token, a refusal, or the expiry deadline.
///
/// Split from the HTTP so the state machine — the part with the interesting
/// failure modes — is testable without a provider. `poll` is called once per
/// interval; the loop owns the waiting, the back-off and the deadline.
///
/// The first wait happens *before* the first poll. The user has just been shown
/// a code they have not typed yet, so an immediate poll can only ever return
/// `authorization_pending`, and some providers count it against the rate limit.
async fn poll_for_token<F, Fut>(
    mut poll: F,
    interval: Duration,
    expires_in: Duration,
) -> Result<TokenResponse>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<PollStep>>,
{
    let deadline = Instant::now() + expires_in;
    let mut interval = interval;

    loop {
        if Instant::now() >= deadline {
            return Err(CliError::NotPermitted(
                "the device code expired before it was approved".into(),
            ));
        }
        sleep(interval).await;

        match poll().await? {
            PollStep::Done(token) => return Ok(*token),
            PollStep::Pending => {}
            PollStep::SlowDown => {
                interval += Duration::from_secs(SLOW_DOWN_INCREMENT_SECS);
            }
            PollStep::Failed(why) => return Err(CliError::NotPermitted(why)),
        }
    }
}

/// Strip a trailing slash so `{issuer}/.well-known/...` never doubles it.
/// Matches what the gateway does with the same value.
fn normalize_issuer(issuer: &str) -> &str {
    issuer.trim().trim_end_matches('/')
}

/// Fetch the provider's discovery document and pull out the two endpoints.
async fn discover(http: &reqwest::Client, issuer: &str) -> Result<Discovery> {
    let url = format!(
        "{}/.well-known/openid-configuration",
        normalize_issuer(issuer)
    );
    let discovery: Discovery = http
        .get(&url)
        .send()
        .await
        .map_err(|e| CliError::Connection(format!("{}: {}", url, e)))?
        .error_for_status()
        .map_err(|e| CliError::Connection(format!("{}: {}", url, e)))?
        .json()
        .await
        .map_err(|e| CliError::Connection(format!("{}: {}", url, e)))?;

    if discovery.device_authorization_endpoint.is_empty() {
        return Err(CliError::NotPermitted(format!(
            "{} publishes no device_authorization_endpoint, so it does not support the device grant",
            normalize_issuer(issuer)
        )));
    }
    if discovery.token_endpoint.is_empty() {
        return Err(CliError::Connection(format!(
            "{} publishes no token_endpoint",
            normalize_issuer(issuer)
        )));
    }
    Ok(discovery)
}

/// Ask the provider for a device code and user code.
async fn request_device_code(
    http: &reqwest::Client,
    endpoint: &str,
    client_id: &str,
    scope: &str,
) -> Result<DeviceAuthorization> {
    let mut form = vec![("client_id", client_id)];
    if !scope.is_empty() {
        form.push(("scope", scope));
    }

    let response = http
        .post(endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|e| CliError::Connection(format!("{}: {}", endpoint, e)))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| CliError::Connection(format!("{}: {}", endpoint, e)))?;

    if !status.is_success() {
        // A device-authorization failure is nearly always a bad client id, and
        // the provider says so in the body. Surfacing it beats "HTTP 400".
        let detail = serde_json::from_str::<TokenErrorBody>(&body)
            .map(|e| match e.error_description {
                Some(d) => format!("{}: {}", e.error, d),
                None => e.error,
            })
            .unwrap_or_else(|_| format!("HTTP {}", status));
        return Err(CliError::NotPermitted(format!("{}: {}", endpoint, detail)));
    }

    serde_json::from_str(&body).map_err(|e| {
        CliError::Connection(format!(
            "{}: unreadable device authorization: {}",
            endpoint, e
        ))
    })
}

/// One poll of the token endpoint, classified for the loop.
async fn poll_once(
    http: &reqwest::Client,
    endpoint: &str,
    client_id: &str,
    device_code: &str,
) -> Result<PollStep> {
    let response = http
        .post(endpoint)
        .form(&[
            ("grant_type", DEVICE_GRANT_TYPE),
            ("device_code", device_code),
            ("client_id", client_id),
        ])
        .send()
        .await
        .map_err(|e| CliError::Connection(format!("{}: {}", endpoint, e)))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| CliError::Connection(format!("{}: {}", endpoint, e)))?;

    if status.is_success() {
        let token: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| CliError::Connection(format!("{}: unreadable token: {}", endpoint, e)))?;
        return Ok(PollStep::Done(Box::new(token)));
    }

    match serde_json::from_str::<TokenErrorBody>(&body) {
        Ok(err) => Ok(classify(&err.error, err.error_description.as_deref())),
        // A non-OAuth error body means something other than the provider
        // answered — a proxy, a login page, an outage. Terminal, not pending.
        Err(_) => Ok(PollStep::Failed(format!(
            "HTTP {} from {}",
            status, endpoint
        ))),
    }
}

/// Write the token to `token.json` in the config directory.
///
/// Opened `0600` rather than written and then chmod-ed: the second form leaves
/// the token world-readable for the width of one syscall, which is exactly the
/// window a shared host has someone watching.
fn save_token(token: &StoredToken) -> Result<()> {
    let path = Config::token_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(token)?;

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(contents.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, contents)?;
    }

    Ok(())
}

/// Read the stored token, if there is one.
pub fn load_token() -> Result<Option<StoredToken>> {
    let path = Config::token_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path)?;
    Ok(Some(serde_json::from_str(&contents)?))
}

/// Delete the stored token. Reports whether there was one to delete.
fn delete_token() -> Result<bool> {
    let path = Config::token_path()?;
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&path)?;
    Ok(true)
}

/// Where the issuer and client id come from: the flags, else the config.
fn resolve_provider(args: &LoginArgs, config: &Config) -> Result<(String, String, String)> {
    let configured = config.oidc.as_ref();

    let issuer = args
        .issuer
        .clone()
        .or_else(|| configured.map(|c| c.issuer.clone()))
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            CliError::Config(
                "no issuer: pass --issuer, or set issuer under [oidc] in the config".into(),
            )
        })?;

    let client_id = args
        .client_id
        .clone()
        .or_else(|| configured.map(|c| c.client_id.clone()))
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            CliError::Config(
                "no client id: pass --client-id, or set client_id under [oidc] in the config"
                    .into(),
            )
        })?;

    // Flags replace the configured scopes rather than adding to them. Adding
    // would make it impossible to ask for fewer scopes than the config names,
    // and asking for fewer is the direction that matters.
    let scopes = if !args.scope.is_empty() {
        args.scope.clone()
    } else {
        configured.map(|c| c.scopes.clone()).unwrap_or_default()
    };

    Ok((issuer, client_id, scopes.join(" ")))
}

/// Describe the stored token without printing it.
fn report_status(formatter: &Formatter) -> Result<()> {
    match load_token()? {
        None => println!(
            "{}",
            formatter.info("No stored token. Run `boswell login`.")
        ),
        Some(token) => {
            println!("Issuer:     {}", token.issuer);
            println!("Token type: {}", token.token_type);
            if let Some(scope) = &token.scope {
                println!("Scope:      {}", scope);
            }
            match token.expires_at {
                None => println!("Expires:    not stated by the provider"),
                Some(exp) if token.is_expired_at(unix_now()) => {
                    println!(
                        "{}",
                        formatter.warning(&format!("Expired at {} (unix)", exp))
                    );
                }
                Some(exp) => {
                    let remaining = exp.saturating_sub(unix_now());
                    println!("Expires:    {} (unix), in {}s", exp, remaining);
                }
            }
            println!("Stored at:  {}", Config::token_path()?.display());
        }
    }
    Ok(())
}

/// Execute the login command.
pub async fn execute_login(args: LoginArgs, config: &Config, formatter: &Formatter) -> Result<()> {
    if args.status {
        return report_status(formatter);
    }

    let (issuer, client_id, scope) = resolve_provider(&args, config)?;
    let http = reqwest::Client::new();

    let discovery = discover(&http, &issuer).await?;
    let auth = request_device_code(
        &http,
        &discovery.device_authorization_endpoint,
        &client_id,
        &scope,
    )
    .await?;

    // Printed, not opened in a browser. The CLI runs over SSH and inside
    // containers as often as it runs on a desktop, and a command that silently
    // launches a browser somewhere the operator is not looking is worse than
    // one that always asks them to click.
    match &auth.verification_uri_complete {
        Some(complete) => {
            println!("Open: {}", complete);
            println!("Code: {} (already in the link above)", auth.user_code);
        }
        None => {
            println!("Open: {}", auth.verification_uri);
            println!("Code: {}", auth.user_code);
        }
    }
    println!(
        "{}",
        formatter.info("Waiting for approval. Ctrl-C to give up.")
    );

    let interval = Duration::from_secs(auth.interval.unwrap_or(DEFAULT_POLL_INTERVAL_SECS));
    let token = poll_for_token(
        || {
            poll_once(
                &http,
                &discovery.token_endpoint,
                &client_id,
                &auth.device_code,
            )
        },
        interval,
        Duration::from_secs(auth.expires_in),
    )
    .await?;

    let stored = StoredToken {
        issuer: normalize_issuer(&issuer).to_string(),
        expires_at: token.expires_in.map(|secs| unix_now() + secs),
        access_token: token.access_token,
        token_type: token.token_type,
        scope: token.scope,
    };
    save_token(&stored)?;

    println!(
        "{}",
        formatter.success(&format!("Logged in to {}", stored.issuer))
    );
    println!("Token saved to {}", Config::token_path()?.display());
    Ok(())
}

/// Execute the logout command.
pub async fn execute_logout(formatter: &Formatter) -> Result<()> {
    if delete_token()? {
        println!("{}", formatter.success("Stored token discarded"));
    } else {
        println!("{}", formatter.info("No stored token to discard"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OutputFormat;
    use std::cell::RefCell;
    use std::sync::Mutex;

    /// `BOSWELL_CONFIG_DIR` is process-global, and cargo runs a crate's tests
    /// on threads of one process. Two tests pointing it at different
    /// directories at once made the token file appear and vanish under each
    /// other; serializing the ones that touch it is the fix.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn formatter() -> Formatter {
        Formatter::new(OutputFormat::Table, false)
    }

    fn token_response() -> TokenResponse {
        TokenResponse {
            access_token: "at".into(),
            token_type: "Bearer".into(),
            expires_in: Some(3600),
            scope: Some("read".into()),
        }
    }

    /// The two codes that mean "keep going" are the only two that do. Getting
    /// this wrong in either direction is a real failure: treating `slow_down`
    /// as terminal aborts a login the user is about to approve, and treating
    /// `access_denied` as pending hammers the provider forever.
    #[test]
    fn only_pending_and_slow_down_continue_the_loop() {
        assert!(matches!(
            classify("authorization_pending", None),
            PollStep::Pending
        ));
        assert!(matches!(classify("slow_down", None), PollStep::SlowDown));
        assert!(matches!(
            classify("access_denied", None),
            PollStep::Failed(_)
        ));
        assert!(matches!(
            classify("expired_token", None),
            PollStep::Failed(_)
        ));
        assert!(matches!(
            classify("invalid_client", None),
            PollStep::Failed(_)
        ));
    }

    /// An unrecognized code is terminal, not pending. The alternative is an
    /// infinite loop against a provider that has already said no.
    #[test]
    fn an_unknown_error_code_is_terminal_and_keeps_its_description() {
        let step = classify("some_new_code", Some("the tenant is suspended"));
        let PollStep::Failed(why) = step else {
            panic!("an unknown code must be terminal");
        };
        assert!(why.contains("some_new_code"), "got {}", why);
        assert!(why.contains("the tenant is suspended"), "got {}", why);
    }

    /// The happy path: pending twice, then a token.
    #[tokio::test(start_paused = true)]
    async fn the_loop_returns_the_token_once_the_grant_completes() {
        let calls = RefCell::new(0);
        let token = poll_for_token(
            || async {
                *calls.borrow_mut() += 1;
                if *calls.borrow() < 3 {
                    Ok(PollStep::Pending)
                } else {
                    Ok(PollStep::Done(Box::new(token_response())))
                }
            },
            Duration::from_secs(5),
            Duration::from_secs(600),
        )
        .await
        .expect("the grant completes");

        assert_eq!(token.access_token, "at");
        assert_eq!(*calls.borrow(), 3);
    }

    /// `slow_down` must actually slow the loop down. Ignoring it gets the
    /// client rate-limited by the provider, which then looks like a Boswell bug.
    #[tokio::test(start_paused = true)]
    async fn slow_down_lengthens_the_interval_by_five_seconds() {
        let calls = RefCell::new(0);
        let start = Instant::now();

        poll_for_token(
            || async {
                *calls.borrow_mut() += 1;
                match *calls.borrow() {
                    1 => Ok(PollStep::SlowDown),
                    _ => Ok(PollStep::Done(Box::new(token_response()))),
                }
            },
            Duration::from_secs(5),
            Duration::from_secs(600),
        )
        .await
        .expect("the grant completes");

        // 5s before the first poll, then 10s before the second.
        assert_eq!(Instant::now() - start, Duration::from_secs(15));
    }

    /// The first poll waits. An immediate one can only return
    /// `authorization_pending` — the user has not typed the code yet.
    #[tokio::test(start_paused = true)]
    async fn the_first_poll_happens_after_one_interval_not_immediately() {
        let start = Instant::now();
        poll_for_token(
            || async { Ok(PollStep::Done(Box::new(token_response()))) },
            Duration::from_secs(5),
            Duration::from_secs(600),
        )
        .await
        .expect("the grant completes");

        assert_eq!(Instant::now() - start, Duration::from_secs(5));
    }

    /// A user who never approves must not leave the CLI polling forever.
    #[tokio::test(start_paused = true)]
    async fn the_loop_gives_up_at_the_expiry_deadline() {
        let calls = RefCell::new(0);
        let err = poll_for_token(
            || async {
                *calls.borrow_mut() += 1;
                Ok(PollStep::Pending)
            },
            Duration::from_secs(5),
            Duration::from_secs(20),
        )
        .await
        .expect_err("an unapproved grant must time out");

        assert!(format!("{}", err).contains("expired"), "got {}", err);
        // Four polls fit in twenty seconds at five-second intervals; the fifth
        // would be past the deadline.
        assert_eq!(*calls.borrow(), 4);
    }

    /// A refusal stops the loop at once rather than waiting out the deadline.
    #[tokio::test(start_paused = true)]
    async fn a_refusal_ends_the_loop_immediately() {
        let calls = RefCell::new(0);
        let err = poll_for_token(
            || async {
                *calls.borrow_mut() += 1;
                Ok(PollStep::Failed("the request was declined".into()))
            },
            Duration::from_secs(5),
            Duration::from_secs(600),
        )
        .await
        .expect_err("a refusal must be terminal");

        assert!(format!("{}", err).contains("declined"), "got {}", err);
        assert_eq!(*calls.borrow(), 1);
    }

    /// Trailing slashes on the issuer must not double up in the discovery URL.
    #[test]
    fn the_issuer_loses_its_trailing_slash() {
        assert_eq!(
            normalize_issuer("https://id.example.com/"),
            "https://id.example.com"
        );
        assert_eq!(
            normalize_issuer("  https://id.example.com  "),
            "https://id.example.com"
        );
        assert_eq!(
            normalize_issuer("https://id.example.com"),
            "https://id.example.com"
        );
    }

    /// Flags win over the config, and the scope list is replaced rather than
    /// merged — asking for *fewer* scopes has to be possible.
    #[test]
    fn flags_override_the_configured_provider() {
        let mut config = Config::default();
        config.oidc = Some(crate::config::LoginConfig {
            issuer: "https://configured.example".into(),
            client_id: "configured".into(),
            scopes: vec!["read".into(), "write".into()],
        });

        let args = LoginArgs {
            issuer: Some("https://flag.example".into()),
            client_id: Some("flag".into()),
            scope: vec!["read".into()],
            status: false,
        };

        let (issuer, client_id, scope) = resolve_provider(&args, &config).unwrap();
        assert_eq!(issuer, "https://flag.example");
        assert_eq!(client_id, "flag");
        assert_eq!(scope, "read");
    }

    /// With no flags the config supplies everything, and the scopes are joined
    /// with spaces the way OAuth spells a scope list.
    #[test]
    fn the_config_supplies_the_provider_when_no_flags_are_given() {
        let mut config = Config::default();
        config.oidc = Some(crate::config::LoginConfig {
            issuer: "https://configured.example".into(),
            client_id: "configured".into(),
            scopes: vec!["read".into(), "write".into()],
        });

        let args = LoginArgs {
            issuer: None,
            client_id: None,
            scope: Vec::new(),
            status: false,
        };

        let (issuer, client_id, scope) = resolve_provider(&args, &config).unwrap();
        assert_eq!(issuer, "https://configured.example");
        assert_eq!(client_id, "configured");
        assert_eq!(scope, "read write");
    }

    /// Neither flag nor config means a sentence naming both ways to fix it,
    /// not a panic and not a request to a nonexistent host.
    #[test]
    fn a_missing_provider_is_a_configuration_error() {
        let config = Config::default();
        let args = LoginArgs {
            issuer: None,
            client_id: None,
            scope: Vec::new(),
            status: false,
        };

        let err = resolve_provider(&args, &config).expect_err("no issuer is an error");
        assert!(format!("{}", err).contains("--issuer"), "got {}", err);
    }

    /// A token with no stated expiry is not treated as expired. The provider
    /// declined to say; guessing on its behalf would log the operator out of a
    /// perfectly good session.
    #[test]
    fn a_token_without_an_expiry_never_reads_as_expired() {
        let token = StoredToken {
            issuer: "https://id.example".into(),
            access_token: "at".into(),
            token_type: "Bearer".into(),
            expires_at: None,
            scope: None,
        };
        assert!(!token.is_expired_at(u64::MAX));
    }

    #[test]
    fn a_token_is_expired_at_and_after_its_expiry() {
        let token = StoredToken {
            issuer: "https://id.example".into(),
            access_token: "at".into(),
            token_type: "Bearer".into(),
            expires_at: Some(100),
            scope: None,
        };
        assert!(!token.is_expired_at(99));
        assert!(token.is_expired_at(100));
        assert!(token.is_expired_at(101));
    }

    /// The stored form must not carry a refresh token, whatever the provider
    /// sent. A serde round trip is the check that matters: the field would have
    /// to exist on the struct to survive one.
    #[test]
    fn the_stored_token_has_no_refresh_token_field() {
        let token = StoredToken {
            issuer: "https://id.example".into(),
            access_token: "at".into(),
            token_type: "Bearer".into(),
            expires_at: Some(100),
            scope: Some("read".into()),
        };
        let json = serde_json::to_string(&token).unwrap();
        assert!(!json.contains("refresh"), "got {}", json);
    }

    /// The whole point of the file is that it holds a bearer token, so its mode
    /// is part of the feature, not a detail. `save_token` writes through
    /// `Config::token_path`, which under `cfg(test)` is redirected away from a
    /// developer's real `~/.boswell`.
    #[cfg(unix)]
    #[test]
    fn the_token_file_is_written_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("boswell-token-{}", std::process::id()));
        std::env::set_var("BOSWELL_CONFIG_DIR", &dir);

        let token = StoredToken {
            issuer: "https://id.example".into(),
            access_token: "secret".into(),
            token_type: "Bearer".into(),
            expires_at: Some(unix_now() + 60),
            scope: None,
        };
        save_token(&token).unwrap();

        let path = Config::token_path().unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "token file must be owner-only");

        assert_eq!(load_token().unwrap().as_ref(), Some(&token));
        assert!(delete_token().unwrap());
        assert_eq!(load_token().unwrap(), None);
        assert!(!delete_token().unwrap());

        std::env::remove_var("BOSWELL_CONFIG_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `--status` with nothing stored says so instead of failing.
    ///
    /// Driven through `block_on` rather than `#[tokio::test]`: the env guard
    /// has to be held across the call, and holding a `std::sync::MutexGuard`
    /// across an `.await` is a clippy error the workspace denies.
    #[test]
    fn status_with_no_token_is_not_an_error() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("boswell-status-{}", std::process::id()));
        std::env::set_var("BOSWELL_CONFIG_DIR", &dir);
        std::fs::create_dir_all(&dir).unwrap();

        let args = LoginArgs {
            issuer: None,
            client_id: None,
            scope: Vec::new(),
            status: true,
        };
        // No issuer configured either: --status must not need one.
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(execute_login(args, &Config::default(), &formatter()))
            .expect("status must work with nothing stored");

        std::env::remove_var("BOSWELL_CONFIG_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }
}

//! xAI / Grok OAuth credential loading, refresh, and device-code login.
//!
//! Two paths, matching [#4257](https://github.com/Hmbown/CodeWhale/issues/4257):
//!
//! 1. **Delegate-login** — reuse the official Grok CLI token file at
//!    `~/.grok/auth.json` (or `$GROK_HOME/auth.json` / `$GROK_AUTH_PATH`).
//! 2. **Native device-code** — request a code from `auth.x.ai`, print the
//!    verification URL + user code, poll the token endpoint, and write tokens
//!    back to the Grok CLI auth file shape (so both tools stay compatible).
//!
//! Access tokens are sent as `Authorization: Bearer` on the OpenAI-compatible
//! xAI Chat Completions route (`https://api.x.ai/v1`). Token values are never
//! logged.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Official Grok CLI public OIDC client id (public client; no secret).
pub const GROK_OIDC_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
/// Default issuer / authorization server.
pub const XAI_OIDC_ISSUER: &str = "https://auth.x.ai";
/// Scopes requested by device-code login (matches Grok CLI surface).
pub const DEFAULT_SCOPES: &str =
    "openid profile email offline_access api:access grok-cli:access team:read";
const REFRESH_SKEW_SECS: i64 = 60;
const DEVICE_POLL_DEFAULT_SECS: u64 = 5;
const DEVICE_POLL_MAX_SECS: u64 = 900;

/// One entry in `~/.grok/auth.json` (map key = `{issuer}::{client_id}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokAuthEntry {
    /// Access token (JWT). Field name matches the Grok CLI (`key`).
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// RFC3339 expiry timestamp written by the Grok CLI.
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub oidc_issuer: Option<String>,
    #[serde(default)]
    pub oidc_client_id: Option<String>,
    #[serde(default)]
    pub auth_mode: Option<String>,
    /// Preserve unknown CLI fields on rewrite.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Token endpoint response (device-code exchange or refresh).
#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: Option<String>,
    verification_uri_complete: Option<String>,
    expires_in: Option<u64>,
    interval: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Resolved bearer credential ready for API use.
#[derive(Debug, Clone)]
pub struct XaiOAuthCredentials {
    pub access_token: String,
    #[allow(dead_code)]
    pub refresh_token: Option<String>,
    #[allow(dead_code)]
    pub expires_at: Option<String>,
    #[allow(dead_code)]
    pub issuer: String,
    #[allow(dead_code)]
    pub client_id: String,
}

/// Whether `[providers.xai] auth_mode` selects the OAuth path.
#[must_use]
pub fn auth_mode_uses_xai_oauth(mode: &str) -> bool {
    matches!(
        normalize_auth_mode(mode).as_str(),
        "oauth"
            | "xai_oauth"
            | "xai"
            | "grok"
            | "grok_oauth"
            | "grok_cli"
            | "device"
            | "device_code"
            | "device_auth"
    )
}

fn normalize_auth_mode(mode: &str) -> String {
    mode.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

/// Resolve the Grok CLI auth file path.
///
/// Priority:
/// 1. `GROK_AUTH_PATH` / `XAI_AUTH_PATH`
/// 2. `$GROK_HOME/auth.json`
/// 3. `~/.grok/auth.json`
#[must_use]
pub fn auth_file_path() -> PathBuf {
    for key in ["GROK_AUTH_PATH", "XAI_AUTH_PATH"] {
        if let Ok(path) = std::env::var(key) {
            let p = PathBuf::from(path.trim());
            if !p.as_os_str().is_empty() {
                return p;
            }
        }
    }
    if let Ok(home) = std::env::var("GROK_HOME") {
        let p = PathBuf::from(home.trim());
        if !p.as_os_str().is_empty() {
            return p.join("auth.json");
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".grok")
        .join("auth.json")
}

#[must_use]
pub fn credentials_present() -> bool {
    auth_file_path().exists()
}

/// Prompt-free structural check for Grok/xAI OAuth material. Never refreshes
/// or writes: a fresh access token or a non-empty refresh token is enough to
/// keep the route selectable, while malformed/empty files count as missing.
#[must_use]
pub fn credentials_valid() -> bool {
    let path = auth_file_path();
    let Ok(mut file) = load_auth_file(&path) else {
        return false;
    };
    let Some((_, entry)) = select_entry(&mut file) else {
        return false;
    };
    entry_access_token_is_fresh(&entry)
        || entry
            .refresh_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty())
}

/// Load + refresh OAuth credentials from the Grok CLI auth file.
pub fn get_access_token() -> Result<String> {
    Ok(get_credentials()?.access_token)
}

pub fn get_credentials() -> Result<XaiOAuthCredentials> {
    let path = auth_file_path();
    if !path.exists() {
        bail!("{}", missing_auth_message());
    }
    let mut file = load_auth_file(&path)?;
    let (scope, mut entry) = select_entry(&mut file).ok_or_else(|| {
        anyhow::anyhow!(
            "xAI OAuth credentials at {} have no usable entry. Run `grok login` \
             or `codewhale auth xai-device` (device-code).",
            path.display()
        )
    })?;

    if entry_access_token_is_fresh(&entry) {
        let token = entry
            .key
            .clone()
            .filter(|t| !t.trim().is_empty())
            .context("xAI OAuth access token is empty")?;
        return Ok(credentials_from_entry(scope, &entry, token));
    }

    let refresh = entry
        .refresh_token
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .context(
            "xAI OAuth access token expired and no refresh_token is stored. \
             Run `grok login` or `codewhale auth xai-device` again.",
        )?;
    let issuer = entry
        .oidc_issuer
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| issuer_from_scope(&scope));
    let client_id = entry
        .oidc_client_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| client_id_from_scope(&scope));

    let refreshed = refresh_access_token(&issuer, &client_id, refresh)?;
    apply_token_response(&mut entry, &issuer, &client_id, &refreshed)?;
    file.insert(scope.clone(), entry.clone());
    write_auth_file(&path, &file)?;

    let token = entry
        .key
        .clone()
        .filter(|t| !t.trim().is_empty())
        .context("xAI OAuth refresh returned an empty access token")?;
    Ok(credentials_from_entry(scope, &entry, token))
}

/// Interactive device-code login. Prints verification URL + user code to
/// `stderr`, polls until approved, and writes `~/.grok/auth.json`.
///
/// Public residual entry point for CLI/TUI wiring (`codewhale auth` /
/// slash command). Call from a headless or TUI surface that can print the
/// verification URL.
pub fn device_code_login() -> Result<XaiOAuthCredentials> {
    let issuer = std::env::var("GROK_OIDC_ISSUER")
        .or_else(|_| std::env::var("XAI_OIDC_ISSUER"))
        .unwrap_or_else(|_| XAI_OIDC_ISSUER.to_string());
    let client_id = std::env::var("GROK_OIDC_CLIENT_ID")
        .or_else(|_| std::env::var("XAI_OIDC_CLIENT_ID"))
        .unwrap_or_else(|_| GROK_OIDC_CLIENT_ID.to_string());
    let scopes = std::env::var("GROK_OIDC_SCOPES")
        .or_else(|_| std::env::var("XAI_OIDC_SCOPES"))
        .unwrap_or_else(|_| DEFAULT_SCOPES.to_string());

    let device = request_device_code(&issuer, &client_id, &scopes)?;
    let verify = device
        .verification_uri_complete
        .clone()
        .or(device.verification_uri.clone())
        .unwrap_or_else(|| format!("{issuer}/device"));

    eprintln!("xAI device-code login");
    eprintln!("  Open:  {verify}");
    eprintln!("  Code:  {}", device.user_code);
    eprintln!("Waiting for approval in the browser… (Ctrl+C to abort)");
    if std::env::var_os("CODEWHALE_XAI_OAUTH_NO_BROWSER").is_none()
        && let Err(err) = webbrowser::open(&verify)
    {
        eprintln!("Could not open the browser automatically: {err}");
    }

    let interval = device.interval.unwrap_or(DEVICE_POLL_DEFAULT_SECS).max(1);
    let deadline = std::time::Instant::now()
        + Duration::from_secs(device.expires_in.unwrap_or(DEVICE_POLL_MAX_SECS).max(30));

    loop {
        if std::time::Instant::now() >= deadline {
            bail!(
                "xAI device-code authorization timed out. Re-run device login \
                 and approve the code before it expires."
            );
        }
        thread::sleep(Duration::from_secs(interval));
        match poll_device_token(&issuer, &client_id, &device.device_code) {
            Ok(token) => {
                let path = auth_file_path();
                let mut file = if path.exists() {
                    load_auth_file(&path).unwrap_or_default()
                } else {
                    BTreeMap::new()
                };
                let scope = format!("{issuer}::{client_id}");
                let mut entry = file.remove(&scope).unwrap_or(GrokAuthEntry {
                    key: None,
                    refresh_token: None,
                    expires_at: None,
                    oidc_issuer: Some(issuer.clone()),
                    oidc_client_id: Some(client_id.clone()),
                    auth_mode: Some("oidc".to_string()),
                    extra: BTreeMap::new(),
                });
                apply_token_response(&mut entry, &issuer, &client_id, &token)?;
                file.insert(scope.clone(), entry.clone());
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("creating xAI OAuth auth directory {}", parent.display())
                    })?;
                }
                write_auth_file(&path, &file)?;
                let access = entry
                    .key
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .context("xAI device-code login returned an empty access token")?;
                eprintln!(
                    "Signed in. Tokens stored at {} (mode 0600).",
                    path.display()
                );
                return Ok(credentials_from_entry(scope, &entry, access));
            }
            Err(err) => {
                let msg = err.to_string();
                if msg.contains("authorization_pending") || msg.contains("slow_down") {
                    continue;
                }
                return Err(err);
            }
        }
    }
}

#[must_use]
pub fn missing_auth_message() -> String {
    format!(
        "xAI OAuth credentials not found.\n\
         Options:\n\
         1. Run `grok login` (or `grok login --device-auth`) and set \
         [providers.xai] auth_mode = \"oauth\"\n\
         2. Run device-code login, then set auth_mode = \"oauth\"\n\
         3. Or use API-key auth: export XAI_API_KEY=... / \
         codewhale auth set --provider xai\n\
         Looked for: {}",
        auth_file_path().display()
    )
}

// ── internals ──────────────────────────────────────────────────────────────

type AuthFile = BTreeMap<String, GrokAuthEntry>;

fn load_auth_file(path: &Path) -> Result<AuthFile> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading xAI/Grok auth file {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing xAI/Grok auth file {}", path.display()))?;
    let obj = value.as_object().with_context(|| {
        format!(
            "xAI/Grok auth file {} must be a JSON object of scope → entry",
            path.display()
        )
    })?;
    let mut out = BTreeMap::new();
    for (k, v) in obj {
        match serde_json::from_value::<GrokAuthEntry>(v.clone()) {
            Ok(entry) => {
                out.insert(k.clone(), entry);
            }
            Err(err) => {
                tracing::warn!(
                    target: "codewhale::xai_oauth",
                    scope = %k,
                    error = %err,
                    "skipping unreadable xAI auth entry"
                );
            }
        }
    }
    Ok(out)
}

fn write_auth_file(path: &Path, file: &AuthFile) -> Result<()> {
    let serialized =
        serde_json::to_vec_pretty(file).context("serializing xAI OAuth credentials")?;
    crate::utils::write_atomic(path, &serialized)
        .with_context(|| format!("writing xAI OAuth credentials to {}", path.display()))?;
    #[cfg(unix)]
    if let Err(err) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
        tracing::warn!(
            target: "codewhale::xai_oauth",
            path = %path.display(),
            error = %err,
            "could not enforce 0o600 on xAI OAuth credentials; relying on host ACLs"
        );
    }
    Ok(())
}

fn select_entry(file: &mut AuthFile) -> Option<(String, GrokAuthEntry)> {
    // Prefer the official Grok CLI client id scope when present.
    let preferred_suffix = format!("::{GROK_OIDC_CLIENT_ID}");
    if let Some((k, v)) = file
        .iter()
        .find(|(k, e)| k.ends_with(&preferred_suffix) && entry_has_usable_secret(e))
    {
        return Some((k.clone(), v.clone()));
    }
    file.iter()
        .find(|(_, e)| entry_has_usable_secret(e))
        .map(|(k, v)| (k.clone(), v.clone()))
}

fn entry_has_usable_secret(entry: &GrokAuthEntry) -> bool {
    entry.key.as_deref().is_some_and(|t| !t.trim().is_empty())
        || entry
            .refresh_token
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
}

fn entry_access_token_is_fresh(entry: &GrokAuthEntry) -> bool {
    let Some(token) = entry.key.as_deref().filter(|t| !t.trim().is_empty()) else {
        return false;
    };
    if let Some(exp) = entry.expires_at.as_deref().and_then(parse_rfc3339_secs) {
        let now = now_unix_secs().unwrap_or(0);
        return exp - now > REFRESH_SKEW_SECS;
    }
    // Fall back to JWT exp claim when expires_at is missing.
    match jwt_expiry_seconds(token) {
        Some(exp) => {
            let now = now_unix_secs().unwrap_or(0) as u64;
            (exp as i64) - (now as i64) > REFRESH_SKEW_SECS
        }
        // Unknown expiry → treat as stale so refresh runs.
        None => false,
    }
}

fn credentials_from_entry(
    scope: String,
    entry: &GrokAuthEntry,
    access_token: String,
) -> XaiOAuthCredentials {
    XaiOAuthCredentials {
        access_token,
        refresh_token: entry.refresh_token.clone(),
        expires_at: entry.expires_at.clone(),
        issuer: entry
            .oidc_issuer
            .clone()
            .unwrap_or_else(|| issuer_from_scope(&scope)),
        client_id: entry
            .oidc_client_id
            .clone()
            .unwrap_or_else(|| client_id_from_scope(&scope)),
    }
}

fn issuer_from_scope(scope: &str) -> String {
    scope
        .split_once("::")
        .map(|(issuer, _)| issuer.to_string())
        .unwrap_or_else(|| XAI_OIDC_ISSUER.to_string())
}

fn client_id_from_scope(scope: &str) -> String {
    scope
        .split_once("::")
        .map(|(_, id)| id.to_string())
        .unwrap_or_else(|| GROK_OIDC_CLIENT_ID.to_string())
}

fn apply_token_response(
    entry: &mut GrokAuthEntry,
    issuer: &str,
    client_id: &str,
    token: &TokenResponse,
) -> Result<()> {
    let access = token
        .access_token
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .context("token response missing access_token")?;
    entry.key = Some(access.to_string());
    if let Some(rt) = token
        .refresh_token
        .as_deref()
        .filter(|t| !t.trim().is_empty())
    {
        entry.refresh_token = Some(rt.to_string());
    }
    entry.oidc_issuer = Some(issuer.to_string());
    entry.oidc_client_id = Some(client_id.to_string());
    entry.auth_mode = Some("oidc".to_string());
    if let Some(expires_in) = token.expires_in {
        entry.expires_at = Some(rfc3339_from_now(expires_in));
    } else if let Some(exp) = jwt_expiry_seconds(access) {
        entry.expires_at = Some(rfc3339_from_unix(exp as i64));
    }
    Ok(())
}

fn refresh_access_token(
    issuer: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenResponse> {
    let url = format!("{}/oauth2/token", issuer.trim_end_matches('/'));
    let client = crate::tls::reqwest_blocking_client_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("Failed to build xAI OAuth refresh client")?;
    let params = [
        ("client_id", client_id),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    let response = client
        .post(url)
        .form(&params)
        .send()
        .context("xAI OAuth refresh request failed")?;
    let status = response.status();
    let body: TokenResponse = response
        .json()
        .context("Failed to parse xAI OAuth refresh response")?;
    if !status.is_success() || body.error.is_some() {
        let err = body
            .error_description
            .or(body.error)
            .unwrap_or_else(|| format!("HTTP {status}"));
        bail!(
            "xAI OAuth refresh failed ({err}). Run `grok login` or device-code login again. \
             If SuperGrok OAuth returns HTTP 403, use XAI_API_KEY instead."
        );
    }
    Ok(body)
}

fn request_device_code(issuer: &str, client_id: &str, scopes: &str) -> Result<DeviceCodeResponse> {
    let url = format!("{}/oauth2/device/code", issuer.trim_end_matches('/'));
    let client = crate::tls::reqwest_blocking_client_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("Failed to build xAI device-code client")?;
    let params = [("client_id", client_id), ("scope", scopes)];
    let response = client
        .post(url)
        .form(&params)
        .send()
        .context("xAI device-code request failed")?;
    let status = response.status();
    let body: DeviceCodeResponse = response
        .json()
        .context("Failed to parse xAI device-code response")?;
    if !status.is_success() || body.error.is_some() {
        let err = body
            .error_description
            .or(body.error)
            .unwrap_or_else(|| format!("HTTP {status}"));
        bail!("xAI device-code request failed ({err})");
    }
    if body.device_code.trim().is_empty() || body.user_code.trim().is_empty() {
        bail!("xAI device-code response missing device_code/user_code");
    }
    Ok(body)
}

fn poll_device_token(issuer: &str, client_id: &str, device_code: &str) -> Result<TokenResponse> {
    let url = format!("{}/oauth2/token", issuer.trim_end_matches('/'));
    let client = crate::tls::reqwest_blocking_client_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("Failed to build xAI device-code poll client")?;
    let params = [
        ("client_id", client_id),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ("device_code", device_code),
    ];
    let response = client
        .post(url)
        .form(&params)
        .send()
        .context("xAI device-code token poll failed")?;
    let status = response.status();
    let body: TokenResponse = response
        .json()
        .context("Failed to parse xAI device-code token response")?;
    if let Some(err) = body.error.as_deref() {
        if matches!(err, "authorization_pending" | "slow_down") {
            bail!("{err}");
        }
        let detail = body
            .error_description
            .clone()
            .unwrap_or_else(|| err.to_string());
        bail!("xAI device-code token exchange failed: {detail}");
    }
    if !status.is_success() {
        bail!("xAI device-code token exchange failed with HTTP {status}");
    }
    Ok(body)
}

fn jwt_expiry_seconds(token: &str) -> Option<u64> {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: Value = serde_json::from_slice(&decoded).ok()?;
    claims.get("exp")?.as_u64()
}

fn now_unix_secs() -> Option<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

fn parse_rfc3339_secs(raw: &str) -> Option<i64> {
    // Prefer chrono when available for full RFC3339; fall back to simple UTC forms.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(dt.timestamp());
    }
    // e.g. 2026-07-09T12:00:00Z
    let trimmed = raw.trim().trim_end_matches('Z');
    let (date, time) = trimmed.split_once('T')?;
    let mut d = date.split('-');
    let y: i32 = d.next()?.parse().ok()?;
    let m: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    let time = time.split('+').next()?.split('-').next()?;
    let mut t = time.split(':');
    let hh: u32 = t.next()?.parse().ok()?;
    let mm: u32 = t.next()?.parse().ok()?;
    let ss: u32 = t
        .next()
        .and_then(|s| s.split('.').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let ndt = chrono::NaiveDate::from_ymd_opt(y, m, day)?.and_hms_opt(hh, mm, ss)?;
    Some(ndt.and_utc().timestamp())
}

fn rfc3339_from_now(expires_in: u64) -> String {
    let ts = now_unix_secs().unwrap_or(0) + expires_in as i64;
    rfc3339_from_unix(ts)
}

fn rfc3339_from_unix(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_else(|| format!("{ts}"))
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn auth_mode_accepts_oauth_aliases() {
        for mode in [
            "oauth",
            "xai_oauth",
            "XAI-OAuth",
            "grok",
            "grok_cli",
            "device_code",
            "device-auth",
        ] {
            assert!(
                auth_mode_uses_xai_oauth(mode),
                "expected oauth mode: {mode}"
            );
        }
        assert!(!auth_mode_uses_xai_oauth("api_key"));
        assert!(!auth_mode_uses_xai_oauth("keyring"));
    }

    #[test]
    fn loads_fresh_token_from_grok_auth_json() {
        let _guard = crate::test_support::lock_test_env();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.json");
        let future = rfc3339_from_now(3600);
        let scope = format!("{XAI_OIDC_ISSUER}::{GROK_OIDC_CLIENT_ID}");
        let file = serde_json::json!({
            scope: {
                "key": "test-access-token",
                "refresh_token": "test-refresh",
                "expires_at": future,
                "oidc_issuer": XAI_OIDC_ISSUER,
                "oidc_client_id": GROK_OIDC_CLIENT_ID,
                "auth_mode": "oidc"
            }
        });
        fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();
        // SAFETY: serialized by the process-wide test environment lock;
        // restored below.
        unsafe {
            std::env::set_var("GROK_AUTH_PATH", &path);
        }
        let result = get_credentials();
        unsafe {
            std::env::remove_var("GROK_AUTH_PATH");
        }
        let creds = result.expect("load");
        assert_eq!(creds.access_token, "test-access-token");
        assert_eq!(creds.client_id, GROK_OIDC_CLIENT_ID);
    }

    #[test]
    fn missing_file_message_mentions_oauth_paths() {
        let _guard = crate::test_support::lock_test_env();
        let msg = missing_auth_message();
        assert!(msg.contains("xAI OAuth credentials not found"), "{msg}");
        assert!(msg.contains("auth_mode"), "{msg}");
        assert!(msg.contains("XAI_API_KEY"), "{msg}");
    }

    #[test]
    fn parse_rfc3339_accepts_zulu() {
        let ts = parse_rfc3339_secs("2026-07-09T12:00:00.000Z").expect("parse");
        assert!(ts > 0);
    }

    #[test]
    fn device_code_constants_match_discovery_shape() {
        assert!(DEFAULT_SCOPES.contains("offline_access"));
        assert!(DEFAULT_SCOPES.contains("api:access"));
        assert_eq!(XAI_OIDC_ISSUER, "https://auth.x.ai");
        assert_eq!(GROK_OIDC_CLIENT_ID.len(), 36);
        // Keep device_code_login referenced so the residual entry point stays linked.
        let _ = device_code_login as fn() -> Result<XaiOAuthCredentials>;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn device_code_login_exchanges_and_persists_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/oauth2/device/code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_code": "device-token",
                "user_code": "CW-TEST",
                "verification_uri": format!("{}/verify", server.uri()),
                "expires_in": 60,
                "interval": 1
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "test-xai-access",
                "refresh_token": "test-xai-refresh",
                "expires_in": 3600,
                "token_type": "Bearer"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let _guard = crate::test_support::lock_test_env();
        let dir = TempDir::new().unwrap();
        let auth_path = dir.path().join("grok-auth.json");
        unsafe {
            std::env::set_var("GROK_OIDC_ISSUER", server.uri());
            std::env::set_var("GROK_AUTH_PATH", &auth_path);
            std::env::set_var("CODEWHALE_XAI_OAUTH_NO_BROWSER", "1");
        }
        let result = tokio::task::block_in_place(device_code_login);
        unsafe {
            std::env::remove_var("GROK_OIDC_ISSUER");
            std::env::remove_var("GROK_AUTH_PATH");
            std::env::remove_var("CODEWHALE_XAI_OAUTH_NO_BROWSER");
        }

        let credentials = result.expect("device login");
        assert_eq!(credentials.access_token, "test-xai-access");
        assert_eq!(
            credentials.refresh_token.as_deref(),
            Some("test-xai-refresh")
        );
        let persisted = fs::read_to_string(&auth_path).expect("persisted auth file");
        assert!(persisted.contains("test-xai-access"));
        assert!(persisted.contains("test-xai-refresh"));
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&auth_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

//! Firebase Cloud Messaging, HTTP v1: the fallback route when the phone has no
//! `UnifiedPush` distributor (owner ruling OW.26, "both, ntfy first").
//!
//! The server holds a Firebase service account (`fcm-service-account.json`,
//! `0600`, beside the config): the owner's credential, never made here. From it
//! an RS256 assertion is exchanged for a one-hour access token (cached), and
//! the alert goes as a DATA message so the app draws the same notification on
//! either route.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};
use serde::Deserialize;

use super::http::{Request, Sender};

/// The file the owner downloads from the Firebase console (Project settings,
/// Service accounts, Generate new private key) and places beside the config.
pub const SERVICE_ACCOUNT_FILE: &str = "fcm-service-account.json";
/// The one OAuth scope sending needs.
pub const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
/// Google's longest assertion lifetime, and so the access token's.
pub const ASSERTION_SECS: u64 = 3600;
/// Refresh this long before the token's stated expiry: a send never races it.
pub const REFRESH_MARGIN_SECS: u64 = 300;
/// How long FCM holds an undelivered alert (a phone in a tunnel).
pub const MESSAGE_TTL: &str = "86400s";
const SEND_URL_BASE: &str = "https://fcm.googleapis.com/v1/projects";

/// The fields of a Google service-account key this client uses.
#[derive(Deserialize, Clone)]
pub struct ServiceAccount {
    pub project_id: String,
    pub client_email: String,
    pub private_key: String,
    pub token_uri: String,
}

impl std::fmt::Debug for ServiceAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceAccount")
            .field("project_id", &self.project_id)
            .finish_non_exhaustive()
    }
}

/// Where the service account lives for a config at `config_path`.
#[must_use]
pub fn default_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(SERVICE_ACCOUNT_FILE)
}

/// The FCM client, or `None` when no service account is installed: FCM is then
/// not a route, and the status says so rather than failing a send.
///
/// # Errors
///
/// A file that exists but is open to others, unreadable, or not a usable key.
pub fn load(path: &Path) -> Result<Option<Fcm>, String> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    crate::owner_file::refuse_if_open(
        path,
        std::os::unix::fs::PermissionsExt::mode(&meta.permissions()),
    )?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let account: ServiceAccount = serde_json::from_str(&text)
        .map_err(|_| format!("{} is not a Google service-account key", path.display()))?;
    Fcm::new(account).map(Some)
}

fn pem_der(pem: &str) -> Option<Vec<u8>> {
    // Only the lines between the armour: text before BEGIN (RFC 7468 allows
    // explanatory text there) is not key material.
    let body: String = pem
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect::<String>()
        .split_whitespace()
        .collect();
    base64::engine::general_purpose::STANDARD.decode(body).ok()
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Seconds since the epoch.
#[must_use]
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub struct Fcm {
    account: ServiceAccount,
    key: RsaKeyPair,
    cached: tokio::sync::Mutex<Option<(String, u64)>>,
}

impl std::fmt::Debug for Fcm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fcm")
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    expires_in: Option<u64>,
}

impl Fcm {
    /// # Errors
    ///
    /// The key is not an RSA PKCS#8 PEM, or the token URI is not https.
    pub fn new(account: ServiceAccount) -> Result<Self, String> {
        let der = pem_der(&account.private_key).ok_or("FCM: private_key is not PEM")?;
        let key = RsaKeyPair::from_pkcs8(&der)
            .map_err(|_| "FCM: private_key is not an RSA PKCS#8 key")?;
        if !account.token_uri.starts_with("https://") {
            return Err("FCM: token_uri is not https".into());
        }
        Ok(Self {
            account,
            key,
            cached: tokio::sync::Mutex::new(None),
        })
    }

    /// The signed JWT that buys an access token (RFC 7523).
    ///
    /// # Errors
    ///
    /// When signing fails.
    pub fn assertion(&self, now: u64) -> Result<String, String> {
        let header = b64url(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = serde_json::json!({
            "iss": self.account.client_email,
            "scope": SCOPE,
            "aud": self.account.token_uri,
            "iat": now,
            "exp": now + ASSERTION_SECS,
        });
        let signing_input = format!("{header}.{}", b64url(claims.to_string().as_bytes()));
        let mut sig = vec![0u8; self.key.public().modulus_len()];
        self.key
            .sign(
                &RSA_PKCS1_SHA256,
                &ring::rand::SystemRandom::new(),
                signing_input.as_bytes(),
                &mut sig,
            )
            .map_err(|_| "FCM: signing the assertion failed".to_string())?;
        Ok(format!("{signing_input}.{}", b64url(&sig)))
    }

    /// The token exchange.
    ///
    /// # Errors
    ///
    /// When signing fails.
    pub fn token_request(&self, now: u64) -> Result<Request, String> {
        let body = format!(
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={}",
            self.assertion(now)?
        );
        Ok(Request {
            url: self.account.token_uri.clone(),
            headers: vec![(
                "content-type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            body: body.into_bytes(),
        })
    }

    /// The send: a data message, high priority, to one app instance by its
    /// Firebase installation ID. HTTP v1's `token` target is deprecated
    /// ("Use fid instead"), as the phone's registration token is since
    /// Firebase Messaging 25.1; the app registers by FID.
    #[must_use]
    pub fn send_request(
        &self,
        access_token: &str,
        device: &str,
        payload: &serde_json::Value,
    ) -> Request {
        // FCM data values are strings; the alert travels as one JSON string
        // under one key, read by the app exactly as the UnifiedPush body is.
        let message = serde_json::json!({
            "message": {
                "fid": device,
                "data": { "alert": payload.to_string() },
                "android": { "priority": "HIGH", "ttl": MESSAGE_TTL },
            }
        });
        Request {
            url: format!("{SEND_URL_BASE}/{}/messages:send", self.account.project_id),
            headers: vec![
                ("authorization".into(), format!("Bearer {access_token}")),
                ("content-type".into(), "application/json".into()),
            ],
            body: message.to_string().into_bytes(),
        }
    }

    async fn access_token(&self, sender: &dyn Sender, now: u64) -> Result<String, String> {
        let fresh = self
            .cached
            .lock()
            .await
            .as_ref()
            .filter(|(_, expires)| now + REFRESH_MARGIN_SECS < *expires)
            .map(|(token, _)| token.clone());
        if let Some(token) = fresh {
            return Ok(token);
        }
        let answer = sender.post(self.token_request(now)?).await?;
        if !answer.ok() {
            return Err(format!("FCM: token exchange answered {}", answer.status));
        }
        let parsed: TokenAnswer = serde_json::from_slice(&answer.body)
            .map_err(|_| "FCM: token exchange answer is not a token".to_string())?;
        let expires = now + parsed.expires_in.unwrap_or(ASSERTION_SECS);
        *self.cached.lock().await = Some((parsed.access_token.clone(), expires));
        Ok(parsed.access_token)
    }

    /// One delivery. A 404 is FCM's "this installation is no longer registered".
    ///
    /// # Errors
    ///
    /// The exchange or the send failed; [`super::Failure::Gone`] for a dead installation.
    pub async fn deliver(
        &self,
        sender: &dyn Sender,
        device: &str,
        payload: &serde_json::Value,
        now: u64,
    ) -> Result<(), super::Failure> {
        let token = self
            .access_token(sender, now)
            .await
            .map_err(super::Failure::Other)?;
        let answer = sender
            .post(self.send_request(&token, device, payload))
            .await
            .map_err(super::Failure::Other)?;
        match answer.status {
            s if super::http::is_success(s) => Ok(()),
            super::GONE_NOT_FOUND => Err(super::Failure::Gone),
            s => Err(super::Failure::Other(format!("FCM answered {s}"))),
        }
    }
}

#[cfg(test)]
#[path = "fcm_tests.rs"]
pub(crate) mod tests;

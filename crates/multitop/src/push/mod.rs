//! A3 push: the server's alerts reach the phone (docs/roadmap.md A3).
//!
//! One seam, two transports (owner ruling OW.26, "both, ntfy first"):
//!
//! * **`UnifiedPush`** -- the app registered an endpoint with whatever distributor
//!   the phone runs (ntfy), plus its Web Push keys; the alert is RFC 8291
//!   ciphertext `POSTed` to that endpoint ([`webpush`]).
//! * **FCM** -- the fallback, when the phone has no distributor, through the
//!   owner's Firebase service account ([`fcm`]).
//!
//! Which one carries an alert is decided at send time by [`plan`] from what the
//! phone has registered and what the server holds: `UnifiedPush` first, FCM after
//! it (also when the `UnifiedPush` delivery fails), nothing when neither is set up
//! -- and the status endpoint says which, rather than a send failing silently.

pub mod fcm;
pub mod http;
pub mod routes;
pub mod watch;
pub mod webpush;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// The registrations, `0600`, beside the serve token.
pub const REGISTRATIONS_FILE: &str = "push-registrations.json";
/// How long a distributor holds an alert the phone has not collected.
pub const UP_TTL_SECS: u32 = 86_400;
/// The status both FCM and Web Push use for "this registration is gone".
pub const GONE_NOT_FOUND: u16 = 404;
/// Web Push's other "gone" (RFC 8030 section 7.3).
pub const GONE: u16 = 410;
/// The longest endpoint or token accepted: real ones are a few hundred bytes.
pub const MAX_REGISTRATION_LEN: usize = 4096;

/// What the phone registered over the paired API.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum Registration {
    /// A `UnifiedPush` endpoint and the Web Push keys (base64url, RFC 8291).
    #[serde(rename = "unifiedpush")]
    UnifiedPush {
        endpoint: String,
        p256dh: String,
        auth: String,
    },
    /// An FCM registration token.
    #[serde(rename = "fcm")]
    Fcm { token: String },
}

impl std::fmt::Debug for Registration {
    // An endpoint is a capability to reach the phone, a token likewise:
    // neither is ever printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.transport().name())
    }
}

/// The two transports, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    UnifiedPush,
    Fcm,
}

impl Transport {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::UnifiedPush => "unifiedpush",
            Self::Fcm => "fcm",
        }
    }
}

fn b64url(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.trim_end_matches('='))
        .ok()
}

impl Registration {
    #[must_use]
    pub const fn transport(&self) -> Transport {
        match self {
            Self::UnifiedPush { .. } => Transport::UnifiedPush,
            Self::Fcm { .. } => Transport::Fcm,
        }
    }

    /// Refuse what could never be delivered, before it is stored.
    ///
    /// # Errors
    ///
    /// The reason, naming the field and never its value.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::UnifiedPush {
                endpoint,
                p256dh,
                auth,
            } => {
                if endpoint.len() > MAX_REGISTRATION_LEN {
                    return Err("endpoint is too long".into());
                }
                http::target(endpoint).map_err(|e| format!("endpoint: {e}"))?;
                if b64url(p256dh).is_none_or(|k| k.len() != webpush::P256_POINT_LEN) {
                    return Err("p256dh is not a base64url P-256 point".into());
                }
                if b64url(auth).is_none_or(|a| a.len() != webpush::AUTH_LEN) {
                    return Err("auth is not a base64url 16-byte secret".into());
                }
                Ok(())
            }
            Self::Fcm { token } => {
                let fits = !token.is_empty() && token.len() <= MAX_REGISTRATION_LEN;
                let clean = token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b));
                if fits && clean {
                    Ok(())
                } else {
                    Err("token is not an FCM registration token".into())
                }
            }
        }
    }
}

/// At most one registration per transport: the phone re-registers, the newest wins.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registrations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unifiedpush: Option<Registration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fcm: Option<Registration>,
}

impl Registrations {
    pub fn put(&mut self, reg: Registration) {
        match reg.transport() {
            Transport::UnifiedPush => self.unifiedpush = Some(reg),
            Transport::Fcm => self.fcm = Some(reg),
        }
    }

    pub fn drop_transport(&mut self, t: Transport) {
        match t {
            Transport::UnifiedPush => self.unifiedpush = None,
            Transport::Fcm => self.fcm = None,
        }
    }
}

/// The routes to try, in order: `UnifiedPush` when registered, then FCM when the
/// phone registered a token AND the server holds a service account. Empty means
/// push is not set up, which the status reports.
#[must_use]
pub fn plan(regs: &Registrations, fcm_ready: bool) -> Vec<Registration> {
    let mut out = Vec::new();
    if let Some(up) = &regs.unifiedpush {
        out.push(up.clone());
    }
    if let (Some(f), true) = (&regs.fcm, fcm_ready) {
        out.push(f.clone());
    }
    out
}

/// Why one route did not deliver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The registration is dead (404/410): it is dropped.
    Gone,
    /// Anything else, as a sentence without secrets.
    Other(String),
}

/// What one alert's delivery did: the route that carried it, if any, and each
/// route's failure before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Delivery {
    pub via: Option<Transport>,
    pub failed: Vec<(Transport, String)>,
}

/// The alert kinds the app draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Down,
    Degraded,
    Recovered,
    Test,
}

/// The payload, identical on both routes. `v` lets the app refuse a shape it
/// does not know rather than draw it wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Alert {
    pub v: u8,
    pub kind: Kind,
    pub host: String,
    pub title: String,
    pub body: String,
}

/// The payload version this server writes.
pub const PAYLOAD_VERSION: u8 = 1;

impl Alert {
    #[must_use]
    pub fn new(kind: Kind, host: &str, title: String, body: String) -> Self {
        Self {
            v: PAYLOAD_VERSION,
            kind,
            host: host.to_string(),
            title,
            body,
        }
    }

    #[must_use]
    pub fn json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

/// The `UnifiedPush` request: the encrypted body to the registered endpoint.
///
/// # Errors
///
/// A registration that is not `UnifiedPush`, or one whose keys do not seal.
pub fn unifiedpush_request(reg: &Registration, alert: &Alert) -> Result<http::Request, String> {
    let Registration::UnifiedPush {
        endpoint,
        p256dh,
        auth,
    } = reg
    else {
        return Err("not a UnifiedPush registration".into());
    };
    let key = b64url(p256dh).ok_or("p256dh is not base64url")?;
    let secret = b64url(auth).ok_or("auth is not base64url")?;
    let body = webpush::encrypt(&key, &secret, alert.json().to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(http::Request {
        url: endpoint.clone(),
        headers: vec![
            ("content-encoding".into(), "aes128gcm".into()),
            ("content-type".into(), "application/octet-stream".into()),
            ("ttl".into(), UP_TTL_SECS.to_string()),
            ("urgency".into(), "high".into()),
        ],
        body,
    })
}

/// The registrations, the FCM client and the sender: everything a send needs.
pub struct Hub {
    store: Option<PathBuf>,
    regs: tokio::sync::RwLock<Registrations>,
    fcm: Option<fcm::Fcm>,
    sender: Arc<dyn http::Sender>,
}

/// What `/api/push` reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub unifiedpush: bool,
    pub fcm: bool,
    pub fcm_ready: bool,
    pub route: Option<Transport>,
}

impl Hub {
    /// A hub with nothing registered and nothing stored: no route until the
    /// phone registers, and registrations kept in memory only.
    #[must_use]
    pub fn new(sender: Arc<dyn http::Sender>, fcm: Option<fcm::Fcm>) -> Self {
        Self {
            store: None,
            regs: tokio::sync::RwLock::new(Registrations::default()),
            fcm,
            sender,
        }
    }

    /// The hub for `--serve`: registrations read from (and written to) `path`.
    ///
    /// # Errors
    ///
    /// A store that is open to others, unreadable, or not JSON.
    pub fn with_store(
        path: &Path,
        sender: Arc<dyn http::Sender>,
        fcm: Option<fcm::Fcm>,
    ) -> Result<Self, String> {
        let regs = match std::fs::metadata(path) {
            Ok(meta) => {
                crate::owner_file::refuse_if_open(
                    path,
                    std::os::unix::fs::PermissionsExt::mode(&meta.permissions()),
                )?;
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                serde_json::from_str(&text)
                    .map_err(|_| format!("{} is not push registrations", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Registrations::default(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut hub = Self::new(sender, fcm);
        hub.store = Some(path.to_path_buf());
        hub.regs = tokio::sync::RwLock::new(regs);
        Ok(hub)
    }

    fn persist(&self, regs: &Registrations) -> Result<(), String> {
        let Some(path) = &self.store else {
            return Ok(());
        };
        let text = serde_json::to_string(regs).map_err(|e| e.to_string())?;
        crate::owner_file::write_private(path, text.as_bytes())
    }

    /// Store a registration the phone sent.
    ///
    /// # Errors
    ///
    /// An invalid registration, or a store that cannot be written.
    pub async fn register(&self, reg: Registration) -> Result<(), String> {
        reg.validate()?;
        let mut regs = self.regs.write().await;
        let mut next = regs.clone();
        next.put(reg);
        self.persist(&next)?;
        *regs = next;
        drop(regs);
        Ok(())
    }

    pub async fn status(&self) -> Status {
        let regs = self.regs.read().await;
        let fcm_ready = self.fcm.is_some();
        Status {
            unifiedpush: regs.unifiedpush.is_some(),
            fcm: regs.fcm.is_some(),
            fcm_ready,
            route: plan(&regs, fcm_ready).first().map(Registration::transport),
        }
    }

    async fn try_route(&self, reg: &Registration, alert: &Alert) -> Result<(), Failure> {
        match reg {
            Registration::UnifiedPush { .. } => {
                let req = unifiedpush_request(reg, alert).map_err(Failure::Other)?;
                let answer = self.sender.post(req).await.map_err(Failure::Other)?;
                match answer.status {
                    s if http::is_success(s) => Ok(()),
                    GONE_NOT_FOUND | GONE => Err(Failure::Gone),
                    s => Err(Failure::Other(format!("distributor answered {s}"))),
                }
            }
            Registration::Fcm { token } => {
                let Some(client) = &self.fcm else {
                    return Err(Failure::Other("no FCM service account".into()));
                };
                client
                    .deliver(self.sender.as_ref(), token, &alert.json(), fcm::now_secs())
                    .await
            }
        }
    }

    /// Send one alert down the [`plan`], stopping at the first route that takes
    /// it. A route answering "gone" is dropped from the store.
    pub async fn send(&self, alert: &Alert) -> Delivery {
        let routes = {
            let regs = self.regs.read().await;
            plan(&regs, self.fcm.is_some())
        };
        let mut delivery = Delivery {
            via: None,
            failed: Vec::new(),
        };
        for reg in routes {
            let t = reg.transport();
            match self.try_route(&reg, alert).await {
                Ok(()) => {
                    delivery.via = Some(t);
                    break;
                }
                Err(Failure::Gone) => {
                    let mut regs = self.regs.write().await;
                    if regs.unifiedpush.as_ref() == Some(&reg) || regs.fcm.as_ref() == Some(&reg) {
                        let mut next = regs.clone();
                        next.drop_transport(t);
                        if self.persist(&next).is_ok() {
                            *regs = next;
                        }
                    }
                    drop(regs);
                    delivery
                        .failed
                        .push((t, "registration gone; dropped".into()));
                }
                Err(Failure::Other(why)) => delivery.failed.push((t, why)),
            }
        }
        delivery
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

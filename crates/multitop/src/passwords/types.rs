use crate::config::{validate_host, validate_user, Server, DEFAULT_PORT};
use secrecy::SecretString;

#[derive(Clone, Debug)]
pub struct ServerDraft {
    pub original: Option<usize>,
    pub host: String,
    pub user: String,
    pub port: String,
    pub upgrade_cmd: String,
    pub password: String,
    pub field: usize,
}

impl ServerDraft {
    pub fn new(original: Option<usize>, server: Option<&Server>, password: Option<&str>) -> Self {
        Self {
            original,
            host: server.map_or_else(String::new, |s| s.host.clone()),
            user: server.map_or_else(String::new, |s| s.user.clone()),
            port: server.map_or_else(|| DEFAULT_PORT.to_string(), |s| s.port.to_string()),
            upgrade_cmd: server
                .and_then(|s| s.upgrade_cmd.clone())
                .unwrap_or_default(),
            password: password.unwrap_or_default().to_string(),
            field: 0,
        }
    }

    pub const fn active_field(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.host,
            1 => &mut self.user,
            2 => &mut self.port,
            3 => &mut self.upgrade_cmd,
            _ => &mut self.password,
        }
    }

    /// Convert this draft into a validated `Server`.
    ///
    /// # Errors
    ///
    /// Returns an error if the host, user, or port fails validation, or if the
    /// port is out of range.
    pub fn into_server(self) -> Result<Server, String> {
        validate_host(&self.host).map_err(|error| error.0)?;
        validate_user(&self.user).map_err(|error| error.0)?;
        let port = self
            .port
            .parse::<u16>()
            .map_err(|_| "Port must be between 1 and 65535.".to_string())?;
        if port == 0 {
            return Err("Port must be between 1 and 65535.".to_string());
        }
        Ok(Server {
            host: self.host,
            user: self.user,
            port,
            upgrade_cmd: (!self.upgrade_cmd.trim().is_empty()).then_some(self.upgrade_cmd),
            custom_command: None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct PasswordManager {
    pub selected: usize,
    /// What the current text entry is for, if any.
    ///
    /// This replaced an `editing: bool` plus an `is_sso: bool`. Rotating the
    /// master password needs two prompts in sequence, and the second has to
    /// carry the first's answer -- which a pair of flags cannot express without
    /// a third flag and an untyped side channel.
    pub edit: Option<PasswordEdit>,
    pub input: String,
    pub resume_upgrade: bool,
    pub draft: Option<ServerDraft>,
    pub notice: Option<String>,
    /// Index awaiting confirmation for removal, if any.
    ///
    /// Deleting a server rewrites config.toml and cannot be undone, so it takes
    /// two keys. Running an upgrade already asks first; removing a server was
    /// the more destructive of the two and asked for nothing.
    pub pending_delete: Option<usize>,
    /// True while a master-password rotation is running off-thread.
    ///
    /// The rotation prompt closes the moment Enter is pressed, because the work
    /// happens elsewhere -- so the panel went straight back to accepting `r`,
    /// with nothing on screen but a one-line notice to say why it should not be
    /// pressed. `Vault::change_password` reads the vault, rewraps the key and
    /// writes it back; two of those overlapping both unlock with the *old*
    /// password, both write, and the last one wins. Both then report success,
    /// so the user is told twice that their master password changed when only
    /// one of the two actually does anything. A mistyped current password on
    /// each attempt also spends two of the kill-resistant limiter's tries.
    pub rotating: bool,
}

/// What a text prompt in the Passwords section is collecting.
#[derive(Clone, Debug)]
pub enum PasswordEdit {
    /// The current vault master password, on the way to changing it.
    RotateCurrent,
    /// The replacement, carrying the current one that was just verified by
    /// being typed. Held as a `SecretString` because it lives across two
    /// keystroke rounds rather than being consumed immediately.
    RotateNew { current: SecretString },
}

impl PasswordManager {
    /// Whether a text prompt is open, for rendering.
    #[must_use]
    pub const fn editing(&self) -> bool {
        self.edit.is_some()
    }

    #[must_use]
    pub const fn new(selected: usize, resume_upgrade: bool) -> Self {
        Self {
            selected,
            edit: None,
            input: String::new(),
            resume_upgrade,
            draft: None,
            notice: None,
            pending_delete: None,
            rotating: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PasswordAction {
    None,
    Save {
        panel: usize,
        password: String,
        resume_upgrade: bool,
    },
    /// Clear the stored password for a host, because its editor was saved with
    /// an empty password field.
    Delete {
        panel: usize,
    },
    /// Switch the panel banner between plain and wide glyphs.
    CycleBannerStyle,
    /// Add hosts from `~/.ssh/config` that are not configured yet.
    ImportSshHosts,
    /// Replace the vault master password. Both are plain `String` because they
    /// are handed straight to the vault and dropped.
    RotateVaultPassword {
        current: String,
        new: String,
    },
    ApplyServers(Vec<Server>),
    /// The result of editing or adding one row: the new server list, and what
    /// to do with that row's password. `None` means the field was left empty,
    /// so any stored password for the host is removed.
    ApplyServerEdit {
        servers: Vec<Server>,
        target_idx: usize,
        password: Option<String>,
    },
}

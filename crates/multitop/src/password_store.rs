//! Password persistence through the operating system credential store.
//!
//! macOS uses Keychain and Linux uses the Secret Service implementation
//! provided by the desktop session (for example GNOME Keyring or KWallet).

use keyring::{Entry, Error};

use crate::config::Server;

const SERVICE: &str = "multitop";

fn account(server: &Server) -> String {
    let user = if server.user.is_empty() {
        "default"
    } else {
        &server.user
    };
    format!("{user}@{}:{}", server.host, server.port)
}

fn entry(server: &Server) -> Result<Entry, String> {
    Entry::new(SERVICE, &account(server)).map_err(|error| error.to_string())
}

/// Read a password. A missing credential is not an error.
pub fn load(server: &Server) -> Result<Option<String>, String> {
    let entry = entry(server)?;
    match entry.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(Error::NoEntry) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub fn save(server: &Server, password: &str) -> Result<(), String> {
    entry(server)?
        .set_password(password)
        .map_err(|error| error.to_string())
}

pub fn delete(server: &Server) -> Result<(), String> {
    match entry(server)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

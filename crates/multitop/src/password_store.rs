//! Password persistence through the operating system credential store,
//! with an in-memory mock store for tests and CI environments.

use std::collections::HashMap;
use std::sync::RwLock;

use keyring::{Entry, Error};

use crate::config::Server;

const SERVICE: &str = "multitop";

static MOCK_STORE: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Enable in-memory mock store explicitly.
pub fn enable_mock_store() {
    let mut store = MOCK_STORE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    if store.is_none() {
        *store = Some(HashMap::new());
    }
}

/// Disable in-memory mock store explicitly.
pub fn disable_mock_store() {
    let mut store = MOCK_STORE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    *store = None;
}

/// Clear in-memory mock store contents.
pub fn clear_mock_store() {
    let mut store = MOCK_STORE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(map) = store.as_mut() {
        map.clear();
    }
}

pub fn is_mock_enabled() -> bool {
    if cfg!(test)
        || std::env::var("MULTITOP_MOCK_KEYCHAIN").is_ok()
        || std::env::var("CI").is_ok()
        || std::env::args().any(|arg| arg.contains("bench") || arg.contains("test"))
    {
        return true;
    }
    MOCK_STORE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some()
}

#[must_use]
pub fn account(server: &Server) -> String {
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

pub const SSO_ACCOUNT: &str = "__sso_master__";

#[allow(clippy::option_option)]
static SSO_CACHE: RwLock<Option<Option<String>>> = RwLock::new(None);

pub fn clear_sso_cache() {
    let mut cache = SSO_CACHE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    *cache = None;
}

/// Load the SSO master password from the credential store.
///
/// # Errors
///
/// Returns an error if the credential store cannot be accessed.
pub fn load_sso() -> Result<Option<String>, String> {
    {
        let cache = SSO_CACHE.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(ref cached) = *cache {
            return Ok(cached.clone());
        }
    }

    let pass = if is_mock_enabled() {
        enable_mock_store();
        let store = MOCK_STORE.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        store.as_ref().and_then(|map| map.get(SSO_ACCOUNT).cloned())
    } else {
        let Ok(entry) = Entry::new(SERVICE, SSO_ACCOUNT) else {
            return Ok(None);
        };
        entry.get_password().ok()
    };

    let mut cache = SSO_CACHE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    *cache = Some(pass.clone());
    drop(cache);
    Ok(pass)
}

/// Save the SSO master password to the credential store.
///
/// # Errors
///
/// Returns an error if the credential store cannot be written.
pub fn save_sso(password: &str) -> Result<(), String> {
    {
        let mut cache = SSO_CACHE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        *cache = Some(Some(password.to_string()));
        drop(cache);
    }
    if is_mock_enabled() {
        enable_mock_store();
        let mut store = MOCK_STORE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(map) = store.as_mut() {
            map.insert(SSO_ACCOUNT.to_string(), password.to_string());
        }
        drop(store);
        return Ok(());
    }
    Entry::new(SERVICE, SSO_ACCOUNT)
        .map_err(|e| e.to_string())?
        .set_password(password)
        .map_err(|e| e.to_string())
}

/// Delete the SSO master password from the credential store.
///
/// # Errors
///
/// Returns an error if the credential store cannot be accessed.
pub fn delete_sso() -> Result<(), String> {
    {
        let mut cache = SSO_CACHE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        *cache = Some(None);
        drop(cache);
    }
    if is_mock_enabled() {
        enable_mock_store();
        let mut store = MOCK_STORE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(map) = store.as_mut() {
            map.remove(SSO_ACCOUNT);
        }
        drop(store);
        return Ok(());
    }
    match Entry::new(SERVICE, SSO_ACCOUNT)
        .map_err(|e| e.to_string())?
        .delete_credential()
    {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

/// Read a password. A missing or locked credential store is not an error.
///
/// # Errors
///
/// Returns an error if the credential store cannot be accessed.
pub fn load(server: &Server) -> Result<Option<String>, String> {
    let server_pass = if is_mock_enabled() {
        enable_mock_store();
        let store = MOCK_STORE.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = account(server);
        store.as_ref().and_then(|map| map.get(&key).cloned())
    } else {
        let Ok(entry) = entry(server) else {
            return load_sso();
        };
        entry.get_password().ok()
    };

    if server_pass.is_some() {
        return Ok(server_pass);
    }

    load_sso()
}

/// Save a password for a server.
///
/// # Errors
///
/// Returns an error if the credential store cannot be written.
pub fn save(server: &Server, password: &str) -> Result<(), String> {
    if is_mock_enabled() {
        enable_mock_store();
        let mut store = MOCK_STORE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(map) = store.as_mut() {
            map.insert(account(server), password.to_string());
        }
        drop(store);
        return Ok(());
    }

    entry(server)?
        .set_password(password)
        .map_err(|error| error.to_string())
}

/// Delete a password for a server.
///
/// # Errors
///
/// Returns an error if the credential store cannot be accessed.
pub fn delete(server: &Server) -> Result<(), String> {
    if is_mock_enabled() {
        enable_mock_store();
        let mut store = MOCK_STORE.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(map) = store.as_mut() {
            map.remove(&account(server));
        }
        drop(store);
        return Ok(());
    }

    match entry(server)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn test_mock_keychain_operations() {
        enable_mock_store();
        clear_mock_store();

        let server = Server {
            host: "mock_host".into(),
            port: 22,
            user: "mock_user".into(),
            upgrade_cmd: None,
        };

        assert_eq!(load(&server).unwrap(), None);

        save(&server, "mock_pass_123").unwrap();
        assert_eq!(load(&server).unwrap().as_deref(), Some("mock_pass_123"));

        delete(&server).unwrap();
        assert_eq!(load(&server).unwrap(), None);
    }
}

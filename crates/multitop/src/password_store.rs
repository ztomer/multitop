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
    let mut store = MOCK_STORE.write().unwrap();
    if store.is_none() {
        *store = Some(HashMap::new());
    }
}

/// Disable in-memory mock store explicitly.
pub fn disable_mock_store() {
    let mut store = MOCK_STORE.write().unwrap();
    *store = None;
}

/// Clear in-memory mock store contents.
pub fn clear_mock_store() {
    let mut store = MOCK_STORE.write().unwrap();
    if let Some(map) = store.as_mut() {
        map.clear();
    }
}

fn is_mock_enabled() -> bool {
    if cfg!(test)
        || std::env::var("MULTITOP_MOCK_KEYCHAIN").is_ok()
        || std::env::var("CI").is_ok()
        || std::env::args().any(|arg| arg.contains("bench") || arg.contains("test"))
    {
        return true;
    }
    MOCK_STORE.read().map(|s| s.is_some()).unwrap_or(false)
}

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

/// Read a password. A missing or locked credential store is not an error.
pub fn load(server: &Server) -> Result<Option<String>, String> {
    if is_mock_enabled() {
        enable_mock_store();
        let store = MOCK_STORE.read().unwrap();
        let key = account(server);
        return Ok(store.as_ref().and_then(|map| map.get(&key).cloned()));
    }

    let entry = match entry(server) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    match entry.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(Error::NoEntry) => Ok(None),
        Err(_) => Ok(None),
    }
}

pub fn save(server: &Server, password: &str) -> Result<(), String> {
    if is_mock_enabled() {
        enable_mock_store();
        let mut store = MOCK_STORE.write().unwrap();
        if let Some(map) = store.as_mut() {
            map.insert(account(server), password.to_string());
        }
        return Ok(());
    }

    entry(server)?
        .set_password(password)
        .map_err(|error| error.to_string())
}

pub fn delete(server: &Server) -> Result<(), String> {
    if is_mock_enabled() {
        enable_mock_store();
        let mut store = MOCK_STORE.write().unwrap();
        if let Some(map) = store.as_mut() {
            map.remove(&account(server));
        }
        return Ok(());
    }

    match entry(server)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
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

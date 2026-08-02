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
    let mut store = MOCK_STORE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if store.is_none() {
        *store = Some(HashMap::new());
    }
}

/// Disable in-memory mock store explicitly.
pub fn disable_mock_store() {
    let mut store = MOCK_STORE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *store = None;
}

/// Clear in-memory mock store contents.
pub fn clear_mock_store() {
    let mut store = MOCK_STORE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(map) = store.as_mut() {
        map.clear();
    }
}

/// Serializes tests that touch the process-global credential state.
///
/// `MOCK_STORE` and `SSO_CACHE` are process-global, and the test harness runs
/// `#[test]` bodies on parallel threads. A test that resets the store during
/// setup would otherwise wipe it out from under a concurrently running test —
/// the resulting failures are nondeterministic and blame the wrong test.
/// Async-aware so that `#[tokio::test]` bodies can hold the guard across an
/// `.await` — a `std::sync::Mutex` guard cannot cross one safely.
static TEST_GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Acquire exclusive access to the global mock store and SSO cache from a
/// synchronous `#[test]`. Blocks the calling thread.
///
/// Hold the returned guard for the whole test body — dropping it early
/// re-opens the race.
#[doc(hidden)]
pub fn lock_for_test() -> tokio::sync::MutexGuard<'static, ()> {
    TEST_GUARD.blocking_lock()
}

/// Acquire exclusive access to the global mock store and SSO cache from an
/// `#[tokio::test]`. Yields rather than blocking the runtime.
///
/// Hold the returned guard for the whole test body — dropping it early
/// re-opens the race.
#[doc(hidden)]
pub async fn lock_for_test_async() -> tokio::sync::MutexGuard<'static, ()> {
    TEST_GUARD.lock().await
}

/// Decide whether the in-memory mock store stands in for the OS keychain.
///
/// Split out so the rule is testable, and because the rule used to be wrong in
/// a way no test could see.
#[must_use]
const fn mock_enabled_from(cfg_test: bool, env_mock: bool) -> bool {
    cfg_test || env_mock
}

/// Whether credentials go to the in-memory mock store instead of the OS
/// keychain.
///
/// # The process arguments are deliberately not consulted
///
/// `CI` used to count as a signal too. It is somebody else's environment
/// variable, set for reasons that have nothing to do with this program, and a
/// release binary that saw it silently sent every password to a store that
/// evaporates on exit -- the same failure as the argument sniffing below, from
/// a different direction. Integration tests call `enable_mock_store` directly,
/// so nothing needed it.
///
/// The argument list used to count as well: it returned true when *any*
/// argument contained "test" or "bench", intended to spot a test or bench
/// harness. In a release binary it
/// read the real command line, so `--remote latest.example.com` enabled the
/// mock store -- "latest" contains "test". So did any config path with "test"
/// in it. Passwords then went to an in-memory store that vanished on exit, and
/// the user was asked for them again on every launch with nothing to explain
/// why. `cfg!(test)` already covers anything built by the test harness, and a
/// bench that wants the mock can set `MULTITOP_MOCK_KEYCHAIN` for itself.
pub fn is_mock_enabled() -> bool {
    if mock_enabled_from(cfg!(test), std::env::var("MULTITOP_MOCK_KEYCHAIN").is_ok()) {
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum SsoCacheState {
    Uncached,
    NotFound,
    Found(String),
}

static SSO_CACHE: RwLock<SsoCacheState> = RwLock::new(SsoCacheState::Uncached);

pub fn clear_sso_cache() {
    let mut cache = SSO_CACHE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *cache = SsoCacheState::Uncached;
}

/// Load the SSO master password from the credential store.
///
/// # Errors
///
/// Returns an error if the credential store cannot be accessed.
pub fn load_sso() -> Result<Option<String>, String> {
    {
        let cache = SSO_CACHE
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &*cache {
            SsoCacheState::Found(p) => return Ok(Some(p.clone())),
            SsoCacheState::NotFound => return Ok(None),
            SsoCacheState::Uncached => {}
        }
    }

    let pass = if is_mock_enabled() {
        enable_mock_store();
        let store = MOCK_STORE
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        store.as_ref().and_then(|map| map.get(SSO_ACCOUNT).cloned())
    } else {
        let Ok(entry) = Entry::new(SERVICE, SSO_ACCOUNT) else {
            return Ok(None);
        };
        entry.get_password().ok()
    };

    let mut cache = SSO_CACHE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *cache = pass.as_ref().map_or_else(
        || SsoCacheState::NotFound,
        |p| SsoCacheState::Found(p.clone()),
    );
    drop(cache);
    Ok(pass)
}

/// Save the SSO master password to the credential store.
///
/// # Errors
///
/// Returns an error if the credential store cannot be written.
pub fn save_sso(password: &str) -> Result<(), String> {
    // The cache is updated only after the store accepts the write. Recording it
    // first meant a failed save still served the password for the rest of the
    // session, so the user was told it had not been saved and then saw it work
    // until they restarted.
    if is_mock_enabled() {
        enable_mock_store();
        let mut store = MOCK_STORE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(map) = store.as_mut() {
            map.insert(SSO_ACCOUNT.to_string(), password.to_string());
        }
        drop(store);
        cache_sso(Some(password));
        return Ok(());
    }
    Entry::new(SERVICE, SSO_ACCOUNT)
        .map_err(|e| e.to_string())?
        .set_password(password)
        .map_err(|e| e.to_string())?;
    cache_sso(Some(password));
    Ok(())
}

/// Record the SSO cache state. Call only after the store has agreed.
fn cache_sso(password: Option<&str>) {
    let mut cache = SSO_CACHE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *cache = password.map_or(SsoCacheState::NotFound, |p| {
        SsoCacheState::Found(p.to_string())
    });
}

/// Delete the SSO master password from the credential store.
///
/// # Errors
///
/// Returns an error if the credential store cannot be accessed.
pub fn delete_sso() -> Result<(), String> {
    // Same rule as `save_sso`: the cache follows the store, never leads it.
    if is_mock_enabled() {
        enable_mock_store();
        let mut store = MOCK_STORE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(map) = store.as_mut() {
            map.remove(SSO_ACCOUNT);
        }
        drop(store);
        cache_sso(None);
        return Ok(());
    }
    match Entry::new(SERVICE, SSO_ACCOUNT)
        .map_err(|e| e.to_string())?
        .delete_credential()
    {
        // Gone either way, so the cache must say so. Missing this is how
        // "the cache follows the store" turns into the cache serving a
        // password that was just deleted, for the rest of the session.
        Ok(()) | Err(Error::NoEntry) => {
            cache_sso(None);
            Ok(())
        }
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
        let store = MOCK_STORE
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let mut store = MOCK_STORE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let mut store = MOCK_STORE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

#[cfg(test)]
mod mock_signal_tests {
    use super::mock_enabled_from;

    /// Only explicit signals may divert credentials away from the OS keychain.
    ///
    /// The regression this guards: the rule once also inspected the process
    /// arguments, so a release binary run as `--remote latest.example.com`
    /// silently used the mock store, because "latest" contains "test". The
    /// signature is the guarantee -- there is nowhere for argv to enter.
    #[test]
    fn only_explicit_signals_enable_the_mock_store() {
        assert!(!mock_enabled_from(false, false), "a real run");
        assert!(mock_enabled_from(true, false), "the test harness");
        assert!(mock_enabled_from(false, true), "explicit env opt-in");
        // CI is deliberately absent: it is somebody else's environment
        // variable, and honouring it sent a release binary's passwords to a
        // store that evaporates on exit. The signature is the guarantee --
        // there is nowhere for it to enter.
    }
}

#[cfg(test)]
mod sso_cache_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// The cache must follow the store, never lead it.
    ///
    /// `save_sso` used to record `Found` before attempting the write and
    /// `delete_sso` recorded `NotFound` before attempting the delete. On
    /// failure the user was told it had not worked, and then watched it appear
    /// to work for the rest of the session, because every later read came from
    /// a cache that had already committed to the outcome.
    #[tokio::test]
    async fn the_sso_cache_reflects_what_the_store_accepted() {
        let _guard = lock_for_test_async().await;
        enable_mock_store();
        clear_mock_store();
        let _ = delete_sso();

        assert_eq!(load_sso().unwrap(), None, "starts empty");

        save_sso("master-1").unwrap();
        assert_eq!(load_sso().unwrap().as_deref(), Some("master-1"));

        // A delete must be visible immediately, not just after a restart.
        delete_sso().unwrap();
        assert_eq!(
            load_sso().unwrap(),
            None,
            "a completed delete must not keep serving the old password"
        );

        // And a re-save is visible again.
        save_sso("master-2").unwrap();
        assert_eq!(load_sso().unwrap().as_deref(), Some("master-2"));
    }
}

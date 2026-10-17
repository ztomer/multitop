//! The `--serve` token: how it is made, and how it is compared.
//!
//! This is the only thing between the network and every host's metrics once
//! `multitop --serve 0.0.0.0:8080` is running — the bind the phone companion needs.
//! On loopback it does not matter. On a LAN it is the whole of the authentication.
//!
//! **What was wrong here, 2026-10-14 (ROADMAP P0.1).** The token was generated
//! with `DefaultHasher` over the process id and a nanosecond timestamp. That is
//! `SipHash` with FIXED keys, so the token was a deterministic function of two
//! values an attacker can bound — the pid is small and the start time is roughly
//! known — and it was 64 bits, half the floor for a credential. The generator's own
//! comment claimed a rule it did not implement ("only when binding to loopback ...
//! For now, generate if not supplied").
//!
//! Three defects, one place, fixed together:
//!
//! 1. **Source.** At least 128 bits from the OS CSPRNG. Never derived from time
//!    or pid: both are observable, and a credential built from observable inputs
//!    is a guessable credential however well it is hashed.
//! 2. **Comparison.** `got == expected` short-circuits on the first differing
//!    byte, so a caller who can time responses learns the token a byte at a time.
//!    [`matches`] compares in constant time.
//! 3. **State.** The token was `Option<String>` and "no token" meant "no
//!    authentication" — see [`crate::server::check_auth`], which no longer takes
//!    one.
//!
//! **Where it lives and who sees it (ROADMAP A1, 2026-10-17).** The token reached
//! three readers besides its owner: every local user, through `ps`, when it was
//! passed as `--serve-token <TOKEN>`; whatever log a service manager keeps of
//! stderr, which carried `token=<t>` and a ready `curl` line on every start; and the
//! phone, which had to be re-paired after every restart because a fresh token was
//! made each time. It now lives in ONE file, `0600`, created on first use and read
//! on every start after ([`load_or_create`]); a file anyone else can read is refused,
//! the way `ssh` refuses a key. The token is printed only when stderr is a terminal
//! ([`startup_banner`]) -- a log gets the file's path, never its contents.

use std::fmt::Write as _;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use subtle::ConstantTimeEq;

/// Bytes of entropy per generated token: 32 bytes = 256 bits, twice the 128-bit
/// floor. Hex-encoded to 64 characters.
const TOKEN_BYTES: usize = 32;

/// A fresh token, hex-encoded, from the OS CSPRNG.
///
/// # Errors
///
/// When the OS CSPRNG fails, and only then. There is no fallback, and that is
/// deliberate: the alternatives are a weaker source or a token derived from
/// something observable, and a `--serve` on a LAN with a guessable token is worse
/// than a refusal to start. The error is returned rather than panicked on, so the
/// one caller can print something useful and exit non-zero — a panic in a CLI that
/// has just bound a port is a worse report than a sentence.
pub fn generate() -> Result<String, std::io::Error> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)?;
    // `write!` rather than `format!` per byte: this runs once per server start, and
    // the map-and-collect spelling is both an allocation per byte and a clippy error
    // for building a string from an iterator.
    let mut out = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        // Writing into a String cannot fail; `write!` returns a Result either way.
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

/// True when `got` is `expected`, in time that does not depend on where they first
/// differ.
///
/// Length is compared first and returns early, which leaks only the length — and
/// both values here are fixed-length hex, so that is not a secret. Everything beyond
/// it runs to completion regardless of the contents.
#[must_use]
pub fn matches(expected: &str, got: &str) -> bool {
    if expected.len() != got.len() {
        return false;
    }
    expected.as_bytes().ct_eq(got.as_bytes()).into()
}

/// The shortest token a file may hold: 32 characters, which is 128 bits when they
/// are hex, the floor [`generate`] doubles. A hand-written token shorter than this
/// is refused rather than served.
pub const MIN_TOKEN_LEN: usize = 32;

/// Where the token lives when `--serve-token-file` does not say: beside the config
/// file, so two configs are two servers with two tokens.
#[must_use]
pub fn default_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("serve-token")
}

/// The token at `path`, and whether this call created it.
///
/// An existing file is read, and refused if anyone but its owner can read or write
/// it (`chmod 600` is the fix the error names), or if it holds fewer than
/// [`MIN_TOKEN_LEN`] visible ASCII characters. An absent file is created `0600`,
/// exclusively, holding a fresh [`generate`]d token; its directory is created
/// `0700` if it does not exist.
///
/// # Errors
///
/// A sentence naming `path` and what to do, for every refusal above and for any
/// I/O failure. There is no fallback to a token that lives nowhere: a server whose
/// token the owner cannot find is a server the phone cannot pair with.
pub fn load_or_create(path: &Path) -> Result<(String, bool), String> {
    match std::fs::metadata(path) {
        Ok(meta) => read_existing(path, meta.permissions().mode()).map(|t| (t, false)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => create(path),
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

fn read_existing(path: &Path, mode: u32) -> Result<String, String> {
    let shown = path.display();
    if mode & 0o077 != 0 {
        return Err(format!(
            "{shown} is open to other users (mode {:03o}); run: chmod 600 {shown}",
            mode & 0o777
        ));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {shown}: {e}"))?;
    let token = text.trim();
    if token.len() < MIN_TOKEN_LEN || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(format!(
            "{shown} must hold one token of at least {MIN_TOKEN_LEN} visible ASCII \
             characters; delete it and --serve writes a fresh 256-bit one"
        ));
    }
    Ok(token.to_string())
}

fn create(path: &Path) -> Result<(String, bool), String> {
    let shown = path.display();
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let token = generate().map_err(|e| format!("could not read the OS CSPRNG ({e})"))?;
    // `create_new`: never truncate a token another start wrote between the metadata
    // probe and here. Losing that race reads the winner's token instead.
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return load_or_create(path);
        }
        Err(e) => return Err(format!("cannot create {shown}: {e}")),
    };
    file.write_all(format!("{token}\n").as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("cannot write {shown}: {e}"))?;
    Ok((token, true))
}

/// What `--serve` prints on start; the token only when `to_tty`.
///
/// A person at a terminal is the token's owner, a service manager's log is not.
/// Either way the file is named, which is all the owner needs to find it.
#[must_use]
pub fn startup_banner(
    addr: std::net::SocketAddr,
    token: &str,
    path: &Path,
    created: bool,
    to_tty: bool,
) -> String {
    let file = path.display();
    let fresh = if created { " (new)" } else { "" };
    if to_tty {
        format!(
            "multitop --serve on http://{addr} token={token}\n  \
             token file: {file}{fresh}\n  \
             curl -H \"Authorization: Bearer {token}\" http://{addr}/api/hosts"
        )
    } else {
        format!("multitop --serve on http://{addr}\n  token file: {file}{fresh}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `generate` is infallible on every platform this ships to, so the tests unwrap
    /// where production propagates. `unwrap_used` is denied workspace-wide; the deny
    /// is about a library silently guessing, and a test that cannot get randomness is
    /// not something to keep running past.
    fn token() -> String {
        match generate() {
            Ok(t) => t,
            Err(e) => panic!("the OS CSPRNG failed in a test: {e}"),
        }
    }

    #[test]
    fn a_generated_token_carries_at_least_128_bits() {
        // RED before the fix: the old generator returned `{:016x}` of a u64 — 16
        // hex characters, 64 bits, half the floor.
        let token = token();
        assert!(
            token.len() >= 32,
            "token is {} hex chars ({token:?}) — {} bits, below the 128-bit floor",
            token.len(),
            token.len() * 4
        );
    }

    #[test]
    fn a_generated_token_is_hex() {
        assert!(token().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_generated_token_is_the_length_it_promises() {
        // The hex arm is a per-byte loop with a discarded Result. If `write!` ever
        // stopped advancing, this is what notices — a short token is the failure that
        // the entropy floor test would otherwise report as a weaker floor.
        assert_eq!(token().len(), TOKEN_BYTES * 2);
    }

    #[test]
    fn two_generated_tokens_differ() {
        // Not a claim about the CSPRNG's quality — a claim that it was consulted at
        // all. The old generator was a function of pid and a timestamp, so two calls
        // collided only if they shared a pid AND landed in the same nanosecond; that
        // is "unlikely", not "impossible".
        assert_ne!(token(), token());
    }

    #[test]
    fn the_right_token_matches() {
        assert!(matches("deadbeef", "deadbeef"));
    }

    #[test]
    fn a_wrong_token_does_not_match() {
        assert!(!matches("deadbeef", "deadbeee"));
        assert!(!matches("deadbeef", "beefdead"));
    }

    #[test]
    fn a_different_length_never_matches() {
        assert!(!matches("deadbeef", "deadbeeff"));
        assert!(!matches("deadbeef", "dead"));
        assert!(!matches("", "x"));
    }

    #[test]
    fn an_empty_token_does_not_match_a_real_one() {
        // The shape a bare `Authorization: Bearer ` produces: the prefix strips and
        // nothing is left.
        assert!(!matches("deadbeef", ""));
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        match tempfile::tempdir() {
            Ok(d) => d,
            Err(e) => panic!("no temp dir: {e}"),
        }
    }

    fn mode(path: &Path) -> u32 {
        match std::fs::metadata(path) {
            Ok(m) => m.permissions().mode() & 0o777,
            Err(e) => panic!("no metadata for {}: {e}", path.display()),
        }
    }

    fn set_mode(path: &Path, mode: u32) {
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)) {
            panic!("chmod {}: {e}", path.display());
        }
    }

    #[test]
    fn a_missing_file_is_created_owner_only_with_a_generated_token() {
        // The file is the credential, so its mode is the whole point: a token written
        // 0644 is a token every local user can read, which is the `ps` leak again.
        let d = dir();
        let path = d.path().join("nested/serve-token");
        let (token, created) = load_or_create(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(created);
        assert_eq!(token.len(), TOKEN_BYTES * 2);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&d.path().join("nested")), 0o700);
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(on_disk, format!("{token}\n"));
    }

    #[test]
    fn a_restart_serves_the_same_token() {
        // The phone pairs once. A token that changed on every start re-paired it on
        // every restart, which is what made printing the token on start seem needed.
        let d = dir();
        let path = d.path().join("serve-token");
        let first = load_or_create(&path).unwrap_or_else(|e| panic!("{e}"));
        let second = load_or_create(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(first.0, second.0);
        assert!(!second.1, "the second start claims it created the file");
    }

    #[test]
    fn a_token_file_others_can_open_is_refused() {
        let d = dir();
        let path = d.path().join("serve-token");
        let _ = load_or_create(&path).unwrap_or_else(|e| panic!("{e}"));
        for open in [0o644, 0o640, 0o604, 0o620] {
            set_mode(&path, open);
            let Err(e) = load_or_create(&path) else {
                panic!("mode {open:o} was accepted");
            };
            assert!(e.contains("chmod 600"), "{e}");
        }
        set_mode(&path, 0o400);
        assert!(load_or_create(&path).is_ok(), "owner read-only is fine");
    }

    #[test]
    fn a_hand_written_token_must_be_long_and_visible() {
        let d = dir();
        let path = d.path().join("serve-token");
        let long = "a".repeat(MIN_TOKEN_LEN);
        for (body, ok) in [
            (format!("{long}\n"), true),
            (long[1..].to_string(), false),
            (format!("{} {}", &long[..16], &long[..16]), false),
            (String::new(), false),
        ] {
            if let Err(e) = std::fs::write(&path, &body) {
                panic!("write: {e}");
            }
            set_mode(&path, 0o600);
            assert_eq!(load_or_create(&path).is_ok(), ok, "{body:?}");
        }
    }

    #[test]
    fn the_banner_shows_the_token_to_a_terminal_and_never_to_a_log() {
        // Before A1, `--serve` printed `token=<t>` and a curl line carrying it to
        // stderr unconditionally, so a service manager's log of stderr held the
        // credential in plain text on every start. Calibrated: a banner that ignores
        // `to_tty` -- that old behaviour -- turns this red.
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 8080));
        let token = "f".repeat(64);
        let path = Path::new("/home/u/.config/multitop/serve-token");
        for created in [true, false] {
            let log = startup_banner(addr, &token, path, created, false);
            assert!(
                !log.contains(&token),
                "a non-tty banner carries the token: {log}"
            );
            assert!(log.contains(&path.display().to_string()), "{log}");
            let tty = startup_banner(addr, &token, path, created, true);
            assert!(tty.contains(&token), "{tty}");
            assert!(tty.contains(&path.display().to_string()), "{tty}");
        }
    }
}

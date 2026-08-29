//! Building the shell command the child actually runs.
//!
//! Separated from the runner because this is a quoting problem, not an I/O one,
//! and quoting is exactly the kind of thing that should be readable on its own
//! and testable without a pty.

use std::ffi::CString;

use super::{
    DONE_SENTINEL, PW_READY_SENTINEL, STARTED_SENTINEL, SUDO_FAILED_CODE, SUDO_FAILED_SENTINEL,
};

/// The shell command the child runs.
///
/// The preamble is unchanged in shape from the one it replaces, and unchanged
/// in the reason: the password is never in argv, because `/proc/<pid>/cmdline`
/// is world-readable on Linux and every account on a monitored host could read
/// it for the length of a run. `read` puts it in a shell variable and `printf`
/// is a builtin, so no process is spawned that could carry it.
///
/// The login-and-interactive shells are what make an alias like `ud` resolve,
/// which is what most people actually put in `upgrade_cmd`.
pub fn wrap(command: &str, with_password: bool) -> String {
    let quoted = sh_quote(command);
    let inner = quoted.replace('\'', r"'\''");
    // The command is bracketed between two bare-word markers. Bare words on
    // purpose: they pass through three levels of shell quoting unchanged.
    //
    // `__mt_rc` carries the command's own status across the closing marker.
    // Without it the `echo` would be the last command in the shell and its
    // success would become the run's -- every upgrade reported as having
    // worked, whatever it did.
    let bracket = format!(
        "echo {STARTED_SENTINEL}; eval {inner}; __mt_rc=$?; echo {DONE_SENTINEL}; exit $__mt_rc"
    );
    let body = format!(
        "if command -v zsh >/dev/null 2>&1; then \
           zsh -l -i -c 'setopt expand_aliases 2>/dev/null; source ~/.zshrc 2>/dev/null; source ~/.zprofile 2>/dev/null; {bracket}'; \
         elif command -v bash >/dev/null 2>&1; then \
           bash -i -c 'shopt -s expand_aliases 2>/dev/null; source ~/.bashrc 2>/dev/null; source ~/.bash_profile 2>/dev/null; {bracket}'; \
         else sh -c '{bracket}'; fi"
    );
    if !with_password {
        return body;
    }
    format!(
        "stty -echo 2>/dev/null; printf '{PW_READY_SENTINEL}\\n'; IFS= read -r __mt_pw; \
         stty echo 2>/dev/null; printf '%s\\n' \"$__mt_pw\" | sudo -S -p '' -v 2>/dev/null; \
         __mt_rc=$?; unset __mt_pw; \
         if [ $__mt_rc -ne 0 ]; then printf '{SUDO_FAILED_SENTINEL}\\n'; exit {SUDO_FAILED_CODE}; fi; \
         {body}"
    )
}

/// `/bin/sh -c <script>`, as C strings built before any fork.
pub fn shell_argv(script: &str) -> Option<Vec<CString>> {
    Some(vec![
        CString::new("/bin/sh").ok()?,
        CString::new("-c").ok()?,
        CString::new(script).ok()?,
    ])
}

/// Single-quote for a POSIX shell.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

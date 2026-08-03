//! Importing from `~/.ssh/config` adds hosts and changes nothing else.
//!
//! The parser existed and was tested with no caller. The part that is easy to
//! get wrong is not parsing but the merge: an SSH config carries no
//! `upgrade_cmd` and no password, so treating it as authoritative would wipe
//! both for every host it happens to mention, and dropping servers it does not
//! mention would delete hosts the user configured by hand.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::config::{merge_ssh_hosts, parse_ssh_config, Server};

fn server(host: &str, user: &str, port: u16, cmd: Option<&str>) -> Server {
    Server {
        host: host.to_string(),
        port,
        user: user.to_string(),
        upgrade_cmd: cmd.map(ToString::to_string),
    }
}

const SSH_CONFIG: &str = "\
Host media
    HostName 192.168.0.33
    User ztomer
    Port 22

Host spare
    HostName 192.168.0.90
    User ztomer

Host *
    ServerAliveInterval 60
";

#[test]
fn new_hosts_are_added() {
    let existing = vec![server("192.168.0.33", "ztomer", 22, Some("us;ud"))];
    let (merged, added) = merge_ssh_hosts(&existing, parse_ssh_config(SSH_CONFIG));

    assert_eq!(added, 1, "only the spare box is new");
    assert_eq!(merged.len(), 2);
    assert!(merged.iter().any(|s| s.host == "192.168.0.90"));
}

#[test]
fn an_existing_host_keeps_its_upgrade_command() {
    let existing = vec![server("192.168.0.33", "ztomer", 22, Some("us;ud"))];
    let (merged, _) = merge_ssh_hosts(&existing, parse_ssh_config(SSH_CONFIG));

    let kept = merged
        .iter()
        .find(|s| s.host == "192.168.0.33")
        .expect("the configured host must survive");
    assert_eq!(
        kept.upgrade_cmd.as_deref(),
        Some("us;ud"),
        "the SSH config has no upgrade_cmd; importing must not erase the one that exists"
    );
}

#[test]
fn servers_absent_from_the_ssh_config_are_left_alone() {
    let existing = vec![
        server("192.168.0.158", "ztomer", 22, Some("./update_sys.sh")),
        server("127.0.0.1", "ztomer", 22, Some("update-local")),
    ];
    let (merged, added) = merge_ssh_hosts(&existing, parse_ssh_config(SSH_CONFIG));

    assert_eq!(added, 2);
    assert!(
        merged.iter().any(|s| s.host == "192.168.0.158"),
        "a host the SSH config does not mention must not be dropped"
    );
    assert!(merged.iter().any(|s| s.host == "127.0.0.1"));
    assert_eq!(merged.len(), 4);
}

#[test]
fn the_same_machine_under_a_different_account_is_a_different_server() {
    // Already configured as root; the SSH config offers ztomer on the same box.
    let existing = vec![server("192.168.0.33", "root", 22, None)];
    let (merged, added) = merge_ssh_hosts(&existing, parse_ssh_config(SSH_CONFIG));

    assert_eq!(
        added, 2,
        "matching on host alone would have skipped ztomer@192.168.0.33"
    );
    assert!(merged
        .iter()
        .any(|s| s.host == "192.168.0.33" && s.user == "ztomer"));
    assert!(merged
        .iter()
        .any(|s| s.host == "192.168.0.33" && s.user == "root"));
}

#[test]
fn importing_twice_adds_nothing_the_second_time() {
    let existing: Vec<Server> = Vec::new();
    let (once, first) = merge_ssh_hosts(&existing, parse_ssh_config(SSH_CONFIG));
    let (twice, second) = merge_ssh_hosts(&once, parse_ssh_config(SSH_CONFIG));

    assert_eq!(first, 2);
    assert_eq!(second, 0, "a second import must be a no-op");
    assert_eq!(twice.len(), once.len());
}

#[test]
fn wildcard_blocks_are_not_imported_as_hosts() {
    let (merged, _) = merge_ssh_hosts(&[], parse_ssh_config(SSH_CONFIG));
    assert!(
        !merged.iter().any(|s| s.host.contains('*')),
        "`Host *` is a defaults block, not a machine: {merged:?}"
    );
}

#[test]
fn ordering_puts_imports_after_what_was_already_there() {
    let existing = vec![server("127.0.0.1", "ztomer", 22, None)];
    let (merged, _) = merge_ssh_hosts(&existing, parse_ssh_config(SSH_CONFIG));

    assert_eq!(
        merged[0].host, "127.0.0.1",
        "existing panels must keep their positions, or the user's layout shuffles"
    );
}

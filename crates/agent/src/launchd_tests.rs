//! launchd, from the shapes this Mac printed (2026-10-10).

use super::*;

const LIST: &str = "PID\tStatus\tLabel\n\
-\t0\tcom.apple.SafariHistoryServiceAgent\n\
5291\t0\tcom.antiknob.daemon\n\
-\t78\tcom.divoom.monthlybest\n\
-\t-9\tcom.example.killed\n\
-\t0\tcom.ztomer.prune-logs\n";

#[test]
fn the_list_reads_pid_status_and_label_and_skips_its_header() {
    let rows = parse_list(LIST);
    assert_eq!(rows.len(), 5, "{rows:?}");
    assert_eq!(
        rows[1],
        Row {
            pid: Some(5291),
            status: 0,
            label: "com.antiknob.daemon".into()
        }
    );
    assert_eq!(rows[2].pid, None);
    assert_eq!(rows[2].status, 78);
    assert_eq!(rows[3].status, -9, "a signal is a negative status");
}

#[test]
fn etime_reads_every_form_ps_prints() {
    assert_eq!(parse_etime("00:05"), Some(5));
    assert_eq!(parse_etime("12:34"), Some(12 * 60 + 34));
    assert_eq!(parse_etime("01:02:03"), Some(3723));
    assert_eq!(
        parse_etime("09-01:58:29"),
        Some(((9 * 24 + 1) * 60 + 58) * 60 + 29)
    );
    assert_eq!(parse_etime("nonsense"), None);
    assert_eq!(parse_etime("1:2:3:4"), None);
}

#[test]
fn only_the_users_loaded_agents_are_jobs_in_their_order() {
    let labels: Vec<String> = [
        "com.divoom.monthlybest",
        "com.local.lockscreen", // a plist that is not loaded
        "com.antiknob.daemon",
    ]
    .map(String::from)
    .to_vec();
    let ages = parse_etimes(" 5291 01:00:00\n");
    let v = jobs(&labels, &parse_list(LIST), &ages, 10_000);
    let jobs = v["jobs"].as_array().unwrap();
    let names: Vec<&str> = jobs.iter().map(|j| j["label"].as_str().unwrap()).collect();
    assert_eq!(names, ["com.divoom.monthlybest", "com.antiknob.daemon"]);
    assert_eq!(jobs[0]["last_rc"], 78);
    assert!(jobs[0]["running_since"].is_null());
    assert_eq!(jobs[1]["running_since"], 10_000 - 3600);
    assert!(jobs[1]["alert_since"].is_null(), "nothing on a Mac alerts");
}

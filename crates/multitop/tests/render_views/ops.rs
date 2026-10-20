//! The Ops view's screen for `render_views` (servers ROADMAP 12.17b).

use super::{base, App};

/// The Ops view: a troubled host, a clean one without Docker, one whose
/// session broke (under its last answer) and one with no `mcp` command.
pub fn ops(term: (u16, u16)) -> App {
    use multitop::ops::{
        Check, Container, DockerHealth, DockerState, Job, OpsState, Part, Snapshot, ZfsState,
    };
    let mut app = base(4, term);
    for p in app.panels.iter_mut().take(3) {
        p.server.mcp = Some("cd ~/prj/x && bin/mcp_host --root .".to_string());
    }
    let dims = multitop::ui::agent_dims(
        ratatui::layout::Size {
            width: term.0,
            height: term.1,
        },
        4,
    );
    let at = multitop::tasks::ops_poll::now_epoch() - 12;
    let check = |n: &str, ok| Check {
        name: format!("native:{n}"),
        instance: Some("media_server".into()),
        ok,
        rc: i64::from(!ok),
        ts: at - 900,
        age: Some("15 min ago".into()),
    };
    let health = |checks: Vec<multitop::ops::Check>| multitop::ops::Health {
        checks,
        oldest_age: Some("15 min ago".into()),
        oldest_age_s: Some(900),
    };
    let job = |l: &str, rc| Job {
        label: l.into(),
        instance: None,
        last_start: Some(at - 7200),
        last_rc: Some(rc),
        running_since: None,
        alert_since: None,
    };
    let ct = |n: &str, h: &str| Container {
        name: n.into(),
        state: DockerState::Running,
        status: "Up 2 hours".into(),
        health: DockerHealth::from_word(h),
        instance: Some("m".into()),
        compose_project: Some("m".into()),
    };
    let troubled = Snapshot {
        server: ("mcp_host".into(), "1.65.2".into()),
        at,
        health: Part::Ready(health(vec![
            check("check_disk_space", true),
            check("check_vpn_leak", false),
        ])),
        vpn: Part::Absent("no VPN stack on this host".into()),
        cron: Part::Ready(vec![job("backup", 1), job("healthcheck", 0)]),
        containers: Part::Ready(vec![ct("gluetun", "unhealthy"), ct("jellyfin", "healthy")]),
        zfs: Part::Ready(vec![multitop::ops::Pool {
            name: "pool0".into(),
            state: ZfsState::Suspended,
            action: "attach the missing device".into(),
            msg: "One or more devices could not be opened".into(),
            members: vec![multitop::ops::Member {
                name: "sdc".into(),
                state: ZfsState::Removed,
            }],
        }]),
        disk: Part::Absent("this host lists no disk.usage".into()),
        facts: Part::Absent("this host lists no host.facts".into()),
        alerts: Part::Ready(vec![multitop::ops::Alert {
            ts: at - 60,
            subject: "[media_server] backup FAILED (rc=1)".into(),
        }]),
    };
    let mut clean = troubled.clone();
    clean.health = Part::Ready(health(vec![check("check_disk_space", true)]));
    clean.cron = Part::Ready(vec![job("healthcheck", 0)]);
    clean.containers = Part::Absent("no Docker on this host".into());
    clean.alerts = Part::Ready(vec![]);
    let _ = app.toggle_ops(dims);
    app.panels[0].last_ops = Some(OpsState::Ready(Box::new(troubled)));
    app.panels[1].last_ops = Some(OpsState::Ready(Box::new(clean.clone())));
    app.panels[2].last_ops = Some(OpsState::Failed {
        why: "ssh: connect to host cache-03 port 22: Connection refused".into(),
        last: Some(Box::new(clean)),
    });
    app.rerender_all(dims);
    app
}

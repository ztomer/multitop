//! The Ops view's screen for `render_views` (servers ROADMAP 12.17b).

use super::{base, App};

/// The Ops view: a troubled host, a clean one without Docker, one whose
/// session broke (under its last answer) and one with no `mcp` command.
pub fn ops(term: (u16, u16)) -> App {
    use multitop::ops::{Check, Container, Job, OpsState, Part, Snapshot};
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
        ok,
        rc: i64::from(!ok),
        ts: at - 900,
    };
    let job = |l: &str, rc| Job {
        label: l.into(),
        last_start: Some(at - 7200),
        last_rc: Some(rc),
        running_since: None,
        alert_since: None,
    };
    let ct = |n: &str, h: &str| Container {
        name: n.into(),
        state: "running".into(),
        status: "Up 2 hours".into(),
        health: h.into(),
        compose_project: Some("m".into()),
    };
    let troubled = Snapshot {
        server: ("mcp_host".into(), "1.65.2".into()),
        at,
        health: Part::Ready(vec![
            check("check_disk_space", true),
            check("check_vpn_leak", false),
        ]),
        cron: Part::Ready(vec![job("backup", 1), job("healthcheck", 0)]),
        containers: Part::Ready(vec![ct("gluetun", "unhealthy"), ct("jellyfin", "healthy")]),
        alerts: Part::Ready(vec![multitop::ops::Alert {
            ts: at - 60,
            subject: "[media_server] backup FAILED (rc=1)".into(),
        }]),
    };
    let mut clean = troubled.clone();
    clean.health = Part::Ready(vec![check("check_disk_space", true)]);
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

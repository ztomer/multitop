use super::*;
use crate::push::http::{Pending, Request, Response, Sender};
use crate::push::Registration;
use std::sync::Mutex;

use HostState::{Degraded, Down, Up};

#[test]
fn a_steady_state_emits_nothing_good_or_bad() {
    let mut w = Watcher::new(2);
    for _ in 0..50 {
        assert_eq!(w.observe("a", Some(Up)), None);
    }
    let mut bad = Watcher::new(2);
    let first: Vec<_> = (0..50)
        .filter_map(|_| bad.observe("b", Some(Down)))
        .collect();
    assert_eq!(
        first.len(),
        1,
        "a host down from the start is reported once: {first:?}"
    );
}

#[test]
fn a_confirmed_transition_emits_exactly_one_and_a_blip_emits_none() {
    let mut w = Watcher::new(2);
    let mut out = Vec::new();
    // Baseline up, one bad look (a reconnect), back up: nothing.
    for s in [Up, Up, Down, Up, Up, Up] {
        out.extend(w.observe("h", Some(s)));
    }
    assert!(out.is_empty(), "a one-look blip is not an alert: {out:?}");
    // Down for good: exactly one, then silence while it stays down.
    for s in [Down, Down, Down, Down, Down] {
        out.extend(w.observe("h", Some(s)));
    }
    assert_eq!(
        out,
        vec![Transition {
            host: "h".into(),
            from: Some(Up),
            to: Down
        }]
    );
    // And recovery is one more.
    for s in [Up, Up, Up] {
        out.extend(w.observe("h", Some(s)));
    }
    assert_eq!(out.len(), 2);
    assert_eq!(out[1].to, Up);
    assert_eq!(out[1].from, Some(Down));
}

#[test]
fn unread_neither_confirms_nor_resets_and_hosts_are_independent() {
    let mut w = Watcher::new(2);
    assert_eq!(w.observe("a", None), None);
    assert_eq!(w.observe("a", Some(Up)), None);
    assert_eq!(w.observe("a", None), None);
    assert_eq!(w.observe("a", Some(Up)), None, "baseline");
    assert_eq!(w.observe("b", Some(Degraded)), None);
    let t = w
        .observe("b", Some(Degraded))
        .expect("b confirmed degraded");
    assert_eq!((t.host.as_str(), t.from, t.to), ("b", None, Degraded));
    assert_eq!(
        Watcher::new(0).observe("z", Some(Down)).map(|t| t.to),
        Some(Down),
        "0 means 1"
    );
}

fn server(host: &str, name: Option<&str>) -> Server {
    Server {
        host: host.into(),
        name: name.map(str::to_string),
        ..Default::default()
    }
}

fn cfg() -> Config {
    crate::config::Config {
        servers: vec![],
        theme: None,
        upgrade_history_lines: crate::config::DEFAULT_UPGRADE_HISTORY_LINES,
        history_lines_raised_from: None,
        banner_style: crate::layout::BannerStyle::default(),
        plaintext_passwords: vec![],
        alert_cpu: Some(80),
        alert_mem: Some(85),
        alert_disk: Some(90),
        alerts: vec![],
    }
}

#[test]
fn host_state_reads_errors_then_thresholds_then_up() {
    let s = server("web", None);
    let mut live = LiveState::default();
    assert_eq!(host_state(&live, &s, &cfg()), None, "nothing read yet");
    let mut snap = multitop_agent::render::Snapshot::default();
    live.snapshots.insert("web".into(), snap.clone());
    assert_eq!(host_state(&live, &s, &cfg()), Some(Up));
    snap.cpu_pct = 99.0;
    live.snapshots.insert("web".into(), snap);
    assert_eq!(host_state(&live, &s, &cfg()), Some(Degraded));
    live.errors.insert("web".into(), "Connection closed".into());
    assert_eq!(host_state(&live, &s, &cfg()), Some(Down));
}

#[test]
fn alerts_name_the_machine_and_say_what_happened() {
    let named = server("10.0.0.5", Some("beelink"));
    let t = |to| Transition {
        host: "10.0.0.5".into(),
        from: Some(Up),
        to,
    };
    let down = alert_for(&t(Down), &named);
    assert_eq!(down.kind, Kind::Down);
    assert_eq!(down.title, "beelink is unreachable");
    assert!(down.body.contains("10.0.0.5"));
    assert_eq!(alert_for(&t(Degraded), &named).kind, Kind::Degraded);
    let up = alert_for(&t(Up), &server("db", None));
    assert_eq!(
        (up.kind, up.title.as_str()),
        (Kind::Recovered, "db recovered")
    );
}

/// Records every request; answers 201.
#[derive(Default)]
struct Recorder(Mutex<Vec<Request>>);

impl Sender for Recorder {
    fn post(&self, req: Request) -> Pending<'_> {
        self.0.lock().expect("lock").push(req);
        Box::pin(async {
            Ok(Response {
                status: 201,
                body: vec![],
            })
        })
    }
}

fn up_registration() -> Registration {
    let rng = ring::rand::SystemRandom::new();
    let key = ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
        .expect("key");
    let public = key.compute_public_key().expect("public");
    let b = |x: &[u8]| base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, x);
    Registration::UnifiedPush {
        endpoint: "https://ntfy.example/upTEST?up=1".into(),
        p256dh: b(public.as_ref()),
        auth: b(&[9u8; 16]),
    }
}

/// THE A3 proof (docs/roadmap.md): through the real loop -- live state, the
/// watcher, the hub -- a synthetic transition sends exactly one notification and
/// a steady state sends none.
#[tokio::test(start_paused = true)]
async fn the_serve_loop_sends_one_push_per_transition_and_none_for_a_steady_state() {
    let recorder = Arc::new(Recorder::default());
    let hub = Arc::new(Hub::new(recorder.clone(), None));
    hub.register(up_registration()).await.expect("registers");
    let live: SharedState = Arc::new(tokio::sync::RwLock::new(LiveState::default()));
    live.write()
        .await
        .snapshots
        .insert("web".into(), multitop_agent::render::Snapshot::default());
    spawn(live.clone(), vec![server("web", None)], cfg(), hub);

    let sent = || recorder.0.lock().expect("lock").len();
    // Steady and up for ten looks: nothing.
    tokio::time::sleep(TICK * 10).await;
    assert_eq!(sent(), 0, "a steady state sent a push");
    // The host goes away and stays away for ten looks: exactly one.
    live.write()
        .await
        .errors
        .insert("web".into(), "gone".into());
    tokio::time::sleep(TICK * 10).await;
    assert_eq!(sent(), 1, "one transition, one push");
    let first = recorder.0.lock().expect("lock")[0].clone();
    assert_eq!(first.url, "https://ntfy.example/upTEST?up=1");
    assert!(first
        .headers
        .contains(&("content-encoding".into(), "aes128gcm".into())));
}

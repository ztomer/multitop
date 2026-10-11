//! The Ops container table and endpoint discovery.

use std::path::{Path, PathBuf};

use super::*;
use crate::docker_transport::Discovery;

#[test]
fn health_reads_the_three_words_docker_puts_in_a_status() {
    assert_eq!(health_of("Up 3 hours (healthy)"), "healthy");
    assert_eq!(health_of("Up 1 second (health: starting)"), "starting");
    assert_eq!(health_of("Up 2 days (unhealthy)"), "unhealthy");
    assert_eq!(health_of("Exited (0) 5 minutes ago"), "none");
}

#[test]
fn every_container_is_listed_with_state_health_and_project() {
    let body = r#"[
        {"Names":["/web"],"State":"running","Status":"Up 3 hours (healthy)",
         "Image":"nginx","Labels":{"com.docker.compose.project":"site"}},
        {"Names":["/job"],"State":"exited","Status":"Exited (0) 1 hour ago",
         "Image":"alpine","Labels":{}}
    ]"#;
    let endpoint = DockerEndpoint::Unix("/tmp/d.sock".into());
    let v = parse(body, &endpoint).unwrap();
    assert_eq!(v["endpoint"], "unix:///tmp/d.sock");
    let c = v["containers"].as_array().unwrap();
    assert_eq!(c[0]["name"], "web");
    assert_eq!(c[0]["health"], "healthy");
    assert_eq!(c[0]["compose_project"], "site");
    assert_eq!(c[1]["state"], "exited");
    assert!(c[1]["compose_project"].is_null());
    assert!(parse("{}", &endpoint).is_err(), "an object is not a list");
}

fn colima_config(dir: &Path) {
    let meta = dir.join("contexts/meta/abc");
    std::fs::create_dir_all(&meta).unwrap();
    std::fs::write(dir.join("config.json"), r#"{"currentContext":"colima"}"#).unwrap();
    std::fs::write(
        meta.join("meta.json"),
        r#"{"Name":"colima","Endpoints":{"docker":{"Host":"unix:///h/.colima/default/docker.sock"}}}"#,
    )
    .unwrap();
}

#[test]
fn the_docker_context_is_found_the_way_the_cli_finds_it() {
    // The case that shipped wrong: Colima, selected by context, no DOCKER_HOST.
    let tmp = tempfile::tempdir().unwrap();
    colima_config(tmp.path());
    let none = |_: &Path| false;
    let d = Discovery {
        docker_host: None,
        context: None,
        config_dir: Some(tmp.path()),
        home: None,
        exists: &none,
    };
    assert_eq!(
        DockerEndpoint::discover_with(&d),
        Some(DockerEndpoint::Unix(
            "/h/.colima/default/docker.sock".into()
        ))
    );
    // DOCKER_HOST outranks the context; DOCKER_CONTEXT=default defers to the
    // sockets, and none existing is an honest None.
    let d = Discovery {
        docker_host: Some("tcp://1.2.3.4:2375"),
        ..d
    };
    assert_eq!(
        DockerEndpoint::discover_with(&d),
        Some(DockerEndpoint::Tcp("tcp://1.2.3.4:2375".into()))
    );
    let d = Discovery {
        docker_host: None,
        context: Some("default"),
        ..d
    };
    assert_eq!(DockerEndpoint::discover_with(&d), None);
}

#[test]
fn without_a_context_the_first_well_known_socket_that_exists_is_used() {
    let home = PathBuf::from("/h");
    let colima = home.join(".colima/default/docker.sock");
    let exists = |p: &Path| p == colima;
    let d = Discovery {
        docker_host: None,
        context: None,
        config_dir: None,
        home: Some(&home),
        exists: &exists,
    };
    assert_eq!(
        DockerEndpoint::discover_with(&d),
        Some(DockerEndpoint::Unix(colima.to_string_lossy().into_owned()))
    );
}

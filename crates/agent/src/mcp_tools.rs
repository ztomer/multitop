//! What `multitop-agent mcp` can answer on this machine, and the seam
//! between the answers and the machine.
//!
//! A tool is listed only when this host can answer it: `cron.status` where
//! launchd runs the user's agents, `docker.containers` where a daemon is
//! configured. The Ops view's other two sections are never listed here, and
//! [`absent`] says why, so they render as an absence with a reason -- never a
//! green "none" for a log nothing writes.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{json, Value};

use crate::docker_transport::DockerEndpoint;

/// The machine, as the tools need it. [`Live`] asks the real one; a test
/// supplies its own.
pub trait System {
    /// `launchctl list`, or `None` where there is no launchd.
    fn launchctl_list(&self) -> Option<String>;
    /// The `Label` of each of the user's launch agents.
    fn agent_labels(&self) -> Vec<String>;
    /// `ps -o pid=,etime=` for `pids`.
    fn ages(&self, pids: &[u32]) -> String;
    /// The configured Docker daemon, if any.
    fn docker(&self) -> Option<DockerEndpoint>;
    /// The containers that daemon reports.
    ///
    /// # Errors
    ///
    /// It could not be reached or answered something else.
    fn containers(&self, endpoint: &DockerEndpoint) -> Result<Value, String>;
    /// Epoch seconds now.
    fn now(&self) -> i64;
}

/// What a tool does to the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Reads; changes nothing.
    Read,
    /// Changes state that is easily restored.
    Write,
    /// Overwrites or removes state. Never listed or called: this server
    /// has no opt-in for one, so a destructive tool added to the table
    /// stays unreachable until one is built (and its test goes red first).
    Destructive,
}

/// A tool this server can list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Cron,
    Containers,
}

impl Tool {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cron => "cron.status",
            Self::Containers => "docker.containers",
        }
    }

    /// What the tool says it does: the description `tools/list` carries.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Cron => {
                "The user's launchd agents as a cron table: label, PID while running, \
                 the last exit status (negative is the signal that ended it)."
            }
            Self::Containers => {
                "Every container on the configured Docker daemon (the docker context's, \
                 so Colima is found): name, state, status, health, compose project, image."
            }
        }
    }

    /// What it does to the machine, stated beside the tool and nowhere
    /// else: the MCP layer derives the annotations from this, and a
    /// [`Effect::Destructive`] tool is never listed or called.
    #[must_use]
    pub const fn effect(self) -> Effect {
        match self {
            Self::Cron | Self::Containers => Effect::Read,
        }
    }

    /// The shape of every successful answer (`outputSchema`); `required`
    /// names only the fields every success carries.
    #[must_use]
    pub fn output_schema(self) -> Value {
        match self {
            Self::Cron => json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "always \"launchd\"" },
                    "jobs": { "type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string" },
                            "pid": { "type": ["integer", "null"] },
                            "last_rc": { "type": ["integer", "null"] },
                            "running_since": { "type": ["integer", "null"] },
                            "last_start": { "type": ["integer", "null"] },
                            "alert_since": { "type": ["integer", "null"] },
                        },
                        "required": ["label"],
                    } },
                },
                "required": ["source", "jobs"],
            }),
            Self::Containers => json!({
                "type": "object",
                "properties": {
                    "endpoint": { "type": "string" },
                    "containers": { "type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "state": { "type": "string" },
                            "status": { "type": "string" },
                            "health": { "type": ["string", "null"] },
                            "compose_project": { "type": ["string", "null"] },
                            "image": { "type": "string" },
                        },
                        "required": ["name", "state"],
                    } },
                },
                "required": ["endpoint", "containers"],
            }),
        }
    }

    /// Run it.
    ///
    /// # Errors
    ///
    /// The machine could not answer.
    pub fn run(self, system: &dyn System) -> Result<Value, String> {
        match self {
            Self::Cron => {
                let list = system.launchctl_list().ok_or("launchctl did not answer")?;
                let rows = crate::launchd::parse_list(&list);
                let labels = system.agent_labels();
                let pids: Vec<u32> = rows
                    .iter()
                    .filter(|r| labels.contains(&r.label))
                    .filter_map(|r| r.pid)
                    .collect();
                let ages = crate::launchd::parse_etimes(&system.ages(&pids));
                Ok(crate::launchd::jobs(&labels, &rows, &ages, system.now()))
            }
            Self::Containers => {
                let endpoint = system.docker().ok_or("no Docker daemon is configured")?;
                system.containers(&endpoint)
            }
        }
    }
}

/// Every tool in the table, listed or not.
pub const ALL: [Tool; 2] = [Tool::Cron, Tool::Containers];

/// What this host can answer, decided once per request from the machine.
/// A destructive tool is never among them.
#[must_use]
pub fn listed(system: &dyn System) -> Vec<Tool> {
    let mut tools = Vec::new();
    if system.launchctl_list().is_some() {
        tools.push(Tool::Cron);
    }
    if system.docker().is_some() {
        tools.push(Tool::Containers);
    }
    tools.retain(|t| t.effect() != Effect::Destructive);
    tools
}

/// Each section the Ops view reads that this host does not list, and why.
#[must_use]
pub fn absent(system: &dyn System) -> Value {
    let mut why = json!({
        "health.run": "no health suite on this machine",
        // The Ops view reads the health VERDICT (`health://latest`), and
        // the resource is listed with `health.list` - so that is the tool
        // whose absence says why this host has no health section. This
        // agent answers no health at all, so it names all three.
        "health.list": "no health suite on this machine",
        "health.run_all": "no health suite on this machine",
        "alerts.since": "nothing on this machine logs alerts",
        // The ZFS section reads `zfs.health`, the tool that owns a pool's
        // state and its vdev tree. This agent has no such tool -- it reads
        // no pools -- and says so rather than letting the Ops view go
        // quiet about the one fact a SUSPENDED pool hides.
        "zfs.health": "this agent has no ZFS tool: it reads no pools",
        // The machine model's tools (servers ROADMAP O92): the tunnel, the
        // machine's filesystems, and the machine's own facts. This agent
        // streams the cpu, the memory and the disks it watches; it does not
        // answer them as facts, and it measures no tunnel's exit.
        "net.egress": "this agent has no net.egress tool: it measures no tunnel's exit",
        "disk.usage": "this agent has no disk.usage tool: it streams the disks it watches",
        "host.facts": "this agent has no host.facts tool: it streams the machine's cpu and memory",
    });
    if system.launchctl_list().is_none() {
        why["cron.status"] = json!("no launchd here; this agent reads cron only from launchd");
    }
    if system.docker().is_none() {
        why["docker.containers"] =
            json!("no Docker: no DOCKER_HOST, no docker context, no daemon socket");
    }
    why
}

/// The real machine.
pub struct Live {
    home: Option<PathBuf>,
}

impl Live {
    #[must_use]
    pub fn new() -> Self {
        Self {
            home: std::env::var_os("HOME").map(PathBuf::from),
        }
    }
}

impl Default for Live {
    fn default() -> Self {
        Self::new()
    }
}

fn output(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

impl System for Live {
    fn launchctl_list(&self) -> Option<String> {
        output(Command::new("launchctl").arg("list"))
    }

    fn agent_labels(&self) -> Vec<String> {
        let Some(dir) = self.home.as_ref().map(|h| h.join("Library/LaunchAgents")) else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut plists: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "plist"))
            .collect();
        plists.sort();
        // A plist with no Label (an updater's empty stub) is not a job.
        plists
            .iter()
            .filter_map(|p| {
                output(
                    Command::new("plutil")
                        .args(["-extract", "Label", "raw", "-o", "-"])
                        .arg(p),
                )
            })
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    fn ages(&self, pids: &[u32]) -> String {
        if pids.is_empty() {
            return String::new();
        }
        let list = pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        // `ps` exits 1 when one PID has gone; what it printed still counts.
        Command::new("ps")
            .args(["-o", "pid=,etime=", "-p", &list])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }

    fn docker(&self) -> Option<DockerEndpoint> {
        DockerEndpoint::discover()
    }

    fn containers(&self, endpoint: &DockerEndpoint) -> Result<Value, String> {
        crate::docker_ops::list(endpoint)
    }

    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
    }
}

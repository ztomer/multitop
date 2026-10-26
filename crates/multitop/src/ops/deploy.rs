//! The host's checkout, and whether the build that answered is still the
//! host's (queue MON.STALE).
//!
//! **THE BUILD THAT ANSWERS CAN OUTLIVE ITS DEPLOY.** The Ops poll keeps one
//! session per host open across polls, so the `mcp_host` answering is the
//! process that session started. A deploy replaces the binary on disk and
//! leaves that process running: every later answer comes from the build
//! BEFORE the deploy, and the panel's `mcp-host 1.103.0` is true of the
//! process and false of the host (2026-10-26: deployed to 1.103.1, and the
//! frame said nothing).
//!
//! The fact that tells them apart is one the host already reports:
//! `host.facts` reads the checkout's `VERSION` and `head` on every call, so
//! the answer that carries the old build's `serverInfo` version carries the
//! deploy's version beside it. No clock is consulted - a reading is not
//! stale because it is old (its age is printed), it is stale because the
//! build that made it is not the one the host was deployed to.

use serde_json::Value;

/// The primary instance's checkout, as `host.facts` publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    /// The instance the checkout belongs to, when the server says.
    pub instance: Option<String>,
    /// The checkout's commit.
    pub head: String,
    /// The checkout's `VERSION`: what the host was deployed to.
    pub version: String,
}

impl Repo {
    /// The commit as the frame prints it: the first eight characters.
    #[must_use]
    pub fn short_head(&self) -> &str {
        self.head.get(..8).unwrap_or(&self.head)
    }
}

/// The `repo` block of a `host.facts` answer; `None` when the server does
/// not publish one, or publishes no version.
#[must_use]
pub fn parse_repo(facts: &Value) -> Option<Repo> {
    let r = facts.get("repo").filter(|r| r.is_object())?;
    let text = |k: &str| r.get(k).and_then(Value::as_str).map(str::to_string);
    Some(Repo {
        instance: text("instance").filter(|s| !s.is_empty()),
        head: text("head").unwrap_or_default(),
        version: text("version").filter(|s| !s.is_empty())?,
    })
}

/// The deploy the build that answered predates: the checkout, when its
/// version is not the version the answering `mcp_host` reported.
#[must_use]
pub fn deploy_after<'a>(running: &str, repo: Option<&'a Repo>) -> Option<&'a Repo> {
    repo.filter(|r| r.version != running.trim())
}

/// The stale line's words: the build that answered, and the deploy it
/// predates, named by version and commit.
#[must_use]
pub fn stale_words(server: &(String, String), repo: &Repo) -> String {
    let head = if repo.head.is_empty() {
        String::new()
    } else {
        format!(" ({})", repo.short_head())
    };
    format!(
        "answered by {} {}, a build from before the host's deploy of {}{head}",
        server.0, server.1, repo.version
    )
}

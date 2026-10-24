//! `machine.triage`: the machine's ONE verdict (servers ROADMAP O92).
//!
//! What needs the owner's hand across the machine's instances, each item naming
//! its instance, with the sources' own words as witnesses and the owner's
//! own fix quoted verbatim, and a quiet line per instance with nothing to
//! say, so that silence and "all clear" never look alike.
//!
//! This client draws it and composes nothing: the item's words, its
//! witnesses and its fix are the server's, and a fix the server does not
//! give is said to be absent - never filled in.

use serde_json::Value;

/// The tool that answers the machine's verdict.
pub const TRIAGE_TOOL: &str = "machine.triage";

/// One thing that needs the owner's hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriageItem {
    /// The instance (stack) it belongs to: required on every item.
    pub instance: String,
    /// The fault's kind, the server's word (`zfs_pool`, `check`, ...).
    pub kind: String,
    /// What is wrong, in the server's words.
    pub what: String,
    /// Each source's own words for it.
    pub witnesses: Vec<String>,
    /// The owner's own fix, verbatim; `None` when the owner states none.
    pub fix: Option<String>,
}

/// The machine's verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Triage {
    pub needs_attention: bool,
    pub items: Vec<TriageItem>,
    /// One line per instance with nothing to say: what was read.
    pub quiet: Vec<String>,
}

/// `machine.triage`'s answer, read strictly: an item without its instance,
/// or a verdict without its fields, is a failure named by the field.
///
/// # Errors
///
/// The name of the first field that is missing or of the wrong type.
pub fn parse_triage(v: &Value) -> Result<Triage, String> {
    let needs_attention = v
        .get("needs_attention")
        .and_then(Value::as_bool)
        .ok_or("machine.triage: no needs_attention")?;
    let items = v
        .get("items")
        .and_then(Value::as_array)
        .ok_or("machine.triage: no items")?
        .iter()
        .map(item)
        .collect::<Result<Vec<_>, _>>()?;
    let quiet = strings(v.get("quiet"), "quiet")?;
    Ok(Triage {
        needs_attention,
        items,
        quiet,
    })
}

fn item(v: &Value) -> Result<TriageItem, String> {
    let text = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("machine.triage: an item has no {key}"))
    };
    let fix = match v.get("fix") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("machine.triage: an item's fix is not text".into()),
    };
    Ok(TriageItem {
        instance: text("instance")?,
        kind: text("kind")?,
        what: text("what")?,
        witnesses: strings(v.get("witnesses"), "witnesses")?,
        fix,
    })
}

fn strings(v: Option<&Value>, key: &str) -> Result<Vec<String>, String> {
    v.and_then(Value::as_array)
        .ok_or_else(|| format!("machine.triage: no {key}"))?
        .iter()
        .map(|s| {
            s.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("machine.triage: {key} holds something that is not text"))
        })
        .collect()
}

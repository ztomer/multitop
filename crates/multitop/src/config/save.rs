use crate::config::Server;
use std::path::Path;

/// Remove every `sudo_password` key from the config, returning how many went.
///
/// # Errors
/// Returns the write failure as a string if the rewritten file cannot be saved.
pub fn strip_plaintext_passwords(path: &Path) -> Result<usize, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| error.to_string())?;

    let mut removed = 0;
    if let Some(servers) = doc
        .get_mut("servers")
        .and_then(toml_edit::Item::as_array_of_tables_mut)
    {
        for table in servers.iter_mut() {
            if table.remove("sudo_password").is_some() {
                removed += 1;
            }
        }
    }
    if removed > 0 {
        crate::state::write_atomic(path, &doc.to_string())?;
    }
    Ok(removed)
}

pub fn save_theme(path: &Path, theme_name: &str) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    // `toml_edit`, not `toml`, for the reason `save_banner_style` below spells
    // out -- and this function is where that reason came from without being
    // applied. Parsing to `toml::Table` and re-serialising rebuilds the file
    // from its values; comments and blank lines are not values, so a single
    // press of `t` reduced a hand-maintained config to a bare key list.
    //
    // The fix landed on the toggle immediately below this one and not on this
    // one, which is the shape this round has found more than any other: an
    // instance cured while its sibling, doing the identical job on the identical
    // file, kept the defect.
    let Ok(mut doc) = content.parse::<toml_edit::DocumentMut>() else {
        return;
    };
    doc["theme"] = toml_edit::value(theme_name);
    let _ = crate::state::write_atomic(path, &doc.to_string());
}

pub fn save_banner_style(path: &Path, style: crate::layout::BannerStyle) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut doc) = content.parse::<toml_edit::DocumentMut>() else {
        return;
    };
    doc["banner_style"] = toml_edit::value(style.as_str());
    let _ = crate::state::write_atomic(path, &doc.to_string());
}

/// Write the server list, preserving everything else the file holds.
///
/// # Errors
/// Returns a string if the config directory cannot be created or the file
/// cannot be written.
pub fn save_servers(path: &Path, servers: &[Server]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc = if content.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        content
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| error.to_string())?
    };

    // Reuse the existing table for a server that is still present, so the
    // comment above it survives. Rebuilding every entry from scratch -- which is
    // what serialising a parsed value does -- threw away the whole file's
    // comments and blank lines every time a server was added or edited.
    let existing: Vec<toml_edit::Table> = doc
        .get("servers")
        .and_then(toml_edit::Item::as_array_of_tables)
        .map(|a| a.iter().cloned().collect())
        .unwrap_or_default();

    // Matched on the full identity -- host, port and user -- and each existing
    // table is handed out at most once.
    //
    // The match was on `host` alone. Two entries on one machine with different
    // users or ports are different things, and this project is explicit about
    // that everywhere else: `replace_panels` carries credentials across an edit
    // keyed on the full identity precisely because handing the first entry's
    // password to the rest would send one account's sudo password to another's
    // session. The writer kept the host-only match, so on every save both
    // entries cloned the *first* matching table -- one silently acquired the
    // other's hand-written keys and the other's were destroyed.
    //
    // `claimed` is what makes it at-most-once; full-identity matching alone
    // would still hand one table to two genuinely identical entries.
    let mut claimed = vec![false; existing.len()];
    let mut out = toml_edit::ArrayOfTables::new();
    for server in servers {
        let matches_identity = |t: &toml_edit::Table| {
            t.get("host").and_then(|v| v.as_str()) == Some(server.host.as_str())
                && t.get("port").and_then(toml_edit::Item::as_integer)
                    == Some(i64::from(server.port))
                && t.get("user").and_then(|v| v.as_str()).unwrap_or_default() == server.user
        };
        let found = (0..existing.len()).find(|&i| !claimed[i] && matches_identity(&existing[i]));
        // A row whose identity the user has just edited matches nothing. Give it
        // the first table nobody has claimed, in order, so the comment above it
        // survives the edit. A row that was *added* leaves none unclaimed --
        // every existing entry matched itself -- so it correctly gets a fresh
        // table rather than inheriting a stranger's keys.
        let found = found.or_else(|| (0..existing.len()).find(|&i| !claimed[i]));
        let mut table = found.map_or_else(toml_edit::Table::new, |i| {
            claimed[i] = true;
            existing[i].clone()
        });
        table["host"] = toml_edit::value(server.host.clone());
        table["port"] = toml_edit::value(i64::from(server.port));
        table["user"] = toml_edit::value(server.user.clone());
        match &server.upgrade_cmd {
            Some(command) => table["upgrade_cmd"] = toml_edit::value(command.clone()),
            None => {
                table.remove("upgrade_cmd");
            }
        }
        out.push(table);
    }
    doc["servers"] = toml_edit::Item::ArrayOfTables(out);
    crate::state::write_atomic(path, &doc.to_string())
}

//! The triage row, at the top of the card (servers ROADMAP O92): the
//! machine's one verdict, drawn in the server's words.
//!
//! One row per item, each naming its stack, so the reader sees WHICH stack
//! is unhappy rather than counting ticks; under it the owner's fix verbatim,
//! or the plain fact that the owner states none, and the witnesses. Then
//! the quiet line of each stack with nothing to say: what was read there,
//! so an instance nobody read never passes for one that is fine.
//! Each "; " clause of a quiet line gets its own row, never clipped.

use super::super::triage::Triage;
use super::{Frame, Tone};

const LABEL: &str = "triage";

pub(super) fn triage(f: &mut Frame<'_>, t: &Triage) {
    if t.items.is_empty() {
        if t.needs_attention {
            f.line(
                Tone::Warn,
                LABEL,
                "the server says something needs your hand, and names nothing",
            );
        } else {
            f.line(Tone::Good, LABEL, "nothing needs your hand");
        }
    }
    for item in &t.items {
        f.line(
            Tone::Bad,
            LABEL,
            &format!("{}: {}", item.instance, item.what),
        );
        match &item.fix {
            Some(fix) => f.item(&format!("fix: {fix}")),
            None => f.item("fix: none stated by the owner"),
        }
        for w in &item.witnesses {
            f.item(w);
        }
    }
    // Since media_server 1.99.25 a quiet line carries clauses joined by
    // "; " ("N suite checks ok (...); M other scheduled check(s) ok (...)"),
    // longer than a card row: each clause after the first goes on its own
    // continuation row, verbatim, so no clause is clipped away.
    for q in &t.quiet {
        let mut clauses = q.split("; ");
        f.line(Tone::Quiet, LABEL, clauses.next().unwrap_or_default());
        for c in clauses {
            f.item(c);
        }
    }
}

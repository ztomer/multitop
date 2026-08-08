use crate::app::App;
use crate::refit::refit_line;

fn pane_window(
    total: usize,
    height: usize,
    pinned: usize,
    scroll_offset: usize,
) -> (usize, usize, usize, usize) {
    let pinned = pinned
        .min(total)
        .min((height / 2).max(1))
        .min(height.saturating_sub(1));
    let body_budget = height.saturating_sub(pinned.max(1));
    if pinned > 0 && total > pinned {
        let body_len = total - pinned;
        let max_offset = body_len.saturating_sub(body_budget);
        let eff = scroll_offset.min(max_offset);
        let end = body_len.saturating_sub(eff);
        let start = end.saturating_sub(body_budget);
        (pinned, pinned + start, pinned + end, eff)
    } else {
        let max_offset = total.saturating_sub(height);
        let eff = scroll_offset.min(max_offset);
        let end = total.saturating_sub(eff);
        let start = end.saturating_sub(height);
        // `eff`, not 0. This branch scrolls the window by `eff` and used to
        // report no scroll at all -- and `draw` writes the reported value back
        // as the pane's offset, so the frame that scrolled also reset itself.
        // Reachable only with no pinned block, which today means a one-row pane,
        // where the one row is the banner and nothing was visibly wrong. It is
        // still one quantity computed and a different one returned, in the
        // function this round has now changed twice.
        (0, start, end, eff)
    }
}

/// Show the tail when there is more content than room, optionally pinning
/// the header (line 0) so the server name stays visible. Supports scrolling via `scroll_offset`.
///
/// Returns the rows to draw and **how far back the view is scrolled**, for the
/// caller to render as a badge.
///
/// # Row 0 has one owner
///
/// This used to compose the scroll badge onto `out[0]` itself. `draw` then
/// overwrote `lines[0]` with the host banner, unconditionally, on every frame --
/// so the badge was built and destroyed within one frame and the scroll-position
/// indicator has never once been on screen. Two pieces of code believed they
/// owned that row and the later one silently won.
///
/// Handing the offset back as a value, rather than as text already baked into a
/// row somebody else is about to overwrite, is what makes that unrepresentable.
#[must_use]
pub fn visible(
    lines: &[String],
    height: usize,
    pinned: usize,
    target_cols: usize,
    scroll_offset: usize,
) -> (Vec<String>, usize) {
    if height == 0 || lines.is_empty() {
        return (Vec::new(), 0);
    }
    let total = lines.len();
    let (pinned, start, end, badge) = pane_window(total, height, pinned, scroll_offset);
    let mut out = Vec::with_capacity(height.min(total));
    if pinned > 0 {
        out.extend_from_slice(&lines[..pinned]);
    }
    out.extend_from_slice(&lines[start..end]);
    if !out.is_empty() && target_cols > 0 {
        for line in &mut out {
            *line = refit_line(line, target_cols);
        }
    }
    (out, badge)
}

/// Window a pinned header over a `RingLines` body and a trailing block, like
/// `visible` for the Upgrade pane.
///
/// The pane is composed at draw time from the ring rather than mirrored into
/// `view`, so this is the one path that must not materialise the whole log to
/// show it: the window takes at most `height` lines from any source, and the
/// ring's slots are borrowed, not cloned. The windowing rule itself is
/// `pane_window`, shared with `visible`.
///
/// `tail` is the pane's notices, already wrapped. They go here rather than being
/// left to `Panel::note`'s mode check because that check runs when the notice is
/// *written* and this one runs when the pane is *drawn*, and the two modes need
/// not agree: every startup notice is written in Monitor mode, so pressing `u`
/// made them all disappear -- including the one that says the Upgrade pane's
/// scrollback was clamped, which is about the very pane that was hiding it.
///
/// # `pinned` is a parameter, not `header.len()`
///
/// It was `header.len()`, and that was correct exactly as long as `header` held
/// nothing but pinned content. The twenty-ninth pass then appended the *held*
/// notices to it -- lines whose whole purpose is to be above the content and
/// reached by scrolling -- and the clamp pinned them: the pane drew "1 earlier
/// notice above" with that notice four rows higher, on screen. A label stating
/// something the frame contradicts.
///
/// Measuring a slice is a derivation like any other, and it is only right while
/// nothing else can get into the slice. The count comes from the one place that
/// knows what it built.
#[must_use]
pub fn visible_upgrade(
    header: &[String],
    pinned: usize,
    body: &crate::panel::RingLines,
    tail: &[String],
    height: usize,
    target_cols: usize,
    scroll_offset: usize,
) -> (Vec<String>, usize) {
    if height == 0 {
        return (Vec::new(), 0);
    }
    let total = header.len() + body.len() + tail.len();
    if total == 0 {
        return (Vec::new(), 0);
    }
    let (pinned, start, end, badge) = pane_window(total, height, pinned, scroll_offset);
    // `height` is the caller's; a caller asking for a height larger than the
    // content must not make this reserve it.
    let mut out = Vec::with_capacity(height.min(total));
    if pinned > 0 {
        out.extend_from_slice(&header[..pinned]);
    }
    // The window may straddle any of the three boundaries (a header taller than
    // the pinned clamp scrolls into the body; the body's tail runs into the
    // notices). Take whatever part of it falls in each source.
    let header_end = header.len();
    let body_end = header_end + body.len();
    let h_hi = end.min(header_end);
    if start < h_hi {
        out.extend_from_slice(&header[start..h_hi]);
    }
    let b_lo = start.max(header_end).min(body_end);
    let b_hi = end.min(body_end);
    if b_hi > b_lo {
        out.extend(body.slice(b_lo - header_end, b_hi - b_lo).cloned());
    }
    let t_lo = start.max(body_end);
    if end > t_lo {
        out.extend_from_slice(&tail[t_lo - body_end..end - body_end]);
    }
    if !out.is_empty() && target_cols > 0 {
        for line in &mut out {
            *line = refit_line(line, target_cols);
        }
    }
    (out, badge)
}

/// The pane's notices, split into what it draws at rest and what it pushes
/// above the content.
///
/// # Why a pane's notices need a bound at all
///
/// `MAX_NOTES` bounds how many notices a panel *keeps*, and its comment says the
/// bound exists so a repeated one "cannot crowd out the pane it is drawn in".
/// It could not do that. A pane's cost is in **wrapped lines**, and at forty
/// columns one notice is four of them -- so four notices are sixteen rows, and
/// in an eleven-row pane they were the entire pane. Rendered at 40x12 over a
/// live monitor frame, not one line of the machine was on screen: no cpu, no
/// memory, no load. A bound stated in one quantity to govern a different one,
/// and converting between them needs the width, which `Panel` does not have.
///
/// # Why it is a split and not a truncation
///
/// Held-back notices go *above* the pane's content rather than being dropped,
/// so `Home` still reaches every one of them -- that was the twenty-seventh
/// pass's finding and it does not get to be undone by this one. What the two
/// rules together give is: the newest notices and the content are both on screen
/// at rest, a line says how many are above, and scrolling back finds them.
///
/// The share is half the pane, which is what `pane_window` already allows the
/// pinned block, and for the same reason: the thing being announced must not
/// displace the thing it is about. Whole notices only -- a notice cut in half is
/// the defect the wrapping exists to prevent.
pub fn notice_split(
    notes: &[String],
    target_cols: usize,
    height: usize,
) -> (Vec<String>, Vec<String>) {
    if notes.is_empty() || target_cols == 0 {
        return (Vec::new(), Vec::new());
    }
    let wrapped: Vec<Vec<String>> = notes
        .iter()
        .map(|note| crate::layout::wrap_words(note, target_cols))
        .collect();

    // Newest first, whole blocks only, until the next one would not fit.
    let fit = |budget: usize| -> usize {
        let mut used = 0;
        let mut kept = 0;
        for block in wrapped.iter().rev() {
            if used + block.len() > budget {
                break;
            }
            used += block.len();
            kept += 1;
        }
        kept
    };

    let budget = (height / 2).max(1);
    if fit(budget) == wrapped.len() {
        return (Vec::new(), wrapped.into_iter().flatten().collect());
    }
    // Something is going above, so a row of the budget goes to saying so.
    let kept = fit(budget.saturating_sub(1));
    let split = wrapped.len() - kept;
    let hidden = split;
    let mut shown = crate::layout::wrap_words(
        &format!(
            "\u{2026} {hidden} earlier notice{} above",
            if hidden == 1 { "" } else { "s" }
        ),
        target_cols,
    );
    let mut iter = wrapped.into_iter();
    let held: Vec<String> = iter.by_ref().take(split).flatten().collect();
    shown.extend(iter.flatten());
    (held, shown)
}

/// Rows an ordinary pane pins: row 0, which the host banner owns.
///
/// This was `Panel::pinned_lines`, a field stamped on every view switch. It was
/// 1 at every site that read it -- the only writes that set anything else were
/// made on the way *into* the Upgrade view, which is the one pane that does not
/// read the field, because `visible_upgrade` is given the header it just built
/// and measures that. A stored copy of a number the renderer recomputes anyway,
/// with a doc comment claiming it was what pinned the header. Deleted, and the
/// constant says what the rule actually is.
const ORDINARY_PINNED: usize = 1;

/// The lines one panel's pane shows, windowed to `height` and fitted to
/// `target_cols`, plus the scroll-badge offset.
///
/// **The single entry point to "what is in that pane".** There are two pane
/// sources -- `view` for the ordinary modes, and the status header composed
/// over the `last_upgrade` ring for the Upgrade pane -- and which one applies
/// is decided here and nowhere else. `draw` calls it, and so does every test
/// that wants the text a user would see; a test reading `panel.view` directly
/// is reading a buffer the Upgrade pane does not draw.
#[must_use]
pub fn pane_lines(
    app: &App,
    panel: usize,
    height: usize,
    target_cols: usize,
    scroll_offset: usize,
) -> (Vec<String>, usize) {
    let Some(p) = app.panels.get(panel) else {
        return (Vec::new(), 0);
    };
    // Composed here because here is the only place that knows the pane's width
    // *and* its height -- see `notice_split` for why both are needed. One place
    // for both panes; it was the ordinary pane's business alone, and the Upgrade
    // pane consequently drew no notices at all.
    //
    // `held` goes above the pane's content, where scrolling back reaches it;
    // `shown` goes below, where it is on screen at rest.
    let (held, shown) = notice_split(&p.notes, target_cols, height);
    if p.mode == crate::app::Mode::Upgrade {
        let mut header = app.upgrade_pane_header(panel);
        // Only the status block is pinned. The held notices go after it so they
        // sit above the log, in the scrollable part -- which is the whole claim
        // the "N earlier notices above" line makes.
        let pinned = header.len();
        header.extend(held);
        visible_upgrade(
            &header,
            pinned,
            &p.last_upgrade,
            &shown,
            height,
            target_cols,
            scroll_offset,
        )
    } else if held.is_empty() && shown.is_empty() {
        visible(&p.view, height, ORDINARY_PINNED, target_cols, scroll_offset)
    } else {
        // The clone is paid only when a notice exists, which is rare and never
        // on the streaming path.
        let mut body = Vec::with_capacity(p.view.len() + held.len() + shown.len());
        // Row 0 is the banner's and must stay row 0, so the held block goes
        // *after* it rather than at the front.
        let mut view = p.view.iter().cloned();
        body.extend(view.next());
        body.extend(held);
        body.extend(view);
        body.extend(shown);
        visible(&body, height, ORDINARY_PINNED, target_cols, scroll_offset)
    }
}

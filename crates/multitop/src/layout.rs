//! Fitting rows of labels into a terminal that may be narrower than they are.
//!
//! # The rule
//!
//! **A label is whole or it is absent. There is no third state.** (Kare, review
//! round B, binding on UX/UI.)
//!
//! Everything here exists because four separate rows were laid out to constants
//! and handed to a `Paragraph`, which clips. At 80 columns that lost the sort
//! badge and cut `[Theme: Kare` mid-word; at 40 it cut the keybar to `Upgrad`,
//! left an orphaned `[` in the settings hints, and amputated a destructive
//! dialog's own cancel instruction to `Esc t`. Each was found separately and
//! each looked like its own bug. They are one bug: no row had a budget.
//!
//! # Why a shed *order* rather than "drop from the right"
//!
//! The rightmost chunk is not the least useful one. On every row here the thing
//! the user cannot guess -- the way out -- is last, and dropping from the right
//! drops exactly that. So each caller declares what it is willing to lose, in
//! order, and the way out is never in the list.

/// Which chunks of a single-line row fit, given a width budget.
///
/// `widths` are the display widths of each chunk in **display order**.
/// `sep` is the width of the separator drawn *between* two kept chunks.
/// `shed` lists chunk indices in the order they may be given up, first to go
/// first; anything not named is never dropped.
///
/// Returns the kept indices, in display order. Chunks are kept or dropped
/// whole: a returned index means that chunk is drawn in full.
///
/// When even the un-sheddable chunks do not fit, they are all returned anyway
/// -- the caller is being asked to draw into a terminal too narrow for its own
/// essentials, and silently dropping the last of them would leave a row that
/// says nothing at all.
#[must_use]
pub fn fit_row(widths: &[usize], sep: usize, budget: usize, shed: &[usize]) -> Vec<usize> {
    let mut kept: Vec<bool> = vec![true; widths.len()];
    let used = |kept: &[bool]| -> usize {
        let count = kept.iter().filter(|k| **k).count();
        let text: usize = widths
            .iter()
            .zip(kept)
            .filter(|(_, k)| **k)
            .map(|(w, _)| *w)
            .sum();
        text + sep * count.saturating_sub(1)
    };

    for &index in shed {
        if used(&kept) <= budget {
            break;
        }
        if let Some(slot) = kept.get_mut(index) {
            *slot = false;
        }
    }

    (0..widths.len()).filter(|i| kept[*i]).collect()
}

/// Share a row's width between cells that must not shrink and cells that may.
///
/// `fixed` are widths that are never reduced -- a state marker, a port, a
/// two-state badge whose two states are the whole point of the column.
/// `flex_min` are the smallest useful widths of the cells that may shrink, in
/// display order.
///
/// Returns a width for each flexible cell. The remainder after the fixed cells
/// and separators is split evenly, then any cell that wanted less than its
/// share hands the surplus back to the others -- so a short `user` column does
/// not hoard space a long `upgrade command` could use.
///
/// Below the point where even `flex_min` fits, every flexible cell gets its
/// minimum and the row is allowed to be wider than the terminal; the caller
/// clips. That is deliberate: the alternative is a table whose columns stop
/// lining up with their own header, which is harder to read than a clipped one.
#[must_use]
pub fn share_width(
    fixed: usize,
    flex_min: &[usize],
    flex_want: &[usize],
    budget: usize,
) -> Vec<usize> {
    let n = flex_min.len();
    if n == 0 {
        return Vec::new();
    }
    let floor: usize = flex_min.iter().sum();
    let Some(mut pool) = budget.checked_sub(fixed + floor) else {
        return flex_min.to_vec();
    };

    let mut out = flex_min.to_vec();
    // Hand out the surplus in rounds, so a cell that wants only a little takes
    // only a little and the rest stays available.
    let mut hungry: Vec<usize> = (0..n).filter(|i| flex_want[*i] > flex_min[*i]).collect();
    while pool > 0 && !hungry.is_empty() {
        let share = (pool / hungry.len()).max(1);
        let mut still_hungry = Vec::new();
        for &i in &hungry {
            if pool == 0 {
                still_hungry.push(i);
                continue;
            }
            let need = flex_want[i] - out[i];
            let give = share.min(need).min(pool);
            out[i] += give;
            pool -= give;
            if out[i] < flex_want[i] {
                still_hungry.push(i);
            }
        }
        if still_hungry.len() == hungry.len() && share == 1 && pool == 0 {
            break;
        }
        hungry = still_hungry;
    }
    out
}

/// The host label for a panel banner, fitted to the width it has.
///
/// # What gets sacrificed, in order
///
/// The `user@` prefix goes first. It is identical in every panel of a normal
/// configuration, so it distinguishes nothing and is the cheapest thing to
/// lose. Only then is the host name itself cut -- and it is cut **from the
/// left**, because the tail is what tells `web-01` from `web-02`.
///
/// That direction is the whole point. The banner used to be mapped into
/// fullwidth codepoints (U+FF01-FF5E), which doubled the cell cost and then
/// clipped on the right: at four panels on a small terminal `ztomer@web-01`
/// needed 26 cells in a 20-cell pane and rendered as `ｚｔｏｍｅｒ＠ｗｅ`.
/// The digits -- the only part that differs between hosts -- were exactly what
/// fell off, on a tool where the selected panel is the machine `u` runs
/// `apt upgrade` against.
///
/// (Those codepoints are also absent from every mono terminal face in common
/// use -- Menlo, SF Mono, `JetBrains` Mono, Berkeley Mono -- so the banner was
/// drawn in a fallback CJK font, a different typeface and baseline from every
/// line beneath it.)
#[must_use]
pub fn fit_banner(user: &str, host: &str, budget: usize) -> String {
    if budget == 0 {
        return String::new();
    }
    let named = !user.is_empty() && !user.eq_ignore_ascii_case("default");
    if named {
        let full = format!("{user}@{host}");
        if full.chars().count() <= budget {
            return full;
        }
    }
    if host.chars().count() <= budget {
        return host.to_string();
    }
    // Keep the end: `…b-01` says more than `web-0…`.
    let tail: String = host
        .chars()
        .skip(host.chars().count() - budget.saturating_sub(1))
        .collect();
    format!("\u{2026}{tail}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_banner_keeps_the_user_when_there_is_room() {
        assert_eq!(fit_banner("ztomer", "web-01", 40), "ztomer@web-01");
    }

    /// The prefix is identical in every panel, so it is the cheapest loss.
    #[test]
    fn the_user_prefix_is_dropped_before_the_host_is_cut() {
        assert_eq!(fit_banner("ztomer", "web-01", 10), "web-01");
    }

    /// The defect this exists for: two hosts must not render identically.
    #[test]
    fn a_cut_host_keeps_the_end_that_distinguishes_it() {
        let a = fit_banner("ztomer", "webserver-01", 6);
        let b = fit_banner("ztomer", "webserver-02", 6);
        assert_ne!(a, b, "the digits are what tell the machines apart");
        assert!(a.ends_with("01"), "got {a}");
        assert!(b.ends_with("02"), "got {b}");
        assert!(a.chars().count() <= 6, "got {a}");
    }

    #[test]
    fn a_default_user_is_not_a_name() {
        assert_eq!(fit_banner("", "web-01", 40), "web-01");
        assert_eq!(fit_banner("default", "web-01", 40), "web-01");
    }

    #[test]
    fn a_budget_of_nothing_is_not_a_panic() {
        assert_eq!(fit_banner("ztomer", "web-01", 0), "");
        assert_eq!(fit_banner("ztomer", "web-01", 1).chars().count(), 1);
    }

    /// The rule, stated as a test: nothing is ever half-drawn.
    #[test]
    fn a_chunk_is_kept_whole_or_dropped_whole() {
        // Three chunks of 10, separated by 2, want 34 columns.
        let widths = [10, 10, 10];
        assert_eq!(fit_row(&widths, 2, 34, &[0, 1]), vec![0, 1, 2]);
        // One column short: the first sheddable goes, and the rest are intact.
        assert_eq!(fit_row(&widths, 2, 33, &[0, 1]), vec![1, 2]);
    }

    #[test]
    fn sheds_in_the_declared_order_not_right_to_left() {
        let widths = [10, 10, 10];
        // Index 2 is the way out; it is not in the shed list, so it survives
        // even though it is last.
        assert_eq!(fit_row(&widths, 2, 10, &[0, 1]), vec![2]);
    }

    #[test]
    fn stops_shedding_as_soon_as_it_fits() {
        let widths = [5, 5, 5];
        // 19 needed, 15 available: dropping one (12) is enough, so the second
        // in the shed order stays.
        assert_eq!(fit_row(&widths, 2, 15, &[0, 1]), vec![1, 2]);
    }

    /// A terminal too narrow for the essentials still gets the essentials, not
    /// an empty row.
    #[test]
    fn the_unsheddable_survive_a_budget_that_cannot_hold_them() {
        let widths = [10, 10];
        assert_eq!(fit_row(&widths, 2, 3, &[]), vec![0, 1]);
    }

    #[test]
    fn everything_fits_when_there_is_room() {
        let widths = [4, 4];
        assert_eq!(fit_row(&widths, 1, 100, &[0, 1]), vec![0, 1]);
    }

    #[test]
    fn a_short_cell_hands_its_surplus_to_a_long_one() {
        // Fixed 10, two flexible wanting 5 and 40, minimums 3 and 3, budget 40.
        // The pool is 40 - 10 - 6 = 24. The short cell takes 2 and gives back.
        let got = share_width(10, &[3, 3], &[5, 40], 40);
        assert_eq!(got[0], 5, "a cell never gets more than it wants");
        assert_eq!(got.iter().sum::<usize>(), 30, "the whole pool is used");
        assert!(got[1] > got[0], "the hungry cell gets the surplus");
    }

    #[test]
    fn below_the_floor_every_flexible_cell_gets_its_minimum() {
        assert_eq!(share_width(30, &[3, 3], &[20, 20], 10), vec![3, 3]);
    }

    #[test]
    fn no_flexible_cells_is_not_a_panic() {
        assert!(share_width(10, &[], &[], 80).is_empty());
    }
}

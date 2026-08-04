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

/// How the panel banner draws the host name.
///
/// A user preference, not a detection. A TUI is handed a byte stream: it does
/// not know the terminal's font and no escape sequence asks. `Wide` is offered
/// to the user who knows their font has fullwidth Latin glyphs (U+FF01-FF5E),
/// which Menlo, SF Mono, `JetBrains` Mono and Berkeley Mono do not -- without
/// them the banner is drawn by a fallback CJK face, a different typeface and
/// baseline from every line beneath it. Nothing here guesses which the user has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BannerStyle {
    /// Plain ASCII in the accent colour. One cell per character.
    #[default]
    Plain,
    /// Fullwidth Latin. Two cells per printable ASCII character.
    Wide,
}

impl BannerStyle {
    /// The config-file spelling, and what round-trips through it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Wide => "wide",
        }
    }

    /// Parse the config value. Anything unrecognised is `Plain`: an
    /// unreadable preference must not cost the user a legible banner.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("wide") {
            Self::Wide
        } else {
            Self::Plain
        }
    }

    /// What the user sees in Settings.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Plain => "Plain",
            Self::Wide => "Wide",
        }
    }

    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Plain => Self::Wide,
            Self::Wide => Self::Plain,
        }
    }
}

/// The host label for a panel banner, fitted to the width it has.
///
/// `style` chooses the glyphs, and only the glyphs: the fitting rule below is
/// the same either way, because it is the rule that keeps two machines from
/// naming themselves identically. `Wide` costs two cells per character, so it
/// is fitted against half the budget and then mapped -- it never gets its own
/// clipping path, which is exactly how the fullwidth banner used to lose the
/// digits.
///
/// **Identity outranks the preference.** Below the width where a wide banner
/// could still carry one distinguishing character, it falls back to plain.
/// A banner nobody can read a host name off is not a banner.
///
/// Returns the label **and the cells it occupies**, because the caller centres
/// it and cannot work that out safely from the string alone: `chars().count()`
/// undercounts a wide banner by half, and `fullwidth_display_width` answers a
/// different question -- the width `fullwidth(s)` *would* have, which is wrong
/// for an already-mapped string and wrong again for the plain fallback. A
/// function that returns text of a width only it can know hands back the width.
#[must_use]
pub fn fit_banner_styled(
    user: &str,
    host: &str,
    budget: usize,
    style: BannerStyle,
) -> (String, usize) {
    // Wide needs two cells per glyph, so it must have at least two to say
    // anything at all; below that the preference is dropped rather than the
    // host name.
    if style == BannerStyle::Wide && budget >= 2 {
        // Half the budget, conservatively: `fullwidth` leaves spaces and the
        // ellipsis single-width, so the mapped string is never wider than this
        // reserves. Fitting first and mapping second means the wide path cannot
        // disagree with the plain one about what to sacrifice.
        let plain = fit_banner(user, host, budget / 2);
        let cells = multitop_agent::fmt::fullwidth_display_width(&plain);
        return (multitop_agent::fmt::fullwidth(&plain), cells);
    }
    let plain = fit_banner(user, host, budget);
    let cells = plain.chars().count();
    (plain, cells)
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
    // One cell is not enough for an ellipsis *and* something to distinguish the
    // host by, and the ellipsis is the half that carries no information: at a
    // budget of 1 every host used to render as a bare `…`, which is the exact
    // failure this function exists to prevent, reached from the other end.
    if budget == 1 {
        return host.chars().last().map(String::from).unwrap_or_default();
    }
    // Keep the end: `…b-01` says more than `web-0…`.
    let tail: String = host
        .chars()
        .skip(host.chars().count() - (budget - 1))
        .collect();
    format!("\u{2026}{tail}")
}

/// Break prose onto lines no wider than `width`.
///
/// Panes hard-truncate rather than wrap -- `upgrade_view::next_action` keeps its
/// sentences "under ~40 visible columns" for exactly that reason, and
/// `config_ui` learned it when a delete confirmation lost its own
/// `[Esc] cancel` at 40 columns. A notice is prose: it has nothing to shed, so
/// it wraps or it loses the end, and the end of a notice is the part that says
/// what to do.
#[must_use]
pub fn wrap_words(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let extra = if line.is_empty() {
            word.chars().count()
        } else {
            word.chars().count() + 1
        };
        if !line.is_empty() && line.chars().count() + extra > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// The whole reason the wide banner was removed in the first place: it
    /// doubled the cell cost, clipped from the right, and the digits -- the
    /// only part that differs between hosts -- were what fell off. Turning the
    /// preference on must not be able to bring that back.
    #[test]
    fn a_wide_banner_still_tells_two_machines_apart() {
        for budget in 1..40usize {
            let (a, _) = fit_banner_styled("ztomer", "webserver-01", budget, BannerStyle::Wide);
            let (b, _) = fit_banner_styled("ztomer", "webserver-02", budget, BannerStyle::Wide);
            assert_ne!(a, b, "identical banners at budget {budget}: {a:?}");
        }
    }

    /// It must fit the cells it was given, and the width it reports must be
    /// the width it actually drew -- in both styles. The caller centres the
    /// banner with this number and cannot recover it from the string: the same
    /// `char` count means one width in plain and twice that in wide, and
    /// `fullwidth_display_width` answers neither question for a string that has
    /// already been mapped.
    #[test]
    fn the_reported_width_is_the_width_it_drew() {
        for style in [BannerStyle::Plain, BannerStyle::Wide] {
            for budget in 0..40usize {
                let (got, cells) = fit_banner_styled("ztomer", "webserver-01", budget, style);
                assert!(
                    cells <= budget,
                    "{style:?} used {cells} cells of {budget}: {got:?}"
                );
                let truth: usize = got
                    .chars()
                    .map(|c| usize::from(('\u{ff01}'..='\u{ff5e}').contains(&c)) + 1)
                    .sum();
                assert_eq!(cells, truth, "{style:?} misreported its own width: {got:?}");
            }
        }
    }

    /// Identity outranks the preference. Below the width where a wide banner
    /// could carry one distinguishing glyph, it draws plain rather than
    /// drawing nothing.
    #[test]
    fn a_wide_banner_falls_back_rather_than_vanishing() {
        let (got, cells) = fit_banner_styled("ztomer", "web-01", 1, BannerStyle::Wide);
        assert!(!got.is_empty(), "a one-cell banner is still a banner");
        assert_eq!(cells, 1);
    }

    /// Reached from the other end: at a budget of one, the ellipsis is the half
    /// that carries no information, so spending the only cell on it made every
    /// host render as a bare `…`.
    #[test]
    fn one_cell_is_spent_on_the_host_not_on_the_ellipsis() {
        let a = fit_banner("ztomer", "webserver-01", 1);
        let b = fit_banner("ztomer", "webserver-02", 1);
        assert_eq!(a, "1");
        assert_eq!(b, "2");
        assert_ne!(a, b);
    }

    /// The preference chooses glyphs and nothing else: what gets sacrificed is
    /// decided once, by the plain fitter, for both styles.
    #[test]
    fn wide_is_the_plain_fit_in_different_glyphs() {
        let plain = fit_banner("ztomer", "web-01", 10);
        let (wide, _) = fit_banner_styled("ztomer", "web-01", 20, BannerStyle::Wide);
        assert_eq!(wide, multitop_agent::fmt::fullwidth(&plain));
    }

    #[test]
    fn a_banner_style_round_trips_through_its_config_spelling() {
        for style in [BannerStyle::Plain, BannerStyle::Wide] {
            assert_eq!(BannerStyle::parse(style.as_str()), style);
        }
        // An unreadable preference must not cost a legible banner.
        assert_eq!(BannerStyle::parse("WIDE"), BannerStyle::Wide);
        assert_eq!(BannerStyle::parse("gothic"), BannerStyle::Plain);
        assert_eq!(BannerStyle::parse(""), BannerStyle::Plain);
        assert_eq!(BannerStyle::default(), BannerStyle::Plain);
    }

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

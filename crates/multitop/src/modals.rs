use multitop_agent::palette_view::PaletteView;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;

/// A modal's ink, from the PAGE chrome: a modal clears its rectangle to the
/// terminal's own background and draws on that, so every role here is adapted
/// against the page.
///
/// This replaced 24 named terminal colours (`White`, `DarkGray`, `Yellow`,
/// `Cyan`, `Red`) that no palette adapted, and two modals -- help and the
/// vault wait -- that drew `THEMES[0]`'s raw values whatever theme was on and
/// whatever the terminal painted.
struct ModalInk {
    text: Style,
    muted: Style,
    key: Style,
    warn: Style,
    alert: Style,
    prompt: Style,
    title: Style,
    border: Style,
}

fn modal_ink(theme: &PaletteView) -> ModalInk {
    use crate::ui::ink;
    let page = &theme.page;
    ModalInk {
        text: Style::default().fg(ink(page.text)),
        muted: Style::default().fg(ink(page.muted)),
        key: Style::default()
            .fg(ink(page.text))
            .add_modifier(Modifier::BOLD),
        warn: Style::default().fg(ink(page.warn)),
        alert: Style::default().fg(ink(page.alert)),
        prompt: Style::default().fg(ink(page.border)),
        title: Style::default()
            .fg(ink(page.accent))
            .add_modifier(Modifier::BOLD),
        border: Style::default().fg(ink(page.border)),
    }
}

/// Which slow vault operation the "please wait" modal is reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waiting {
    /// A Touch ID / fingerprint prompt is outstanding.
    Biometric,
    /// Argon2id is checking a typed master password.
    Verifying,
    /// Argon2id is deriving the key for a brand new vault.
    Creating,
}

impl Waiting {
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Biometric => " Vault Locked ",
            Self::Verifying => " Unlocking Vault ",
            Self::Creating => " Creating Vault ",
        }
    }

    pub(crate) const fn headline(self) -> &'static str {
        match self {
            Self::Biometric => "  Unlocking with Touch ID / fingerprint\u{2026}",
            Self::Verifying => "  Checking the master password\u{2026}",
            Self::Creating => "  Encrypting the new vault\u{2026}",
        }
    }

    pub(crate) const fn hint(self) -> &'static str {
        match self {
            Self::Biometric => "  Esc to cancel and use the vault password instead.",
            Self::Verifying => "  This takes a moment by design. Esc to cancel.",
            // Says not to retype, because retyping is exactly what the empty
            // field invited before this modal existed -- and every extra Enter
            // was another vault initialisation.
            Self::Creating => "  This takes a moment by design \u{2014} no need to type it again.",
        }
    }
}

/// The "please wait" modal, shown while a slow vault operation is outstanding.
pub fn draw_vault_awaiting_biometric(f: &mut Frame, waiting: Waiting, theme: &PaletteView) {
    let area = f.area();
    let popup_width = (64u16).min(area.width.saturating_sub(2));
    let popup_height = (8u16).min(area.height.saturating_sub(2));

    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_rect = Rect::new(x, y, popup_width, popup_height);

    let ink = modal_ink(theme);
    let block = ratatui::widgets::Block::default()
        .title(waiting.title())
        .title_style(ink.title)
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(ink.border);

    f.render_widget(ratatui::widgets::Clear, popup_rect);

    let lines = vec![
        Line::from(""),
        Line::from(vec![Span::styled(waiting.headline(), ink.text)]),
        Line::from(""),
        Line::from(vec![Span::styled(waiting.hint(), ink.muted)]),
    ];

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, popup_rect);
}

/// Horizontal padding inside a modal's border, in cells.
///
/// Named because the height calculation must subtract exactly what the block
/// adds, and because the indent belongs to the block rather than to each line:
/// carried per line, a wrapped continuation starts at the border.
const PAD: u16 = 2;

/// Keys table from `README.md:82`, rendered as the `?` overlay.
///
/// The table is the one place the keys are named in English rather than as
/// the house TUI lib's (`gates_of_heck/tui/lib.sh`) `→` glyphs, so a key that does nothing is visible immediately.
/// Drawn with `Rams` dim + `NO_COLOR` aware via `Palette`.
pub fn draw_command_palette(f: &mut Frame, app: &App) {
    let area = f.area();
    let popup_width = (72u16).min(area.width.saturating_sub(4));
    let popup_height = (12u16).min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let rect = Rect::new(x, y, popup_width, popup_height);

    let ink = modal_ink(&app.current_theme());
    let block = ratatui::widgets::Block::default()
        .title(" Command Palette  : to open ")
        .title_style(ink.title)
        .borders(ratatui::widgets::Borders::ALL)
        .padding(ratatui::widgets::Padding::horizontal(PAD))
        .border_style(ink.border);

    let input = app.command_input.as_str();
    let filter = input.to_lowercase();
    let all = [
        "filter <query>",
        "clear filter",
        "upgrade",
        "docker",
        "fetch",
        "graphs",
        "stats",
        "sort cpu",
        "sort mem",
        "theme",
        "add server",
        "vault unlock",
        "yank",
    ];
    let shown: Vec<&str> = all
        .iter()
        .copied()
        .filter(|c| filter.is_empty() || c.contains(&filter))
        .take(8)
        .collect();

    let mut rows: Vec<Line> = vec![
        Line::from(vec![
            Span::styled(": ", ink.warn),
            Span::styled(input.to_string(), ink.text),
            Span::styled("\u{2588}", ink.warn),
        ]),
        Line::from(""),
    ];
    for cmd in &shown {
        rows.push(Line::from(vec![Span::styled(
            format!("  {cmd}"),
            ink.muted,
        )]));
    }
    if shown.is_empty() {
        rows.push(Line::from(vec![Span::styled("  No matches", ink.muted)]));
    }
    rows.push(Line::from(""));
    rows.push(Line::from(vec![
        Span::styled("Enter", ink.key),
        Span::styled(" run  ", ink.muted),
        Span::styled("Esc", ink.key),
        Span::styled(" close", ink.muted),
    ]));

    f.render_widget(ratatui::widgets::Clear, rect);
    let para = Paragraph::new(rows).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, rect);
}

/// Every main-view key, one per line, in keybar order: views first, then
/// doors, then the rest. A clustered row cannot hold them all, and the last
/// attempt left H, y, z and the palette out entirely.
const HELP_ROWS: &[(&str, &str)] = &[
    ("q", "quit (terminates every SSH session)"),
    ("s", "per server statistics"),
    ("d", "docker containers per server"),
    ("f", "fetch OS, kernel and model card"),
    ("g", "graphs from stats history"),
    ("p", "ops: health, cron, containers, alerts from mcp_host"),
    ("u", "update status, press again to run"),
    ("/", "filter the grid"),
    ("e", "settings: servers, passwords, vault"),
    ("H", "alerts from CPU, memory and disk thresholds"),
    ("y", "yank, copy selected panel SSH target"),
    ("t", "cycle theme"),
    ("c", "sort processes by CPU"),
    ("m", "sort processes by memory"),
    ("z", "focus selected panel alone"),
    ("1-9", "select panel"),
    ("x", "kill top process (asks first)"),
    ("o", "journal of top process"),
    ("r", "renice top process"),
    ("l", "tail the syslog"),
    ("+", "zoom graphs in"),
    ("-", "zoom graphs out"),
    (":", "command palette"),
    ("?", "this help"),
];

pub fn draw_help(f: &mut Frame, theme: &PaletteView) {
    let area = f.area();
    let popup_width = (72u16).min(area.width.saturating_sub(4));
    // One row per key plus border, title and the two note lines: a fixed
    // height would clip the moment a binding is added.
    let popup_height = u16::try_from(HELP_ROWS.len() + 8)
        .unwrap_or(u16::MAX)
        .min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let rect = Rect::new(x, y, popup_width, popup_height);

    let ink = modal_ink(theme);
    let block = ratatui::widgets::Block::default()
        .title(" Help  ? to close ")
        .title_style(ink.title)
        .borders(ratatui::widgets::Borders::ALL)
        .padding(ratatui::widgets::Padding::horizontal(PAD))
        .border_style(ink.border);

    let mut rows: Vec<Line> = HELP_ROWS
        .iter()
        .map(|(key, what)| {
            Line::from(vec![
                Span::styled(*key, ink.warn),
                Span::styled(format!(" - {what}"), ink.text),
            ])
        })
        .collect();
    rows.push(Line::from(""));
    rows.extend([
        Line::from(vec![Span::styled(
            "Upgrade is two presses: u shows what would run, u again confirms.",
            ink.muted,
        )]),
        Line::from(vec![Span::styled(
            "Filter searches host/user + whatever the view shows (process, container, OS).",
            ink.muted,
        )]),
    ]);

    f.render_widget(ratatui::widgets::Clear, rect);
    let para = Paragraph::new(rows).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, rect);
}

/// What the vault password prompt says, in the arrangement that fits.
struct PromptText<'a> {
    ink: &'a ModalInk,
    creating: bool,
    password_dots: &'a str,
    error: Option<&'a str>,
}

impl PromptText<'_> {
    /// The prompt's lines: `explain` adds the recovery-path sentence when
    /// creating, `blanks` the spacing rows. The headline, the field and the
    /// footer naming both keys are always present.
    fn lines(&self, explain: bool, blanks: bool) -> Vec<Line<'static>> {
        let creating = self.creating;
        let ink = self.ink;
        let password_dots = self.password_dots.to_string();
        let error = self.error.map(str::to_string);
        let mut lines: Vec<Line<'static>> = Vec::new();
        if blanks {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![Span::styled(
            if creating {
                "Choose a master password for your new vault:"
            } else {
                "Enter vault master password to unlock:"
            },
            ink.text,
        )]));
        if blanks {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![
            Span::styled("> ", ink.prompt),
            Span::styled(password_dots, ink.text),
        ]));
        if creating && explain {
            lines.push(Line::from(vec![Span::styled(
                "Encrypts sudo passwords. Touch ID unlocks it day to day; \
             this password is the recovery path if that key is lost.",
                ink.muted,
            )]));
        }
        if let Some(error) = &error {
            lines.push(Line::from(vec![Span::styled(error.clone(), ink.alert)]));
        }
        if blanks {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(vec![
            Span::styled("Press ", ink.muted),
            Span::styled("Enter", ink.key),
            Span::styled(
                if creating {
                    " to create, "
                } else {
                    " to unlock, "
                },
                ink.muted,
            ),
            Span::styled("Esc", ink.key),
            Span::styled(" to cancel", ink.muted),
        ]));
        lines
    }
}

/// The prompt for unlocking an existing vault, or choosing a new master
/// password.
///
/// # Why the content is assembled before the box is sized
///
/// The height was a literal -- 12 when creating, 10 when unlocking -- and the
/// comment beside it recorded that it had *already* been bumped once because it
/// "clipped the `Press Enter to create` footer off the bottom". A guessed line
/// count is only ever right at the width it was guessed for. At 40 columns the
/// box is 36 cells wide and every line of prose in it was cut mid-word:
///
/// ```text
/// |  Enter vault master password to unl|
/// |  Press Enter to unlock, Esc to canc|
/// ```
///
/// A modal that amputates its own way out is the defect Kare's ruling removed
/// from the upgrade confirmation by making it a keybar row. A password prompt
/// cannot become a keybar row -- it needs a field -- so it does the other half
/// of that ruling instead: **it sheds.**
///
/// # The shed order
///
/// Wrapping alone is not enough. A twelve-row terminal leaves eight rows inside
/// the borders, and the create prompt's full content needs nine. So the parts
/// are ranked, exactly as `fit_row` ranks a keybar's chunks:
///
/// 1. the explanation of what the password is for -- useful, never essential
/// 2. the blank lines that space the block out -- decoration
///
/// The headline, the field and the footer naming both keys are never shed. An
/// operator who cannot read what the password protects can still act; one who
/// cannot see `Esc` is stuck.
pub fn draw_vault_password_prompt(f: &mut Frame, app: &App) {
    let area = f.area();
    let popup_width = (64u16).min(area.width.saturating_sub(2));

    let ink = modal_ink(&app.current_theme());

    // The same prompt serves unlocking an existing vault and choosing the
    // master password for a new one; only the wording differs.
    let creating = app.vault_creating();
    let block = ratatui::widgets::Block::default()
        .title(if creating {
            " Create Vault "
        } else {
            " Vault Password "
        })
        .title_style(ink.title)
        .borders(ratatui::widgets::Borders::ALL)
        .padding(ratatui::widgets::Padding::horizontal(PAD))
        .border_style(ink.border);

    let password_dots = crate::fmt::mask_secret(app.vault_password_input());
    let error: Option<String> = app
        .vault_create_error()
        .or_else(|| app.vault_password_error())
        .map(ToString::to_string);

    let prompt = PromptText {
        ink: &ink,
        creating,
        password_dots: &password_dots,
        error: error.as_deref(),
    };
    let compose = |explain: bool, blanks: bool| prompt.lines(explain, blanks);

    // The rows the box can hold inside its border, at the width it will be
    // drawn at.
    let inner_w = popup_width.saturating_sub(2 + PAD * 2) as usize;
    let budget = usize::from(area.height.saturating_sub(4));
    let wrapped_len = |lines: &[Line<'static>]| -> usize {
        lines
            .iter()
            .map(|l| {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                crate::layout::wrap_words(&text, inner_w).len().max(1)
            })
            .sum()
    };

    // First arrangement that fits, in shed order. The last is the floor: when
    // even that overflows the terminal is too short for a prompt at all, and
    // the box is clamped to the screen rather than the content.
    let mut lines = compose(false, false);
    for candidate in [compose(true, true), compose(false, true)] {
        if wrapped_len(&candidate) <= budget {
            lines = candidate;
            break;
        }
    }
    let needed = wrapped_len(&lines);

    let popup_height = u16::try_from(needed + 2)
        .unwrap_or(u16::MAX)
        .min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;
    let popup_rect = Rect::new(x, y, popup_width, popup_height);

    f.render_widget(ratatui::widgets::Clear, popup_rect);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, popup_rect);
}

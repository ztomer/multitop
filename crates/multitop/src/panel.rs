use crate::config::Server;
use multitop_agent::fetch::FetchSnapshot;

/// A bounded ring of lines that reuses its memory.
///
/// At most `cap` slots indexed by a head pointer: once the ring is full,
/// pushing a line overwrites the oldest slot in place -- its `String`
/// allocation is cleared and reused for the new line. Nothing is shifted and
/// nothing is freed on the hot path. The `Vec::drain(0..k)` that `apt upgrade`
/// output used to pay on every line once the buffer filled -- a memmove of
/// thousands of `String` headers -- is gone, and a log that streams forever
/// settles into one allocation per slot.
///
/// The slots are grown on demand rather than reserved up front, so eight idle
/// panels do not each carry `upgrade_history_lines` (5000 by default) empty
/// `String` headers for a log that may never be written to.
#[derive(Clone, Debug)]
pub struct RingLines {
    /// The live lines, oldest at `head`. Never longer than `cap`; shorter until
    /// the ring has filled once, and `head` is 0 for exactly that long.
    slots: Vec<String>,
    /// Index of the oldest live line. Meaningful only once `slots.len() == cap`.
    head: usize,
    /// Maximum live lines; the oldest is overwritten when this is reached.
    cap: usize,
}

impl RingLines {
    #[must_use]
    pub const fn new(cap: usize) -> Self {
        Self {
            slots: Vec::new(),
            head: 0,
            cap,
        }
    }

    /// Change the capacity, keeping the newest `cap` lines.
    ///
    /// Called when configuration loads and when the panel list is rebuilt,
    /// never on the streaming path.
    pub fn set_cap(&mut self, cap: usize) {
        if cap == self.cap {
            return;
        }
        let drop_front = self.slots.len().saturating_sub(cap);
        let keep: Vec<String> = self.iter().skip(drop_front).cloned().collect();
        self.slots = keep;
        self.head = 0;
        self.cap = cap;
    }

    pub fn push(&mut self, line: String) {
        if self.cap == 0 {
            return;
        }
        if self.slots.len() < self.cap {
            // Not full yet, so `head` is still 0 and appending keeps the order.
            self.slots.push(line);
        } else {
            // Full: overwrite the oldest slot in place, reusing its allocation.
            let slot = &mut self.slots[self.head];
            slot.clear();
            slot.push_str(&line);
            self.head = (self.head + 1) % self.cap;
        }
    }

    /// Overwrite the line `back` positions from the newest, where 0 is the
    /// newest. Out of range does nothing.
    ///
    /// This is what lets a tool that repaints several lines in place update
    /// them instead of appending another copy of its block on every tick.
    pub fn overwrite_from_end(&mut self, back: usize, line: &str) {
        // `checked_add` first: `back` comes off the wire as a cursor count and
        // `usize::MAX + 1` is a panic, not a miss.
        let Some(from_end) = back.checked_add(1) else {
            return;
        };
        let Some(i) = self.slots.len().checked_sub(from_end) else {
            return;
        };
        let Some(slot) = self.slot_mut(i) else {
            return;
        };
        slot.clear();
        slot.push_str(line);
    }

    /// The `i`th live line, oldest first, mutably.
    fn slot_mut(&mut self, i: usize) -> Option<&mut String> {
        if i >= self.slots.len() {
            return None;
        }
        let idx = if self.slots.len() < self.cap {
            i
        } else {
            (self.head + i) % self.cap
        };
        self.slots.get_mut(idx)
    }

    /// Replace the contents, keeping the capacity. Used to reset a log to a
    /// fixed initial state (a skip message, a test fixture) while it must
    /// still hold `upgrade_history_lines` lines.
    pub fn replace_with(&mut self, lines: impl IntoIterator<Item = String>) {
        self.clear();
        for line in lines {
            self.push(line);
        }
    }

    pub fn clear(&mut self) {
        self.slots.clear();
        self.head = 0;
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    #[must_use]
    pub fn last(&self) -> Option<&String> {
        self.get(self.slots.len().checked_sub(1)?)
    }

    /// The `i`th live line, oldest first.
    #[must_use]
    pub fn get(&self, i: usize) -> Option<&String> {
        let n = self.slots.len();
        (i < n).then(|| &self.slots[(self.head + i) % n])
    }

    pub fn iter(&self) -> impl Iterator<Item = &String> {
        (0..self.slots.len()).filter_map(move |i| self.get(i))
    }

    /// The `count` lines starting at `start`, as an iterator over the live
    /// slots. Out-of-range requests yield nothing.
    pub fn slice(&self, start: usize, count: usize) -> impl Iterator<Item = &String> {
        let start = start.min(self.slots.len());
        let count = count.min(self.slots.len() - start);
        (0..count).filter_map(move |i| self.get(start + i))
    }
}

impl From<Vec<String>> for RingLines {
    /// A ring holding exactly the fixture that built it, with a real capacity
    /// so it keeps behaving like one afterwards.
    ///
    /// The capacity is *not* the fixture's length. It was, and that made
    /// `RingLines::from(Vec::new())` a ring of capacity zero whose every
    /// subsequent `push` was a silent no-op -- a seeded panel that then
    /// streamed an upgrade would have shown nothing, and nothing would have
    /// said why.
    fn from(lines: Vec<String>) -> Self {
        let cap = lines
            .len()
            .max(crate::config::DEFAULT_UPGRADE_HISTORY_LINES);
        Self {
            slots: lines,
            head: 0,
            cap,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Monitor,
    Docker,
    Fetch,
    Upgrade,
    /// The same Monitor stream, drawn as history rather than as a moment.
    Graphs,
    Alerts,
}

impl Mode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Monitor => "monitor",
            Self::Docker => "docker",
            Self::Fetch => "fetch",
            Self::Upgrade => "upgrade",
            Self::Graphs => "graphs",
            Self::Alerts => "alerts",
        }
    }

    #[must_use]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "monitor" => Some(Self::Monitor),
            "docker" => Some(Self::Docker),
            "fetch" => Some(Self::Fetch),
            "upgrade" => Some(Self::Upgrade),
            "graphs" => Some(Self::Graphs),
            "alerts" => Some(Self::Alerts),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpgradeState {
    #[default]
    NIL,
    STARTED,
    DONE,
}

#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct Panel {
    pub server: Server,
    pub mode: Mode,
    pub gen: u64,
    pub last_frame: Option<Vec<String>>,
    pub last_fetch: Option<FetchSnapshot>,
    pub last_upgrade: RingLines,
    pub upgrade_state: UpgradeState,
    pub upgrade_gen: u64,
    pub last_monitor: Option<multitop_agent::proto::Payload>,
    /// What the Monitor packets said, kept so the `G` view has a past to
    /// draw. Filled whatever view the panel is in.
    pub history: crate::history::History,
    pub last_docker: Option<multitop_agent::proto::Payload>,
    pub view: Vec<String>,
    /// How far the pane the user is *currently looking at* is scrolled back.
    pub scroll_offset: usize,
    /// Where the Upgrade log was left, kept separately so leaving that view and
    /// coming back returns to the same place.
    ///
    /// One shared offset could do neither: resetting it on every switch lost
    /// the user's place in a running log, and not resetting it leaked the log's
    /// offset into whichever view was entered next — the Monitor pane opened
    /// scrolled to a position that meant nothing in it.
    pub upgrade_scroll_offset: usize,
    pub sudo_password: Option<String>,
    pub password_saved: bool,
    pub external_password: bool,
    /// A credential-store lookup has been answered: either it found nothing
    /// worth keeping, or `sudo_password` holds the answer. Set *before* a load
    /// is dispatched (an unanswered lookup must not be re-dispatched), which is
    /// the same moment the old synchronous path used to set it.
    pub password_checked: bool,
    /// A credential-store lookup is in flight, dispatched off the loop thread.
    /// The UI shows `Checking` while it is set, and a confirm is deferred so an
    /// upgrade never starts on a password it has not actually read.
    pub password_checking: bool,

    /// Things the app has told the user, kept out of `view` so a frame cannot
    /// destroy them.
    ///
    /// `view` is *derived state*: [`Panel::show_frame`] rebuilds it from an
    /// agent frame on every packet. A notice pushed straight into `view` was
    /// therefore erased by the next frame -- about a second after startup,
    /// which is exactly when every startup notice is written: the
    /// plaintext-password migration, a clamped `upgrade_history_lines`, an
    /// unreadable `state.toml`, a failed state write. Each appeared for one
    /// second and was gone.
    ///
    /// That is the failure [`Panel::note`]'s own doc comment exists to prevent
    /// -- "a message built, stored, and never drawn" -- reached by the other
    /// door: not written to the wrong buffer, written to one that is rebuilt.
    pub notes: Vec<String>,
}

/// How many notices a pane carries. They are all things the user has to act on,
/// so they stick; the bound is only so a repeated one (a state write failing
/// once per upgrade) cannot crowd out the pane it is drawn in.
const MAX_NOTES: usize = 4;

impl Panel {
    #[must_use]
    pub fn new(server: Server) -> Self {
        let pal = &multitop_agent::color::ANSI;
        let history = crate::history_store::load(&server.host)
            .or_else(|| crate::history_store::load(&crate::password_store::account(&server)))
            .unwrap_or_default();
        Self {
            server,
            mode: Mode::Monitor,
            gen: 0,
            last_frame: None,
            last_fetch: None,
            last_upgrade: RingLines::new(crate::config::DEFAULT_UPGRADE_HISTORY_LINES),
            upgrade_state: UpgradeState::NIL,
            upgrade_gen: 0,
            last_monitor: None,
            history,
            last_docker: None,
            // Row 0 belongs to the host banner, which `ui::draw` composes over
            // whatever is there. A body that starts at row 0 therefore has its
            // first line eaten -- and this body is one line long, so the whole
            // of it was eaten and a host coming up rendered as an empty box,
            // indistinguishable from a hung SSH session or a dead app.
            view: vec![
                String::new(),
                format!("{}connecting...{}", pal.muted(), pal.reset),
            ],
            scroll_offset: 0,
            upgrade_scroll_offset: 0,
            notes: Vec::new(),
            sudo_password: None,
            password_saved: false,
            external_password: false,
            password_checked: false,
            password_checking: false,
        }
    }

    /// Append a user-facing notice to this panel.
    ///
    /// # One destination
    ///
    /// This used to branch on the panel's mode: the ring in the Upgrade view,
    /// `notes` everywhere else. That was wrong twice over.
    ///
    /// First it decided *visibility* from the mode at the moment the notice was
    /// **written**, while `ui::pane_lines` decides which pane to draw from the
    /// mode at the moment of the **frame**. The two need not agree, and for the
    /// notices that matter they never did: every startup notice is written in
    /// Monitor mode, so pressing `u` made them all vanish. `notes` is drawn by
    /// every pane now, which fixed that.
    ///
    /// What the branch became after that was a *placement* choice -- and it made
    /// the same notice appear twice. A state write that fails once in the
    /// Monitor view and again during a run leaves one copy in `notes` and one in
    /// the ring, and the Upgrade pane draws both. The ring copy is also the
    /// worse of the two: the ring is fitted to the pane, not wrapped, so it
    /// arrives hard-truncated mid-word, which is the exact defect the wrapping
    /// in `pane_lines` exists to prevent.
    ///
    /// So there is one destination. Upgrade *output* still goes to the ring --
    /// `Msg::Status` and `note_nothing_to_upgrade` push there directly, and
    /// belong in the log in order. An app-level notice is not upgrade output.
    pub fn note(&mut self, line: String) {
        // Already on screen. A second copy says nothing new and takes a row from
        // the pane, and the bound on notices is the whole reason the pane still
        // shows the host. Checked against every notice held rather than only the
        // last: two that alternate -- a failed state write and a vault report,
        // each repeating once per run -- passed a `last()`-only guard every
        // single time, which is the shape of guard this round keeps finding.
        if self.notes.contains(&line) {
            return;
        }
        if self.notes.len() >= MAX_NOTES {
            self.notes.remove(0);
        }
        self.notes.push(line);
    }

    /// Show app-authored lines in the pane, reserving row 0 for the banner.
    ///
    /// `ui::draw` replaces `lines[0]` with the host banner on every frame. A
    /// body that starts at row 0 therefore has its first line eaten -- and a
    /// one-line body is eaten *whole*, which is how a connection error written
    /// into the pane was destroyed on the frame it arrived.
    ///
    /// `Panel::new` and [`Panel::show_last_frame`] reserved that row; the two
    /// message arms that build a body from scratch did not. One place to do it
    /// now, so a third cannot forget. Rendered agent frames go through
    /// [`Panel::show_frame`] instead -- they carry their own row 0.
    pub fn show_body(&mut self, lines: impl IntoIterator<Item = String>) {
        self.view = std::iter::once(String::new()).chain(lines).collect();
    }

    /// Show a rendered frame.
    ///
    /// The one place that replaces `view` with a frame -- it was four, and a
    /// notice re-appended in one of them would have been dropped by the other
    /// three. Notices are no longer appended here at all: they live in
    /// [`Panel::notes`] and are added by `ui::pane_lines`, which is the only
    /// place that knows the pane's width and can therefore wrap them. Appended
    /// here they were hard-truncated by the pane, which loses the end of a
    /// sentence -- and the end of a notice is the part that says what to do.
    pub fn show_frame(&mut self, lines: Vec<String>) {
        self.view = lines;
    }

    /// Whether this panel still needs a credential-store lookup. True only for
    /// a host that has never been answered -- a rejected or absent lookup is an
    /// answer, exactly as it was when the read was synchronous.
    #[must_use]
    pub const fn needs_credential_load(&self) -> bool {
        !self.password_checked && self.sudo_password.is_none()
    }

    /// Mark that a credential-store lookup has been dispatched for this panel.
    /// It must be called before the off-thread read starts, so a slow store can
    /// never cause a second lookup of a panel already being answered.
    pub const fn mark_credential_load_dispatched(&mut self) {
        self.password_checked = true;
        self.password_checking = true;
    }

    /// The off-thread lookup answered. `Some` is kept, silence and errors are
    /// both "no stored password" (the upgrade will prompt on the pty, as before
    /// the synchronous path was moved off the loop thread).
    pub fn answer_credential_load(&mut self, result: Result<Option<String>, String>) {
        self.password_checking = false;
        if let Ok(Some(pass)) = result {
            self.sudo_password = Some(pass);
            self.password_saved = true;
        }
    }

    pub fn set_sudo_password(&mut self, password: String, from_vault: bool) {
        self.sudo_password = Some(password);
        self.password_checked = true;
        // A password landing here (e.g. a vault copy) answers any lookup now
        // in flight; a stale `checking` would defer the confirm forever.
        self.password_checking = false;
        if from_vault {
            self.external_password = true;
        }
    }

    pub fn show_last_frame(&mut self) {
        let pal = &multitop_agent::color::ANSI;
        let lines = self.last_frame.as_ref().map_or_else(
            || {
                // Row 0 is the banner's; see `Panel::new`. An agent frame
                // already carries its own line 0 for the banner to replace,
                // but this fallback is written here and must reserve it, or it
                // says nothing at all.
                vec![
                    String::new(),
                    format!("{}waiting for data...{}", pal.meter_mid(), pal.reset),
                ]
            },
            std::clone::Clone::clone,
        );
        self.show_frame(lines);
    }
}

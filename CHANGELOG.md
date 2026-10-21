# Changelog

All notable changes to `monitor`. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

This file starts at v0.47.0 — earlier history is in git (`git log`), and the
per-defect record is `docs/detection-record.md`, which is the more useful
document for anything before this point.

## v0.64.0 — the kept catalogue is checked against the server's own DIGEST _(2026-10-21)_

### Changed — the kept list is checked against the server's WORD, not a guess (servers ROADMAP E72)
- **THE KEPT LIST WAS CHECKED BY A GUESS.** The client decided that its
  kept copy of a host's tool list was still the truth by INFERring it from
  two things it could see cheaply: the server's advertised **version** and
  the set of tools the server says it does **not** offer. Both are hints,
  not facts — and the guess was wrong often enough to measure: it sent the
  client back for the whole catalogue at every release, and **four of the
  last ten releases touched no tool table at all** (v1.99.13→14, 15→16,
  16→17, 17→18), so those downloads were needless — and, the other way
  round, a table that moved WITHOUT a release was invisible to it.
- **THE SERVER NOW PUBLISHES THE DIGEST OF THE EXACT LIST `tools/list`
  ANSWERS**, in `server/discover`, beside the absent map this client
  already reads: `dev.multitop/catalogueDigest`, the SHA-256 of the
  list's own canonical bytes, in the estate's spelling (`sha256:` and 64
  lowercase hex). That is the server's own word about the list it serves,
  and it is the fact the client checks its copy against — the server's
  half is `media_server` `7c3b02b1`, shipped there and not written here.
- **A DIGEST THAT MATCHES KEEPS THE KEPT LIST — ACROSS A RELEASE.** The
  version moving no longer costs a download: the digest says the list is
  byte-for-byte the one the client holds, so a release that touched no
  tool table leaves the catalogue alone (**149,997 B a gather** measured
  on the live estate today). The client asks for the list only when the
  digest says the list moved.
- **A DIGEST THAT CHANGED PUTS THE KEPT LIST ASIDE**, and the host is
  asked again — and this is the case the guess could not see at all: the
  same binary, the same absent set, a different digest. The frame says
  so: *"tool list: re-listed, the server's catalogue digest changed"*.
- **THE DIGEST IS REMEMBERED, NOT COMPUTED.** It is of the **whole** list
  — names **and** schemas — and this client keeps only the names, so it
  cannot compute it again: it keeps the digest it was **handed**, beside
  the list the word was given for. A digest over the names alone would be
  a smaller fact and a weaker one (a schema change would go unseen); the
  server publishes the stronger one, and the client takes it.
- **A DIGEST THE CLIENT CANNOT CHECK SAYS THE FACT IS ABSENT.** A server
  that publishes none — one too old to know the field, or the local
  agent, which publishes none — has said nothing about its list: the kept
  copy is then checked the OLDER way (the version and the absent set),
  the frame says *"the server published no digest"*, and **that is not an
  all-clear**: an absent fact is stated, never drawn as a copy-is-fine. A
  digest that is not in the estate's spelling is not a digest either, and
  is treated the same way.
- **AND THE CLIENT DOES NOT RE-LIST EVERY SESSION.** The absent digest
  does not turn into a download on every pass — that is the defect this
  closes, not a new one. A kept list taken before the server published a
  digest is put aside **once** (it cannot be checked), and the digest is
  remembered with the list from then on.
- **THE KEPT LIST IS STILL THE WHOLE LIST** — an optimisation, never a
  filter (E61's rule): every tool of it, every page of it, by name.

### Measured (2026-10-21, on the live estate and on the shipped binary)
- **THE ESTATE'S HALF OF THE BYTES**: the digest costs **+105 B per
  session per host** on the wire (the server's own measurement: its
  `server/discover` answer grew 3,457 → 3,562 B on a fixture host, +0.2%)
  and saves the **149,997 B a gather** the version inference forced at
  every release — the three `tools/list` answers measured today: 65,557 B
  on `.33`, 51,716 B on `.90`, 32,724 B on `.158`.
- **THE CLIENT'S HALF, ON DISK**: the kept list grows by **87 B per host**
  (672 → 759 B on `.33` — the 71-byte digest and the JSON around it; 18 B of
  that until the fleet ships the digest, when the word is there to keep) and
  the answer the poll keeps grows by **108 B per host per gather** (its
  catalogue field, 39 → 147 B — it now carries the proof). Neither is on
  the wire.
- **THE PASS, WITH AND WITHOUT THE KEPT LISTS**, measured with the
  counting shim in front of `ssh` (`tools/measure_poll.py`): **148,618 B
  a gather** with the kept list (no `tools/list` line in the per-answer
  table at all) against **298,617 B** with them dropped — the same
  149,997 B of catalogue, measured twice.
- **THE ESTATE HAS NO DIGEST YET** (the server's release is the
  coordinator's, and v1.99.21 is not shipped), so this measurement is the
  **fallback path**: the client keeps the list the old way and says the
  digest was absent. That is the path measured above, and it is
  unchanged by this release — the saving lands when the fleet ships.

### Verified
- `tools/gate.sh --full` green; the Rust suite green.
- The tests were **red first**: a changed digest did not put the kept list
  aside, a release that changed no tool re-listed the catalogue anyway,
  the digest was not kept with the list, and the frame said nothing about
  the digest — all four failed against the old code, and pass now.

## v0.63.0 — the Ops poll stops re-downloading the TOOL CATALOGUE _(2026-10-21)_

### Added — the tool list is kept, keyed to what determines it (servers ROADMAP E61)
- **THE OPS POLL PULLED THE WHOLE CATALOGUE, EVERY GATHER.** The list of
  tools a host offers is the biggest thing this client pulls off a host —
  **65,467 B on `.33`**, 51,716 B on `.90`, 32,724 B on `.158`, measured on
  the live estate — and it is the SAME list every time: it changes at a
  RELEASE, not every six and a half minutes. At the scheduler's own rate
  (a pass every 390 s) that is **149,907 B a gather, 33.1 MB a day**, of
  one immutable list, over and over.
- **THE KEY IS (the server's advertised version, the set of tools it says
  it does NOT offer)**, and BOTH are read from the LIVE server on every
  session, so the kept list cannot hide a change: a release moves the
  version, and a host that gains or loses a tool moves the absent set. A
  kept list taken under another binary, or under another tool set, is put
  aside and the host is asked again — and the reasons the server gives for
  what it does not list are read live every session, so they are never
  the kept copy's.
- **THE SERVER'S OWN `ttlMs` BOUNDS IT** — an hour, for a list — so this
  client never keeps an answer longer than the answer's owner says it is
  good for. A window of zero is the protocol's "a live fact": there is
  nothing to keep, so there is nothing to use, and nothing is kept. A
  server that names no version is not keyed at all, so nothing is kept
  either: the version is the only thing that says which binary holds the
  tool table.
- **THE CACHE IS AN OPTIMISATION, NEVER A FILTER.** What is kept is EVERY
  tool the server listed, every page of them: a tool the kept list omitted
  would lose its section from the panel, and a list that kept only the
  first page would lose every tool on the second.
- **WHERE THE LIST CAME FROM IS ON THE PANEL.** The list is the panel's
  SHAPE — a tool the list does not name has no section — so the frame says
  it: *"tool list: kept 12m old (window 1h)"*, *"tool list: from the
  server (window 1h)"*, and, when a kept list was put aside, WHY: *"tool
  list: re-listed, the kept list was for 1.99.19, the server is 1.99.20"*,
  *"… the kept list had expired (2h old, window 1h)"*. A list nobody can
  date is a shape nobody can trust, and a list quietly replaced would hide
  the very thing the operator needs to know.

### Fixed — the kept lists cannot be taken for the answers
- **THE OTHER CLIENT READS THIS CACHE DIRECTORY TO FIND WHAT THE MONITOR
  KNOWS**, and it takes the FIRST file whose name holds the host — so a
  tool list kept beside the answer, named for the same host, would have
  been taken for the answer itself, and the parity check would have read a
  tool list where it expects the estate's own answers. The lists live in
  the cache directory's own `catalogue/` directory now: out of that
  reader's way, and the directory still holds one answer per host and
  nothing else.

### Measured, on the live estate (2026-10-21)
- **THE LIST, A GATHER:** `tools/list` costs **65,467 B** on `.33`,
  **51,716 B** on `.90` and **32,724 B** on `.158` — **149,907 B** a gather
  across the estate.
- **THE PASS, BEFORE AND AFTER**, measured on the shipped binary with a
  counting shim in front of `ssh` (`tools/measure_poll.py`): the whole
  poll cost **298,728 B a gather** (66.0 MB a day) with the released
  client, and **148,817 B a gather** (32.9 MB a day) with the kept list —
  and the kept list is pulled an hour, not a gather (**3.6 MB a day**). So
  the poll's inbound bytes fall from **66.0 MB a day to 36.5 MB a day**,
  and the catalogue's own share from **33.1 MB a day to 3.6 MB a day**.
- **THE FALL IS THE CATALOGUE, TO THE BYTE**: 298,728 − 148,817 =
  **149,911 B** a gather, against the **149,907 B** the three `tools/list`
  answers cost on their own — and the warm pass's per-answer table has no
  `tools/list` line in it at all. The discover answer (667–1,711 B) is
  still read every session, because that is what says whether the kept
  list is still the truth.
- **A RELEASE STILL RE-LISTS**: every kept list was given another binary's
  version, and the next pass asked every host for its list again — the
  whole poll's bytes back to the cold figure (**298,726 B**) — and the
  client said why, in its own answer (*"Relisted — the kept list was for
  1.99.19-before-a-release, the server is 1.99.19"*) and on the panel
  (*"tool list: re-listed, the kept list was for 1.99.19-before-a-release,
  the server is 1.99.19"*).

### Reported, not written — the better place may be the server
- **THE CATALOGUE IS A PER-BINARY CONSTANT, AND THE SERVER KNOWS IT.** The
  estate's plan says the better place for this may be the server
  (`media_server`'s lane), and it is: the tool table is fixed in the
  binary, so the server could publish the list's DIGEST beside the absent
  map it already publishes in `server/discover` — a few dozen bytes — and
  then any client could check its copy against the server's own word
  instead of inferring it from the version and the absent set. That is
  reported to the estate's plan; **nothing was written in that repo.**

### Verified
- `tools/gate.sh --full` green; the Rust suite green.
- The kept lists are the client's own copy and nothing else: the cache
  directory still holds one answer per host, and the tool lists live in
  `catalogue/`, where the reader that looks for the answers does not look.

## v0.62.0 — the rate is READ from the estate, and the Ops frame can be PRINTED _(2026-10-21)_

### Changed — the client's sampling rate (servers ROADMAP E53)
- **THE RATE WAS A NUMBER COPIED INTO THIS CLIENT.** `MAX_SAMPLING_INTERVAL_S`
  (450 s) was half of a fifteen-minute publication, written down here, and
  the due rule and the staleness threshold were written against it. The
  estate's cron can move — and if it moved to every five minutes, this
  client would have kept sampling every 7.5 minutes and **neither** of its
  own staleness thresholds would have noticed. The server now publishes its
  publication cadence with the verdict (`health://latest`, beside the age:
  `cadence_s` and `cadence`), and this client **derives** its rate from
  what it read: half of the cadence the server published.
- **THE WARNING IS DRIVEN BY THE PUBLISHED CADENCE**, not by the local
  copy: a client that samples slower than half the cadence the server
  published says so where the verdict is shown — *"sampled every 15 min,
  server publishes every 5 min: two publications behind"*. The age on the
  line cannot say it: a verdict two publications behind looks exactly like
  a fresh one.
- **A CADENCE THE SERVER COULD NOT READ IS NOT A WARNING.** It is a gap in
  the reading, not a fault: the client says the rate could not be read
  (*"the cadence could not be read: this client keeps its rate (every 15
  min)"*) and keeps the rate it had.
- **THE STALENESS THRESHOLD FOLLOWS THE CADENCE TOO** — the published
  cadence plus a grace for a suite that is still running — so a verdict is
  stale against the estate's own publication, not against a number typed
  here.
- **THE CACHE CARRIES THE RULE AND THE TRUTH.** A kept answer holds the
  rate the client KEEPS (the rule) and the rate the answer was TAKEN AT
  (the rule, raised to the gap when the client missed its schedule). A
  missed cycle is not a new rate: a client that missed one must not keep
  sampling at half the rate for ever.

### Added — the Ops frame can be printed (servers ROADMAP O90)
- **`multitop --ops-frame <HOST>`** prints the Ops frame for one host,
  uncoloured: the same lines the Ops view draws, from this client's own
  answers (the cache the poll writes), and it asks the estate for nothing.
  The host is named by the name in its `[[servers]]` entry or by its
  address. The other client's parity check (`routines parity`) reads it
  through `--monitor-cmd`, so the two renderings can be compared without
  reading a terminal — the monitor draws its Ops view in its TUI and keeps
  no copy of it, so the check had to read the monitor's screen and say out
  loud that the rendered half was a stand-in.

### Fixed — a machine that declares nothing is not a fault (servers ROADMAP E26)
- **THE `vpn` LINE READ A HEALTHY MACHINE AS A FAULT.** The server's own
  reason for not offering `net.egress` reads *"the machine's instance
  declaration is missing or unreadable (…): its instances are unknown"*,
  and the monitor repeated it. E26's rule is that a machine which declares
  nothing has one instance — itself — and says so; the routines card says
  *"1 stack: adguard_server · no declaration"*. The monitor now says what
  it knows, in the card's words, and a declaration that EXISTS and could
  not be read still keeps the server's own reason — that one IS a fault.

### Measured, on the live estate (2026-10-21)
- **THE CADENCE IS PUBLISHED, on all three hosts:** `cadence_s: 900`,
  `cadence: "every 15 min"` (`.33`, `.90` and `.158`, read over ssh with
  `resources/read health://latest`). The client's rate is therefore
  **450 s**, and the answers say so: `rule_every_s: 450`.
- **THE GATHERS A DAY: 221** per host — the launchd agent's tick is 390 s
  and every tick finds a gather due (measured over six ticks:
  **390–391 s** between them), against the rule's own floor of 192
  (450 s).
- **THE BYTES A DAY: 60.3 MB** across the estate, from **153,111 B**
  (`.33`), **77,273 B** (`.90`) and **42,341 B** (`.158`) per gather —
  measured with `python3 tools/measure_gather.py <label> <ssh-target>
  <mcp-command>`, which asks the host's own `mcp_host` over ssh for exactly
  the requests the gather makes and counts the bytes each reply crossed the
  wire for.
- **THE FRAME FOR A HOST WITH NO DECLARATION** (`.158`), from
  `multitop --ops-frame pihole`:
  `· vpn        no tunnel on this machine: adguard_server, no declaration`
  — where it read *"no Docker on this machine: no daemon socket; the
  machine's instance declaration is missing or unreadable
  (/home/ztomer/prj/adguard_server/config/instances.tsv): its instances are
  unknown"*. The machine's own facts are on the line above it
  (`· stacks     adguard_server: 1 instance, no declaration`), and the
  containers section still says whether the machine has Docker at all.

### Verified
- `tools/gate.sh --full` green (14/14); the Rust suite (1,794 tests across
  the workspace) and the pytest suite as the gate of record ran them.
- The other client's parity check now reads the monitor's RENDERED frame
  (`routines parity --monitor-cmd "multitop --ops-frame"`): **23
  differences across 3 hosts**, every one named, and the note that the
  rendered half was a stand-in is gone.

## v0.61.1 — the poll's rate lands INSIDE the rule, whatever the scheduler does _(2026-10-21)_

### Fixed — the poll's schedule
- **launchd's timer wheel has a granularity of its own.** Measured on this
  Mac on 2026-10-21: a 420 s `StartInterval` fired **425 to 470 s** after
  the last run, so the client's read rate was sometimes 7 minutes 50
  seconds — outside the rule's 7.5, and the card would have said so on the
  days it drifted. The agent's tick is now **390 s**, which puts every
  gather between 6.5 and 7.5 minutes: inside the rule every time.
- **THE DUE RULE FOLLOWS THE SCHEDULER.** The age a host must reach to be
  gathered is now 90 s under the poll's interval, so a tick that lands early
  still finds a gather DUE: the client's read rate cannot halve because a
  second was lost.
- **AN ANSWER SAYS WHEN THE CLIENT MISSED ITS SCHEDULE.** The rate an
  answer carries is the poll's own — unless the gap since the last answer
  is LONGER, in which case the gap is the truth. A client that slept
  through a cycle says so, rather than carrying a rate it did not keep.

### Measured, on the live estate (2026-10-21, after the fix)
- **The gathers:** every **390–450 s** on `.33` (measured: 406 s between
  two gathers), against the retired rate's 30 s.
- **The answer carries 450 s**, so the aliasing warning stays silent
  because the client IS inside the rule — and it fires the moment the
  client is not.

### Verified
- `tools/gate.sh --full` green (14/14); the Rust suite (1,773 tests across
  the workspace) and the pytest suite as the gate of record ran them.

## v0.61.0 — the Ops poll is a JOB: it runs whether or not anyone is watching _(2026-10-21)_

### Changed — the Ops poll (servers ROADMAP E36)
- **THE POLL NEVER RAN.** It was spawned by the Ops VIEW's keypress and by
  nothing else, so the estate's fingerprint tools fired **0.3 a day** while
  the shared tools fired 87: the Ops view was shipped, installed, and nothing
  asked the estate for it. The code said so out loud — *"Polled every 30 s
  while the view is up, and never while it is not"* — so this was a design,
  not a bug, and the design was the defect.
- **THE POLL IS A JOB NOW.** `multitop --ops-poll` is one pass: it gathers
  the hosts whose answer is **due** and keeps what it found beside the
  config. `tools/install_ops_poller.sh` installs the launchd agent that runs
  that pass every 7.5 minutes. A pass that exits cannot be left half-dead;
  the next pass opens a fresh session.
- **THE RATE IS THE RULING'S.** DECISIONS D15 (owner, 2026-10-20): the
  estate publishes every 15 minutes, so a client reads at least every 7.5
  minutes. The 30 s rate is retired — it gathered **2,880 times a day**, ten
  times what the rule needs, and every gather moved the whole tool
  catalogue off the host. The interval is now DERIVED from the rule
  (`OPS_POLL = MAX_SAMPLING_INTERVAL_S`), so the two cannot drift apart.
- **ONE CARD, ONE READ OF THE ESTATE.** The answer a pass found is kept, and
  a host whose answer is younger than the interval is not asked again — by
  the pass, and not by the view either. The two pollers share one due rule,
  so the client's second reader costs the estate one read and not two.
- **THE ANSWER CARRIES THE CLIENT'S OWN RATE.** A client that samples
  slower than half the publication interval says so where the verdict is
  shown, on a line of its own (appended to the summary it was cut off by the
  panel's width, which left the meaning off the end of the sentence).
- **THE CACHE KEEPS THE SERVER'S OWN WORDS.** The raw answers are kept, and
  the client parses them when it draws: an answer read back from the cache
  is parsed by exactly the code that parses a live answer.

### Measured, on the live estate (2026-10-21)
- **One gather, in bytes the host sends:** `.33` **128,455 B** (of which the
  tool catalogue is 63,749 B), `.90` **75,574 B**, `.158` **40,988 B**.
- **A day at the rule's rate** (192 gathers a host): `.33` **24.7 MB**,
  `.90` **14.5 MB**, `.158` **7.9 MB** — **47.1 MB a day** for the estate,
  against **706 MB a day** at the retired 30 s rate: a fifteenth.
- **The client:** one pass over all four hosts is **0.55 s** of wall time, so
  **1.8 min a day** — against **26 min a day** at the 30 s rate.
- **The estate's own time** for one gather on `.33` is **15 ms** of the
  server's own measurement (the audit log's `ms` column), so 2.9 s a day at
  the rule's rate.

### Verified
- `tools/gate.sh --full` green (14/14); the Rust suite (**1,772 tests**
  across the workspace), the pytest suite and the coverage gate as the gate
  of record ran them.
- **Each new behaviour was proven RED first**: the interval, the warning on
  the line, the due rule, the pass that gathers what is due, and the view
  that shows the answer it already has each fail with an assertion when the
  behaviour is taken away.

## v0.60.0 — the Ops view samples at TWICE the estate's publication rate _(2026-10-20)_
### Changed — the Ops view's health section (DECISIONS D15)
- **The owner's ruling, 2026-10-20:** *"don't we need to double the data
  polls at twice of routine's polls to keep with the sampling lemma?"*
  Yes. The estate publishes its health verdict every 15 minutes, and a
  client that samples at the PUBLICATION rate can ALIAS: its read can
  land just before a publication and then wait a whole cycle, so its
  verdict is two publications behind the truth.
- **THE OPS POLL IS NOT CHANGED.** The owner has not ruled on this
  client's interval, and E36 (the poll is not running) is not this
  release's to reopen. The Ops poll already samples every 30 s — far
  inside the rule — and the health section now carries the rule itself
  (`ops::MAX_SAMPLING_INTERVAL_S`), so a client whose interval was ever
  set too slow says so where the verdict is shown.
- **A client that samples TOO SLOWLY says so, in its answer** — not in
  silence. The age on the line cannot say it: a verdict that is two
  publications behind looks exactly like one that is fresh.

### Verified
- `tools/gate.sh --full` 14/14 green; the Rust suite, the pytest suite and
  the coverage gate as the gate of record ran them.
- **The Ops view still never runs the suite** — the same test as before.
- **Each new behaviour was proven RED first**: the sampling rule and the
  warning it carries each fail with an assertion when the behaviour is
  taken away.

## v0.59.0 — the Ops view's health verdict is the PUBLISHED one, and it is DATED _(2026-10-20)_

### Changed — the Ops view's health section (servers ROADMAP E12, the client half)
- **The verdict is READ, and it is the estate's own.** The Ops view already
  read `health://latest` and never ran the suite; what it did with the
  answer was to count the checks. It now carries the whole verdict: the
  machine's own freshness (`oldest_age`, in the SERVER's words — `13 min
  ago`), each check's own age, and the suite the verdict belongs to.
- **THE AGE IS ON THE LINE, and that is the point.** The clients read the
  PUBLISHED verdict instead of re-running the server's 50-check suite, so
  what the section shows is as old as the estate's last cron run. A reader
  that cannot date a verdict cannot tell a 3-minute-old answer from a
  29-minute-old one — and those are different answers about the same
  machine. So the age is on the line, in the server's own words (one
  formatter dates a verdict everywhere it is shown), and a verdict past the
  estate's interval says so and is NOT green.
- **THE VERDICT IS THE MACHINE'S PERIODIC SUITE.** The published table
  carries the DAILY checks' verdicts too, and those are a day old by
  design, so a count that included them would be paired with an age that
  does not describe them. The suite is the server's own answer
  (`health.list`), and the client asks: it never guesses which checks count.
- **A machine that published NO verdict says so** — `⚠ health no verdict
  published: nothing has run on this machine` — and it is never an
  all-clear: "could not ask is never fine", and an empty table is not a
  green tick.

### Verified
- `tools/gate.sh --full` 14/14 green; the Rust suite, the pytest suite and
  the coverage gate as the gate of record ran them.
- **The Ops view still never runs the suite.** A test drives `ops::gather`
  against a fake `mcp_host` and asserts that no request named
  `health.run_all` or `health.run` was made: the verdict is read, never
  run.
- **Each new behaviour was proven RED first** — the age on the line, the
  stale marking, the absent verdict and the narrowing to the suite each
  fail with an assertion when the behaviour is taken away.

## v0.58.1 — a status is an enum, and the frame's glyph is the renderer's _(2026-10-20)_

### Changed — the Ops view's models (servers ROADMAP E48)
- **A status travels as a STATUS, not as a word that can lie.** The owner,
  2026-10-20: *"V is a visual representation of a status. we should use
  enums, not utf symbols. it shouldn't be something we transport around."*
  The Ops view's models held their statuses as `String`s, and a string is
  compared as TEXT: `"ONLINE" == "ONLINE"` says a pool is whole, and a pool
  whose member is `REMOVED` says `ONLINE`.
- **OURS are enums, and a word we do not have is REFUSED.** The egress
  verdict (`confirmed` / `leaked` / `disagreement` / `unverified` /
  `unknown`) and the estate's verdict about a pool (`whole` / `not whole`)
  are the estate's OWN vocabularies — enums in the code, named words on the
  wire, and a word outside the set is a FAULT: the section is unreadable
  rather than read as some verdict it is not. The enum grows; the string
  does not.
- **A verdict's severity was a NUMBER wearing a status's clothes.** The
  bands were a `u8` compared with `max_by_key`, so "how bad is the worst
  verdict on this machine" was answered by arithmetic on a number rather
  than by asking a status. `Severity` is an enum now (`Ok` / `Unsettled` /
  `Bad`), ordered the same way, so the question is answered by asking it.
- **THEIRS stay open and are quoted VERBATIM.** ZFS's `ONLINE` /
  `DEGRADED` / `FAULTED` / `OFFLINE` / `REMOVED` / `UNAVAIL` / `SUSPENDED`
  and Docker's `running` / `exited` / `paused` / `created` / `restarting` /
  `removing` / `dead` and `healthy` / `unhealthy` / `starting` / `none`
  belong to OTHER programs, so they are parsed into enums AT THE EDGE —
  where the drawing happens — and each keeps an `Other(String)` arm: a
  closed list would make the answer fail its own schema the day ZFS adds a
  word, and the frame would show something the server never published.
- **THE FRAME'S GLYPH was already the renderer's, and still is.** `Tone` was
  the picture of a state, and the renderer chose it. This release moves the
  STATE out of the model and leaves the picture where it was — the same
  picture, drawn the same way.

### Verified
- `tools/gate.sh --full` 14/14 green; 1743 Rust tests (0 failed, 21
  ignored), 64 pytest tests and 40 subtests — the LIVE suite, against the
  real hosts — 0 failed; 96.34% of coverable lines (16070/16681).
- **The rendered output did not change.** Every frame line the suites
  assert — `✓ vpn        1 tunnel confirmed`, `✓ zfs        2 pools whole:
  pool0, pool1`, `⚠ stacks     …` — was authored against the OLD code and
  passes unchanged now; only the fixtures that BUILD the model changed,
  from strings to enum constructors. Three assertions were ADDED, for the
  cases the enums make explicit.
- **On the live estate, before and after** — the Ops view against `.33`,
  `.90` and `.158`, through the crate's own `ops::gather` and
  `ops::render::render`, the same tree built at `6bd8d73^` and at
  `6bd8d73`: **the three frames are BYTE-IDENTICAL**, every glyph with it.
- **The two clients' vocabularies are the same WORDS** —
  `scripts/dev/check_client_vocabularies.py` in `media_server` compares the
  server's, this client's and the routines client's, and it is green: a fact
  cannot come back as `"ONLINE"` to one client and `"online"` to the other.

## v0.58.0 — the Ops view draws the machine's tunnel, its capacity and its stacks _(2026-10-20)_

### Added — the machine model in the Ops view (servers ROADMAP O92, the client half)
- **The machine's tunnel, with the stack that declares it.** `net.egress` is
  the estate's most consequential tool — where the tunnel exits, whether it
  LEAKED, and which stack owns it — and no client rendered it at all: the
  2026-10-19 incident was a tunnel in Zurich that every view called "fine",
  because no view drew one. The Ops view draws it now: the tunnel's
  container, the country it exits, the verdict, and the stack that declares
  it (`solver-vpn CH confirmed via solver`). The answer is a list, so a
  machine with two tunnels says so rather than hiding one, and a machine with
  none says so with the SERVER's own reason — never a green tick.
- **The machine's capacity.** The Ops view had the pools' state and no disk,
  no memory and no cpu: on the storage machine it could say "1 pool whole"
  and never how full the pool was. It draws the machine's free disk and the
  share used now — over the filesystems that count (each ZFS pool once, its
  datasets share its space), in the card's own arithmetic, so the two clients
  say the same number — and the machine's CPUs, its load and its memory, flat
  and unlabelled above the sections, the way the card keeps them.
- **The containers, one row per stack.** A machine may run several roots
  (`.90` runs the storage root and the solver root), so the containers are
  grouped by the stack they belong to, each row with its count, and the
  containers in trouble named under their stack. The server resolves the
  instance on every row; the compose project label is the fallback.
- **The machine's name.** `[[servers]]` takes an optional `name`, and the
  banner says it beside the address (`storage (192.168.0.90)`). A card
  titled by a ROLE ("storage") that now carries the solver's cron announces
  the wrong owner.
- **One set of words with the other client.** The Ops view's sections are
  the estate's — the same ones the routines card uses — and no longer say
  `zfs.health` and `docker` where the card says `zfs` and `containers`. The
  pools' whole case says "2 pools whole: pool0, pool1" and not "ONLINE",
  which is the word that lies when a member is REMOVED.

### Fixed — a stated absence is not an alarm
- A machine with no instance declaration is not a fault. `.33` — healthy,
  one instance, no `config/instances.tsv` — read `⚠ stacks media_server: the
  machine's instance declaration is missing`. A machine with no declaration
  has one instance (itself) and says so; that is a stated absence, and alarm
  is reserved for faults. It reads `· stacks media_server: 1 instance, no
  declaration` now. A declaration that EXISTS and could not be read, and a
  declared root that is not there, are still warnings — the two are told
  apart by the facts the server publishes, not by a flag.

## v0.57.0 — the Ops view reads zfs.health, and names it where the host does not _(2026-10-20)_

### Added — the Ops view's fifth section (servers ROADMAP O82)
- **The Ops view reads `zfs.health`**: a pool's name, its state, and the
  member that is not ONLINE — from the tool's own structured answer, never
  from `zpool status`. A pool that is not whole is drawn with the fault and
  what ZFS says to do about it (`✗ zfs.health 1 of 1 not whole`), so the
  2026-10-19 SUSPENDED pool on `.90` — the one thing every other view of
  that host called "fine" — is visible to the client that was meant to show
  it.
- **A host that does not serve the tool says why**, in the server's own
  words (`· zfs.health no ZFS on this host: no /usr/sbin/zpool` on `.158`),
  never a guess and never a green "none".
- **The section is named after the tool that answers it**, so the panel says
  whose answer it is showing.

### Fixed — the client and the local agent
- The client's comment claimed `mcp_host` published no absent reasons. It
  has, since its `dev.multitop/absent` meta landed; the comment now says
  what is true.
- The local agent names `zfs.health` in its absent map too, so this Mac's
  own panel says why the pools are not there instead of going quiet.

### Changed
- **The Android Gradle wrapper moves to 9.8.2** — the release the lint
  asked for (9.8.1 is refused by `AndroidGradlePluginVersion`). It was red
  on `main` before this change.

## v0.56.0 — the phone can pair, and the installer replaces the binary instead of rewriting it _(2026-10-19)_

Minor bump: the Android companion's pairing screen with its Keystore-sealed
store, and an installer that puts the new binary in place by rename.

**Upgrading:** the Android companion is new in this release, and it is the first
slice of it that talks to a server. Nothing on the desktop changes for you until
the read-only screens land (A2.5).

 — pairing in the Android app (ROADMAP A2.4)

- **The app pairs: scan the QR, or paste the string.** One Compose screen over
  the A2.1 pairing core, four honest states, each carried by a glyph and words
  and never by colour alone: `· Not paired`, `→ Pairing`, `✓ Paired to
  <host>:<port>` (with the certificate's `sha256=` for a human to compare
  against the server's line), `✗ Pairing failed: <why>`. CameraX 1.6.2 frames
  into ZXing core 3.5.4 with `ALSO_INVERTED`, so the light-on-dark code a dark
  terminal draws decodes as well as the dark-on-light one; the string can be
  pasted instead. Both go through `PairingPayload.parse`, so a wrong fingerprint
  is refused with its reason.
- **The pairing string is sealed under one Keystore key.** The canonical string
  (`PairingPayload.format`, byte for byte what the server prints) is sealed
  AES-256-GCM under a single Android Keystore key (alias
  `multitop-pairing-v1`), in one file in `noBackupFilesDir`; nothing in the
  clear, re-validated on every read, and an unreadable one (a lost key, a
  flipped bit) is reported and forgotten. Unpair deletes it. Proof:
  `QrDecoderTest` paints the QR the server really draws as a dark and a light
  terminal and decodes both, and calibrates that a stock reader finds nothing in
  the dark one; `PairingStoreTest` and `PairingControllerTest` hold the round
  trip, the absence of plaintext, unpair clearing, and a wrong key and a
  tampered file being unreadable. Not proven on a device: no phone or emulator
  is attached here, so the live camera path and the Keystore key are exercised
  only by building, not by a test.

 — the installer replaces the binary, it does not rewrite it

**Upgrading:** `./install.sh` now replaces the installed binary by rename instead
of rewriting it in place, so an install is a new inode every time. The old path
could leave a binary that macOS had already cached a signature for and then
SIGKILLed on every exec (exit 137); re-running the installer fixes that.

- **The installed binary is replaced by rename, never rewritten in place.** macOS
  caches a binary's code signature per vnode, and `cp -f` rewrote the executed
  file in place: the cache went on describing the old bytes and every later exec
  of that path was SIGKILLed (exit 137) -- the installed multitop was dead that
  way on 2026-10-19 while a copy of the same bytes ran. The installer now copies
  beside the target, signs the copy, and renames it over (a new inode, atomic),
  then runs the result's `--version`. Proof: `tests/test_install_replace.py`
  holds that a reinstall is a new inode that runs and leaves nothing staged (the
  old step kept the inode: 1173678144 -> same).

**Upgrading:** a `--serve` bound to anything but loopback now speaks **TLS only**,
with a self-signed certificate it creates once beside the token (`serve-tls.pem`,
`0600`) and reuses. Plain-HTTP clients of a LAN bind stop working; loopback binds
are unchanged.

 — `--serve` TLS, the server half of the Android companion's pinning (ROADMAP A2.3)

- **A LAN bind is TLS, a loopback bind stays plain HTTP.** On a LAN the bearer
  token and every response crossed the wire in the clear. The certificate is an
  ECDSA P-256 leaf (`CA:FALSE`), kept so the phone pairs once; a file others can
  open, or one whose key does not match its certificate, refuses to start rather
  than being replaced. Loopback keeps plain HTTP: the page at `/` and same-host
  `curl` are the one plaintext path, unobservable from another host. A plaintext
  request to the TLS port gets no HTTP answer. Handshakes run per connection,
  bounded and timed out, so a silent peer delays nobody.
- **The pairing string, printed to a terminal.** `multitop://pair?v=1&addr=...`
  exactly as the app parses it, with its QR code; the server reads its own
  string back by the app's rules before printing it, and says why when it cannot
  (a wildcard bind with no route, a hand-written token). A log gets the
  certificate's fingerprint, never the string, which carries the token. The
  `curl` line pins the key (`--pinnedpubkey`). A golden pair in
  `android/app/src/test/resources/` is written by the Rust tests and read by the
  Kotlin ones, so neither half can drift alone.

**Upgrading:** `--serve-token <TOKEN>` is refused. Put the token in a file only you
can read and pass `--serve-token-file <PATH>`, or pass nothing: `--serve` creates
`serve-token` beside the config file (`0600`) on first start and reads it after.

 — `--serve` before a second client exists to lose the token (ROADMAP A1)

- **The token lives in one owner-only file, not on argv and not in a log.** It
  reached three readers besides its owner: every local user via `ps` (it was a
  command-line argument), any service manager's log of stderr (`token=<t>` and a
  ready `curl` line on every start), and nobody at all after a restart, because a
  fresh one was made each time and the phone had to re-pair. A token file anyone
  else can open is refused with the `chmod 600` that fixes it; a hand-written token
  under 32 characters is refused. The token is printed only when stderr is a
  terminal; otherwise the banner names the file.
- **Authentication is a layer, default-deny.** Every path but the static index
  needs the token, unmatched ones included, so a new route cannot be added
  unauthenticated by forgetting a call, and an unauthenticated caller cannot map
  the routes. A 401 is one fixed body that echoes nothing the caller sent.

## v0.55.0 — the bench measures the code instead of the neighbours, and `render` stops rendering every frame twice _(2026-10-14)_

Minor bump: a real performance fix in the render path, a measurement that stopped
measuring the machine's load, and thirteen new gate floors a consumer can now tune.

**Upgrading:** `render` no longer renders every frame twice, which is a large win on
its own. The bench's budgets moved with it and are ~1.7x the new measurement, so a
deliberate 2x slowdown inside `render` is past both.

 — Phase 0 fallout (goh 0.27.0)

- **The latency bench measures the code, not the machine's load** (P0.7). It
  gated wall-clock frame render under 50,000 ns, which on this box is a gate on
  whoever else is running: the pre-push run at load ~35-65 read 54,976 ns on all
  three tries while the same tree alone read 33,919 ns fifteen minutes earlier.
  "Best of three" lowers the odds and does not change what is measured. The bench
  now runs ONE loop (`client_bench --only=decode|render --iters=N`) under
  `/usr/bin/time -l` and reports instructions retired, taken at N and at 2N so
  process start-up and set-up cancel. Across a 4x synthetic load the per-frame
  count moved 0.07%; wall time moved by tens of percent. Budgets are ~1.7x the
  measurement (decode 23,064 and render 566,614 instructions), and a deliberate
  2x slowdown inside `render` is past both.

- **`render` no longer renders every frame twice.** Found by the new bench gate,
  not by reading the diff: it went red at 1,134,630 instructions, which is the
  same code at almost exactly 2x. A `black_box(render_once(...))` throwaway had
  been left in front of the real call, so every production frame did the work
  twice. The gate is what made it visible and what pins it now.

- **Every scanner declares its floor** (P0.6). Thirteen `GOH_*_MIN_*` keys in
  `.gatesrc`, each at its measured read count, so a full-scope scan that stops
  reading its subject is red rather than clean. `GOH_SUBPROCESS_STDIN_MIN_CALLS`
  one past its count was proved to fail naming the key.

- **Every child process names its stdin** (P0.4): 39 calls across `tools/`,
  `tests/` and `scripts/` declared as `stdin=subprocess.DEVNULL` (a tmux or exec
  e2e child that must read the caller's terminal says so), with the floor at 42.

- **The gate's lock helper runs on goh's pinned interpreter** (P0.3), not bare
  `python3`.


## v0.54.0 — the local machine has an Ops view, and no section claims all is well over nothing _(2026-10-11)_

- **`multitop-agent mcp`: this machine's Ops view.** A local panel used to draw
  "no mcp_host command". The agent now serves MCP on stdio and lists only what
  the machine can answer -- `cron.status` from your launchd agents,
  `docker.containers` from the docker context's daemon (Colima is found) -- and
  says why each other section is absent. Register it with
  `claude mcp add --scope user mac -- multitop --agent mcp`.
- **An empty Ops section says so.** It used to read "✓ docker  0 running, all
  healthy", "✓ cron  0 jobs ok" and "✓ health  0 checks ok". Now: "no
  containers", "none running, N stopped", "no jobs", "no checks", on the quiet
  line. Re-check: a host you believed healthy because of a ✓ may have had nothing
  to judge.
- **The command bar is readable on dark terminals again** (broken v0.50.0 to
  v0.53.0: white on near-white, 1.02-1.73:1). Chrome ink is adapted against the
  surface it is drawn on, and the bar follows the theme.
- **apt's upgrade output no longer tears.** One escape reader (vte) serves the
  painter, the span renderer and `strip_ansi`; apt's pinned progress bar is a
  status line instead of text glued onto the next dpkg line.
- Gates: a failed commit gate prints the lines that name the failure; the e2e
  stale-binary guard reads cargo's dep-info; dependency currency is a ratchet
  (`tools/deps_ratchet.txt`, one entry: signal-hook 0.3, held by crossterm); the
  unchecked 1.85 `rust-version` is gone (the lock already needed 1.89), and
  `tools/check_msrv_claim.py` refuses a version claim no CI job builds.

## v0.53.0 — four majors moved, each proven where it touches the system _(2026-10-08)_

- **axum 0.7 → 0.8.** Route captures are `/{host}` now. The old test asserted a
  404, which an unmatched route also returns, so a broken route passed it; the
  new one asserts each handler's own error body (red with a route broken on
  purpose). The history route had no test at all.
- **dirs 5 → 7.** Both builds resolve the same path. The real vault is
  `~/.config/multitop/vault.bin`, derived from the config file and never from
  dirs, so no vault can move; a test pins the default path against the
  platform rule built from `$HOME`, not against dirs.
- **mach2 0.4 → 0.7.** It provides `mach_host_self` and `mach_port_deallocate`,
  so the agent's hand-written declarations are gone; a new test requires real
  CPU and memory figures from the macOS sampler, which nothing tested before.
- **zstd 0.13 → 0.14** (licence MIT → BSD-3-Clause, like zstd-safe/zstd-sys
  already were). 0.14 decoded all 9 history files the 0.13 build wrote.
- **signal-hook stays 0.3**: crossterm 0.29 (its newest) needs 0.3, and two
  versions in one graph make one type two types. `diag.rs` keeps its own
  thread so a dump works with the runtime stuck.
- `cargo update` in the workspace and `fuzz/`: 10 duplicate crate pairs gone.
- Hooks: the stock gates_of_heck delegates (O39); `check_codesign` builds the
  binary it checks instead of reading `target/`; CI checkouts no longer leave
  the job token in git config; early-exit consumers on a pipe read to
  completion.

## v0.52.0 — the gate is green, and it can now prove the binary is this build _(2026-10-05)_

**Nothing you use changes.** This is the currency and freshness pass: 115 test
assertions rewritten through one helper, a test leak fixed, three dependency
majors moved, and a new gate that fails when the installed `multitop` is not the
binary this source builds. Every vault on disk still opens -- there are tests
that pin the exact bytes now, which is the only reason that sentence is a
measurement rather than a hope.

### Fixed
- **A test could leak an `sshd`.** `ssh_tests.rs` reaped its child *after* a
  `write_all` that could panic first, so a failing write skipped the reap and
  left the upload script -- which starts an `sshd` and holds a port -- running on
  the machine. The reap is now a guard's `Drop`, which unwinding runs. It has its
  own test, because the house gate that caught the original cannot tell a guard
  from a wrapper (see `docs/roadmap.md`).
- **`painted_states` carried a `#[must_use]` that duplicated the return type's.**
  `Iterator` is already `#[must_use]`, so the attribute was a second copy of a
  property `impl DoubleEndedIterator` already had. Removed, not renamed.

### Changed
- **115 `assert!(x.is_empty())` sites are now one helper.** `clippy 1.99.0`
  (2026-10-01) added `assert_is_empty`, and the house added
  `check_no_empty_assert`, which also counts the message-carrying form clippy is
  silent about. Rather than 115 type ascriptions, `crates/testassert` is one
  `assert_empty!` / `assert_not_empty!` pair shared by all three crates. The
  failure line names the expression, which is what identifies which of a dozen
  assertions failed -- and it requires no trait of the value, because
  `mlock::LockedMemory` has no `Debug` on purpose (it holds a vault key) and
  `ratatui`'s `Modifier` has no `len()`.
- **`tower` 0.4 -> 0.5, `sha2` 0.10 -> 0.11 (with `hkdf` and `argon2`), `zbus`
  4 -> 5.** Each was pinned below a version the graph already resolved, so the
  tree carried two majors of one crate and the house gate calls that FATAL. No
  type crossed any of the three boundaries, and none does now. The lockfile's
  duplicated crates: **31 -> 16**, and every one of the sixteen is transitive --
  `secret-service 5.1.0` alone holds what is left of the 0.10 crypto generation.
- **`tools/check_doc_paths.py` no longer fails on a shallow clone.** CI checks
  out with `fetch-depth: 1`, so the checker could not see any deletion and
  reported six docs that correctly record a removal as broken. `main` had been
  red on CI since 2026-10-01 for that reason alone. It now refuses with the
  three commands that fix it, and CI fetches the history it needs.

### Added
- **`tools/installed_freshness.py`** — fails when the installed binary is not
  what this source builds, comparing **bytes**. The installed copy was 64,608
  bytes away from a fresh build of the same commit, with the same version string,
  because the stable toolchain was replaced after it was installed; no gate could
  see it, and every end-to-end suite was evidence about the wrong artifact. Runs
  on the pre-push list, because its subject is the machine's install.
- **Known-answer tests for the vault's key derivation.** Argon2id, the HKDF
  signing sub-key, the Ed25519 verifying key and SHA-256 are pinned to bytes
  computed from the *previous* dependency generation. Every other vault test is a
  roundtrip, which cannot detect a KDF change that makes existing vaults
  unopenable -- which is the failure a major bump actually risks.

## v0.51.0 — the remote MCP command is not a login shell _(2026-09-30)_

**If you configured an `mcp` command that needs a binary from `~/.local/bin`,
it may have been failing with `command not found`.** The docs, the example config
and the doc comment all said the remote runs the command "in the host's login
shell". It does not.

Measured on `.33`, with a positive and a negative control:

```
zsh -l            -> PATH carries /home/ztomer/.local/bin
ssh host '<cmd>'  -> ~/.local/bin absent
```

`mcp::spawn::open` passes the command as a single `ssh` argument, so the remote
runs `$SHELL -c <command>` — not interactive, not a login shell. `~/.zprofile`,
where pipx adds its directory, is never read. Corrected in all three places the
claim appeared (`config.example.toml`, `README.md`, and the `types.rs` doc
comment) rather than the one the failing run happened to hit.

Also: `.158` is not Pi-hole — it is AdGuard Home with unbound, no `pihole` CLI
and no `pihole-FTL`. Its hostname is still `pihole`, which is what keeps the
label alive. Nothing in this repo called it Pi-hole, so nothing changed here.

## v0.50.0 — every theme was illegible on a light terminal _(2026-09-28)_

**If you run multitop in a light-coloured terminal, your colours have changed**,
and the alert banner has changed most. Dark terminals are unaffected: the theme
values you had are the values you still have.

### Added
- **Background detection.** The client asks the terminal what colour it is
  (OSC 11), then `COLORFGBG`, then falls back to the theme's own keybar
  background, and adapts all eight palettes for what it finds.
  `multitop --print-background` reports which, and from which source.
- **`tools/light_terminal_check.py`** — runs the built binary in a pty and
  measures every colour it actually drew, on a light and a dark background, with
  a negative control that fails if the two arms draw identical palettes.

### Fixed
- **The alert banner was unreadable on a light terminal, and 1380 tests could not
  have told you.** The breach and warning branches hard-coded colour escapes.
  On a dark terminal those literals *are* the theme's values, so nothing looked
  wrong, and a literal does not pass through the adaptation layer, so nothing
  could fix it. Measured on the real app, the alert red came out at **3.14:1**
  on white — the only one of eight rendered colours under the 4.5:1 bar. Both
  branches now ask the palette.
- **The detected background reached nothing.** For one commit the client probed
  the terminal, stored the answer, adapted a palette for it, and then rendered
  with a view built *before* the probe ran. Everything was green and the app was
  unchanged. `tools/check_no_hard_coded_colour.py` now fails any colour literal
  in either render crate, and the wiring test drives `boot_app_with` and reads
  the `App`'s own view — a test that calls the rebuild itself would have proved
  the function works and not that anything calls it.
- **Four themes were illegible on their own background, not just on light.**
  Nord's gray, purple and red; Gruvbox's red; Monokai's red. Corrected, and the
  contrast table is in the detection record.
- **The OSC 11 probe accepted its own echo as a reply.** Terminal.app echoes what
  the app writes, so the "response" was the query. The reply is now distinguished
  from the echo.

## v0.49.1 — the release script and the docs could not be trusted _(2026-09-28)_

Housekeeping only. No behaviour in the shipped binary changed; the pane-width
fix, the mux-noise fix and everything in v0.49.0 are what this builds on. What
changed is that two of the things that certify the build were themselves wrong.

### Fixed
- **`RELEASE.md` step one named a script deleted on 2026-09-14.** "Run the gates
  first -- `python3 scripts/local-ci.py`" was correct the day it was written and
  wrong from the day after, with no signal: the file was eleven days past the
  script's death and every release in that window would have started with
  `No such file or directory`. Nothing in the repo could see it, because no gate
  looked. A doc that no longer applies is worse than no doc, because it is
  trusted.
- **`DEVELOPMENT.md` had the same defect, worse.** It told a reader to run
  `python3 scripts/release.py v0.23.0 --cut` in a copy-pasteable block --
  thirteen minor versions stale, and naming the second of two release tools.
- **A second release tool was still shipped, carrying the bug it was written to
  prevent.** `scripts/release.py` (429 lines) rewrote the Homebrew formula with
  `re.sub(r'sha256 "[^"]*"', ..., no count)`, which sets **every** `sha256` line
  in the file -- tarball and both agent binaries -- to the tarball's digest, and
  it had no concept of the agent resources at all. Used, it would produce a
  formula that installs and then fails every agent checksum. Deleted rather than
  fixed: the last release was cut with `release.sh`, this predates the change
  that made the formula carry agents, and the previous release tool
  (`local-ci.py`, retired 2026-09-14) was deleted for exactly the same reason --
  a second copy of a list, drifting.
- **`tools/check_no_emoji.py` no longer exists** and the development docs still
  named it. It moved to the shared house gates; the table now says so and where.

### Added
- **`tools/check_doc_paths.py`** -- a gate that fails when a doc names a path
  that does not exist. It found the two defects above on the real tree, which is
  the only reason to believe it works; the `--self-test` cases were all
  insufficient on their own, and calibrating against the real defect caught three
  design flaws in the checker itself (a code-fence-only scope that missed the
  prose the defect was actually in; a deletion exemption so permissive it
  blessed the very sentence it was written to catch; and a per-line marker test
  that could not see a marker three lines up a wrapped sentence).
  Scope is deliberately a floor: a path in a command, a "Key Files" label, or
  multi-segment prose in an instructing doc must exist, and a path git records
  as deleted is accepted **in prose only** -- naming a removed file in a command
  is wrong however true the removal is. The oracle is `git log
  --diff-filter=D`, not a hand-kept allowlist, so a new deletion needs nobody to
  remember the docs.
- **The roadmap is a backlog again.** It had accumulated a hand-over notes
  section that was 24 days stale and carried two wrong file:line pointers; those
  invariants live in `DEVELOPMENT.md`, which is where a developer looks. The
  mobile companion entry claimed greenfield while the serving half shipped
  (`--serve`, `--serve-token`, 404 lines of `/api/*`), and named a WebSocket
  transport for work that is an HTTP client.

### Removed
- **`scripts/release.py`** and **`scripts/clean_slskd_history.py`** (see above,
  and: the latter is a tool for a different project's slskd music server and has
  no referent in this repository at all).

## v0.49.0 — the upgrade stops being drawn 80 columns wide _(2026-09-27)_

### Fixed
- **The upgrade is rendered at the wrong width.** Every remote channel was
  handed the live pane size — Monitor, Docker, Fetch, Ops — and the two exec
  callers that draw into that same pane passed `cols: 80, rows: 24`. Measured
  on a live host over a multiplexed connection, `stty size` asked *inside* a run
  on a 199-column terminal answered `24 80`: `apt` laid out its progress bar
  and its "After this operation" table for eighty columns and the pane drew the
  result into two hundred. `journalctl -f` and `tail -F` were following a
  screen nobody was looking at. The pane's size is now a required argument of
  every exec spawner, so the next one cannot be added without deciding.
- **`ssh`'s own multiplexer chatter could be quoted as a diagnosis.** The rule
  that recognises it lived in the upgrade reader alone. A plain file at the
  `ControlPath` — measured on OpenSSH 10.3p1 — makes `ssh` print two lines
  about the socket and carry on unmultiplexed, and the other three readers of
  that pipe had never seen them: a monitor panel could be told
  `ControlSocket … already exists, disabling multiplexing` was the reason it
  went dark, those two lines could clear a genuine missing-agent report, and a
  failed upload quoted a full local path. One definition, beside the socket it
  describes, asked by all four.
- **The remote host's own verdict is no longer swallowed.** The old rule also
  matched `Connection to %s closed by remote host.` on a bare
  `contains("connection to") && contains("closed")` — the host telling us it
  hung up, which is exactly the diagnosis the rule exists to protect. The
  vocabulary is now built from the format strings in the `ssh` binary itself,
  so every phrase matched is one only `ssh` prints.
- **Three timing suites that fired under load, which is what the coverage gate
  caught.** A `sleep` standing in for "the loop has read the key" passes when
  the machine is quiet and fails when it is not — 6 of 20 warm runs on
  unmodified `main` for one of them — and one collector gave each message 5
  seconds and stopped collecting *silently* when the window closed, so a
  truncated list failed a later assertion about output that was never missing.
  Every wait is now on a condition: the key is re-sent until the loop's own
  durable record shows it acted, which is correct rather than a retry because
  the app defers a confirm until the credential lookup lands, so an early press
  is queued rather than lost.
- **`tools/lint_linux.sh` had never run here.** `cp -r` cannot stat git's
  fsmonitor socket through a bind mount, and the three commands shared one `&&`
  line, so the failure skipped only the `cd` and cargo then reported
  `could not find Cargo.toml in /tmp` — a message naming neither the cause nor
  the directory it was in. Now copies with `tar --exclude`, and says so when
  the copy produces no manifest. `✓ linux clippy clean` for the first time on
  a macOS checkout.
- **`brew upgrade` was broken by v0.49.0, for about ten minutes, and the
  release script is why.** The Homebrew transform paired a digest with a url by
  LINE ORDER — it armed `pending` on a url line and wrote the sha onto the next
  sha line. That works only while every url in the formula names the same tag,
  and two agent urls had been left at v0.47.3 by a release four versions
  earlier. So on v0.49.0 it matched nothing, changed nothing, and every
  assertion it had still passed: `pending` had simply never been armed. The
  formula shipped 0.49.0 digests beside v0.47.3 urls, and `brew fetch` refused
  it. The final check that should have caught it only looked for the *tarball's*
  old tag, so a different old tag walked straight past. Fixed in
  `tools/bump_formula.py`, which rewrites one resource block at a time by name
  with every count asserted, and requires every tag anywhere in the formula to
  be the tag being released. `tests/test_bump_formula.py` runs the v0.49.0
  input through both transforms: the old one ships the breakage silently, the
  new one repairs it.
- **Two constants claimed to be the resize debounce.** `consts::RESIZE_DEBOUNCE`
  said 250 ms; `run::RESIZE_DEBOUNCE`, which is the one the loop reads, says
  30 ms. Nothing referenced the first, and a reader asking "how long until a
  resize takes effect" in the constants file would have been wrong by 8×. Both
  dead copies removed with their duplicate `RECONNECT_BACKOFF`.

### Added
- `exec_window_test`: the child's own `stty size` has to report the pane it is
  drawn into, at a wide pane, a narrow one, and two at once.
- `ui::layout` pins the geometry for 1–9 panels: the published size is never
  larger than any pane it is drawn into, and for a single panel it is exactly
  the pane. Includes the one case where the render floor publishes more rows
  than the shortest pane has, which is a trade rather than an oversight and is
  now named as one.
- `check_magic_numbers.py` gained a `window` rule. The gate could not see this
  class at all: a struct field initialised to a literal is not a `let`
  binding, and none of its six rules matched one.

### Verified
- The upgrade's pty tracks the pane: 198×29, 118×39, 78×29, 58×29 on
  terminals of 200, 120, 80 and 60 columns, over a shared `ControlMaster`
  connection to a real host — against a hard `24 80` at every size.
- The multiplexer is not the cause of anything else here, and that was checked
  before anything was changed: cold, warm and deliberately-unmultiplexed
  transports deliver identical bytes on all three hosts (`test_exec_live.py`,
  9 tests / 36 subtests), and `ssh` writes nothing to stderr in normal
  operation. Recorded in `docs/detection-record.md` along with one
  measurement that was wrong and the reason it was.

## v0.48.1 — "unchecked", never read as unhealthy _(2026-09-23)_

### Fixed
- **The Ops view's containers line.** ".33: 44 running, 33 healthy" read as
  eleven unhealthy containers; none were - eleven declared no Docker health
  check. The line now reads "N running, all healthy", or warns "N running ·
  H healthy · U unchecked" (a container nothing checks is a gap). Unhealthy
  containers are still named, each on its own line.

### Changed
- **Hooks.** The commit hook runs coverage and the ratchet through the
  gates_of_heck proven-step cache, so the push gate skips a step already
  proven on the identical tree; `tools/gate_lock.py` works in a git worktree.

## v0.48.0 — the Ops view _(2026-09-23)_

### Added
- **`p`: the Ops view** (servers ROADMAP 12.17b). Per host, what its
  `mcp_host` says that the agent does not stream: the last verdict of each
  health check (`health://latest`, never a suite run), cron jobs, container
  health (`docker.containers`), and the alerts its cron wrapper logged in the
  last 24 hours. Each section says why it is absent (a host with no Docker) or
  unreadable, and one failing never hides the rest. A container outside any
  compose project that exited 0 is a finished one-shot, not trouble - the
  routines follower's rule. `/` searches check, job and container names,
  alert subjects and a failed session's reason.
- **A stateless MCP client** (`mcp::client`, protocol 2026-07-28):
  `server/discover` first, every request carrying the version in `_meta`, no
  handshake; an older server is refused with the versions it offers. Each
  request has a deadline, and after a timeout or a closed pipe the session is
  not trusted again. The server is started by the host's login shell over the
  panel's ssh connection (`mcp` in `[[servers]]`); nothing listens.
- The poll holds one session per host, reopens a broken one on the next poll,
  and shows why under the last answer it had. It runs only while its panel is
  in the Ops view (`Tasks::retire_ops` after every key).

### Changed
- **No source file is over 500 lines any more.** `handle_key.rs`, `diag.rs`,
  `state.rs`, `keybar.rs` and `server.rs` were grandfathered over the cap by
  the LOC ratchet; each is now split by responsibility (`run/commands.rs`,
  `run/confirm_keys.rs`, `run/palette.rs`, `ui/keybar_confirm.rs`) or has its
  tests in a sibling `*_tests.rs`, and their ceilings are gone from
  `tools/loc_baseline.txt`.
- The palette's view words go through `execute_cmds` like the keys; its
  Docker and Fetch branches were copies of it.
- The narrow keybar sheds keys by NAME: its index list, coupled to the row's
  order by a comment, overflowed at 8 columns the moment a key was added.
- `Mode` lives in `src/mode.rs`.

## v0.47.3 — no lint suppression, unsafe scope as a gate, keyring 4 _(2026-09-21)_

### Changed
- **Every `#[expect]` and `#[allow]` is gone from the workspace; the house
  gate (`gates_of_heck` `check_no_allow.py`, policy of 2026-09-20) now
  refuses both.** About 150 sites across the three crates, each fixed at the
  finding rather than annotated: lossy `as` casts became `try_from` +
  `unwrap_or`, or go through the new `agent::conv` helpers (`unsigned`,
  `count`, `signed`, `single`, `whole_*`) which convert exactly via
  `num-traits`; exact float asserts compare with `partial_cmp`; functions
  whose body differs per platform are defined once per `cfg` (a synchronous
  Secure Enclave path returns `std::future::ready`, the Linux path stays
  `async`), so neither `unused_async` nor `manual_async_fn` needs quieting;
  `cfg_attr(..., allow(unused_mut))` became a per-`cfg` `let`; oversized
  functions were split into named stages (`run/handle_key.rs`,
  `run/event_loop.rs`, `app/apply.rs`, `config/load.rs`, `config_ui.rs`,
  `exec/pump.rs`, `render_layout.rs`, `password_actions.rs`, `run/spawn.rs`);
  bool clusters became small enums and structs (`Overlay`,
  `CredentialLookup`, `QuitFlags`, `SudoSigns`, `Sections`, `Handshake`);
  `vault::initialize` and `rebind_biometric` are synchronous (they never
  awaited). Every integration-test and bench crate root carries
  `#![cfg(test)]`, so clippy's `allow-expect-in-tests` applies to them as
  it always did to unit tests; test helpers that could fail return `Result`.
- **`unsafe_code` is no longer a workspace lint; its scope is a checker.**
  The agent and the vault are FFI by nature and carried a per-module
  `#[expect(unsafe_code)]` under a workspace-wide deny. `multitop` (no FFI)
  keeps `#![deny(unsafe_code)]` at both crate roots; for the other two,
  `tools/check_unsafe_scope.py` lists the eleven files that may contain
  `unsafe` with the reason each needs it, fails on any other file, and fails
  on a listed file that no longer has any (a ratchet both ways). Wired into
  pre-commit and CI; `check_gate_parity` keeps the two in step.
- **The host-wide `~/.cache/cargo-target` is no longer a root anything
  looks in.** It was the shared target dir until the 2026-09-20 build-dir
  layout; `build.rs` still searched it for an agent to embed (after the
  checkout's own `target/`, before auto-compilation), the `./multitop`
  wrapper and the tmux harness would run a binary from it, and
  `check_agent_version.py` compared against one -- which is how the
  empty-scope sweep found the checker failing on an empty tree with a
  0.47.2 agent still parked there. Every site now resolves only this
  checkout's target dir (`cargo metadata`, `CARGO_TARGET_DIR`, `target/`).
- **`fuzz/Cargo.lock` is checked `--locked`.** The pre-commit fuzz check
  ran an unlocked `cargo check`, which rewrote the lockfile during the
  0.47.3 commit and left the tree dirty behind a green hook. A stale fuzz
  lockfile is now a red step naming its fix (proven red, then green).
- **The bench gate is best-of-three.** One sample under load (57 us for a
  28 us frame render, right after the fuzz ASan builds) refused a push;
  a latency gate measures what the code can do, and a real regression is
  slow every time.
- **The enclave rebind is macOS-only at its call site.** The pre-push
  container lint (`tools/lint_linux.sh`) caught the non-macOS stub as
  `unused_self`; the repair now takes the vault by value and the one call
  is `cfg`'d, so there is no stub and no `mut` a platform never uses.
- **`release.sh`'s tap bump ran for the first time since it was written
  and failed twice on its environment, not the formula.** The Python
  transform sat inside `python3 -c "..."`, so the `sha256 "..."` regexes
  lost their double quotes to bash and the agent-sha line was never
  matched; it is a quoted heredoc now. Then `ruby -c` rejected the
  formula's em dash as an invalid multibyte character because the tool
  shell had no locale; it runs under `LC_ALL=C.UTF-8`. The tag, release
  and assets had already landed, so the bump was finished by hand-running
  the fixed tail; the next release runs end to end.
- **Vault moves from `keyring` 3 to 4, matching `multitop`.**
  The workspace carried two majors of one crate: `multitop` on 4 since the
  August dependency refresh, `multitop-vault` still on 3 with the old
  `apple-native` / `sync-secret-service` feature flags (names that no longer
  exist upstream; the platform stores are built in and Linux secret-service
  selection is the default). The vault now declares
  `keyring = { version = "4", default-features = true }`, identical to
  `multitop`, and both lockfiles drop the second graph: `keyring` 3.6.3,
  `dbus` / `dbus-secret-service` / `libdbus-sys`, `security-framework`
  2.11.1 and `core-foundation` 0.9.4. The keychain calls in use
  (`Entry::new`, `get_password`, `set_password`) are identical across the
  bump and the vault file format is untouched — existing vaults open as-is.

### Fixed
- **Tests name this build's agent through a seam instead of guessing where
  it lives.** `spawn_exec` / `spawn_local_agent` used to look for a
  `multitop-agent` beside or above the running executable — a guess that
  held for exactly one build layout (`target/debug/deps/<test>` under
  `target/debug/<bin>`) and broke silently on 2026-09-20 when cargo's
  intermediates moved to a separate build-dir: ten integration-test crates
  spawned nothing (`NotFound`, reported as if the agent had not been built)
  and one of them parked forever, which is what a "10-minute commit" was.
  The resolver now takes `MULTITOP_AGENT_EXE`, then this process when it is
  a `multitop` build (`--agent`), then `multitop-agent` on `PATH`; the tests
  set the seam to `CARGO_BIN_EXE_multitop` (`tests/common`), which cargo
  hands every integration test in every layout. `local_agent_test`'s two
  tests are un-ignored — they had never run and had rotted past the `Hello`
  frame the stream opens with.
- **`test_concurrent_upgrade_generations_isolated` cannot hang.** It looped
  on the channel until both generations reported *success*, holding its own
  sender, so a failed upgrade left it waiting forever. It records every
  outcome and asserts on both (proven: fails in 0.00s when the agent cannot
  spawn).

## v0.47.2 — upgrade stderr through the painter, finish routing shared _(2026-09-13)_

### Fixed
- **Update screen showed progress text in duplicate.**
  `tasks/upgrade.rs` sent stdout through the `Painter` (append vs repaint by
  cursor movement) but buffered stderr per line in `Report::errbuf` and flushed
  it as plain appends at end of run. `apt` writes its progress display to
  stderr, so every tick appended one more near-identical line — and out of
  order, after all stdout. Stderr now feeds the same shared `Painter` (one
  cursor: the remote pty has only one), styled red and routed with `paint_msg`
  exactly like `exec_runner`. `keep_stderr`, `Report::errbuf` and
  `MAX_UPGRADE_ERR_LINES` are gone; the ring cap bounds memory. `sudo_help`
  detection, connection-noise filtering and blank-stderr dropping moved with
  the bytes, pinned by e2e tests that print progress to stderr.
- **An unterminated final line was appended twice on the exec path.**
  `exec_runner::drain_stdout` sent the painter's `finish()` as a bare
  `AuxLine`, ignoring `back`/`erase_below` — a tool killed mid-write left its
  last line on screen via the open-line paint and got a second copy at close.
  The tail now routes through `paint_msg` like every other paint, and
  `drain_stdout` is generic over the byte stream so the case is tested with
  synthetic framed packets instead of a live run.

## v0.47.1 — unbreak Linux CI, keep chunked markers, fix upgrade hangs & PTY sizing _(2026-09-09)_

### Fixed
- **Lingering agent processes and stuck upgrade view on remote hosts.**
  Commands like `us;ud` that launch Docker containers or subshells leave open
  descriptors (PTY slave, stderr) inherited across forks. The agent's `pump()`
  loop previously waited indefinitely for descriptor EOF without checking if
  the parent shell had exited. `pump()` now actively polls `try_wait(child.pid)`,
  drains queued bytes, closes descriptors, and emits `ExecFrame::Exit`
  promptly when the shell process completes.
- **Double lines on `update-local` and live-rendered tables.**
  `run_upgrade` passed `cols: 0, rows: 0` in `ExecFrame::Request`, which clamped
  to `ws_col: 1, ws_row: 1` in the PTY. Terminal tools like `rich.live.Live`
  detected a 1-column terminal and wrapped character-by-character, breaking
  ANSI cursor rewinds (`\x1b[1A`) and duplicating every row in the log. PTY
  dimensions now default to standard 80×24 when 0, and `run_upgrade` sets 80×24
  explicitly.
- **Interactive pager hangs and unframed reads during upgradable package check.**
  On Ubuntu 24.04/26.04, `apt list --upgradable` run inside a PTY spawned
  `/usr/bin/pager` (`less`), hanging forever waiting for input. The command wrapper
  now exports `PAGER=cat`, and `spawn_upgradable_check` sets `DEBIAN_FRONTEND=noninteractive`
  and properly decodes binary framed packets rather than attempting raw stdout reads.
- **Linux CI compiled nothing since the lint opt-in.** Opting the agent into
  the workspace lints tripped `unsafe_code = "deny"` on the pre-existing
  Linux-only `malloc_trim` block, which had never been named in the
  module-attribute policy — so clippy, test, coverage and fuzz all failed to
  build. `monitor` (and the already-attributed but unlisted `proc_disk`) join
  the FFI module list.
- **A marker split across two reads after a progress bar was lost.** The
  sieve's hold-back length budget measured the whole partial instead of the
  post-`\r` state, so `…\r__multitop_sudo_failed__` arriving in pieces was
  flushed as output. The budget now measures the carriage-return state, with
  deterministic `sieve_test.rs` regression tests.

## v0.47.0 — two advisories, a lint policy, and a crate that was linting nothing _(2026-09-07)_

### Security
- **RUSTSEC-2026-0253: use-after-free in `lru`'s `LruCache::pop()`.** Fixed by
  moving off the affected version. This was sitting in the lockfile.
- **The yanked `chacha20` 0.10.1 → 0.10.2.**

Both were found by wiring `cargo audit` as a gate rather than running it by
memory, and by configuring it to actually bite: `cargo audit` exits 0 on
`unmaintained` and `unsound` findings, so a gate without
`[output] deny = ["warnings", "unmaintained", "unsound", "yanked"]` passes over
precisely the class it was added to police.

### Fixed
- **`crates/agent` was subject to NO lint policy and nobody could tell.** The
  workspace had declared `pedantic` and `nursery` from the start; a member crate
  applies that with `[lints] workspace = true`, and this one — the biggest, the
  one uploaded to every monitored host — never said so. It had accumulated **254
  findings** that every green gate had agreed were absent. There is no warning
  for this anywhere: cargo does not mention the omission and `clippy -D warnings`
  passes, because the crate genuinely has no findings at the levels it is subject
  to. Now gated by `gates_of_heck/checks/check_lints_optin.py`.
- **`make clippy-targets`: every SHIPPED target is linted, not just this
  machine's.** Clippy reports on the cfg it compiled for and is silent about
  every other, so a crate that is half `cfg(target_os = ...)` had a Mac checking
  one half and the ubuntu runner checking the other — neither failing on the
  other's code, and nothing comparing them. That found ten findings in `src` the
  host cannot see, plus two `#[expect]`s that were UNFULFILLED for linux-musl,
  which under `-D warnings` is an ERROR: **the agent's build was broken for the
  platform it ships to while every gate was green.** A missing target std is now
  a hard failure naming the `rustup target add`, never a skip.
- **A 619-line test file was bounded by nothing.**
  `crates/multitop/tests/event_loop_e2e.rs` was named in `.gatesrc`'s
  `GOH_LINE_EXCLUDE` (so the 500-line cap did not apply) and absent from
  `tools/loc_baseline.txt` (whose sweep is `crates/*/src` only), so neither the
  cap nor the ratchet applied and it could grow without limit. Worse, the
  baseline's own header asserted the opposite, so the document read as though the
  hole did not exist. Split into a directory target; the exemption is gone; the
  pairing is now enforced by `check_exclusion_has_ceiling.py`, run from
  `tools/ratchet_check.py` so the hook, CI and local-ci all get it.

### Changed
- **`clippy::pedantic` + `nursery` adopted across the workspace**, in staged
  passes with the count recorded at each: the machine-applicable fixes, then the
  judgement calls, then the libc FFI casts in `sys.rs` scoped narrowly rather
  than blanket-allowed. `lib.rs` was split because the ratchet was right about it.
- **Two forbidden `#[allow]`s deleted** and the wire encoder tightened. `#[allow]`
  is banned here in favour of `#[expect]`, which errors when its lint stops
  firing and therefore cannot rot.

### Removed
- **Three unused dependencies**: `tower-http` (multitop), `rpassword` and
  `core-foundation` (multitop-vault). Each verified by REMOVAL and re-checked
  against all three shipped targets, not by grepping for the crate ident — that
  grep has exactly the blind spot that makes the tool wrong on `serde_bytes`,
  which is reached only through `#[serde(with = "...")]` and is now recorded as
  an ignore with its reason.

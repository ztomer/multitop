use crate::app::push_capped;
use crate::app::App;
use crate::panel::Mode;
use crate::types::Msg;
impl App {
    fn accepts(&self, panel: usize, gen: u64) -> bool {
        self.panels.get(panel).is_some_and(|p| p.gen == gen)
    }

    // One arm per message, and splitting a dispatch table into halves puts
    // the guard for one message in a different file from the message.
    pub fn apply(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Packet {
                panel,
                gen,
                epoch,
                payload,
                dims,
            } => self.on_packet(panel, gen, epoch, &payload, dims),
            Msg::Frame {
                panel,
                epoch,
                lines,
            } => self.on_frame(panel, epoch, lines),
            Msg::Status { panel, gen, text } => self.on_status(panel, gen, text),
            Msg::FetchData {
                panel,
                gen,
                snap,
                lines,
            } => self.on_fetch_data(panel, gen, snap, lines),
            Msg::AuxBegin { panel, gen, header } => self.on_aux_begin(panel, gen, header),
            Msg::AuxRepaint {
                panel,
                gen,
                line,
                back,
                erase_below,
            } => self.on_aux_repaint(panel, gen, line, back, erase_below),
            Msg::AuxLine { panel, gen, line } => self.on_aux_line(panel, gen, line),
            Msg::AuxDone {
                panel,
                gen,
                note,
                success,
            } => self.on_aux_done(panel, gen, note, success),
            Msg::VaultCreated { epoch, unlocked } => self.on_vault_created(epoch, unlocked),
            Msg::VaultUnlockFailed { epoch, error } => self.on_vault_unlock_failed(epoch, error),
            Msg::VaultCreateFailed { epoch, error } => self.on_vault_create_failed(epoch, error),
            Msg::VaultUnlocked { epoch, unlocked } => self.on_vault_unlocked(epoch, unlocked),
            Msg::VaultPasswordRotated { epoch } => self.on_vault_password_rotated(epoch),
            Msg::VaultPasswordRotationFailed { epoch, error } => {
                self.on_vault_password_rotation_failed(epoch, &error)
            }
            Msg::VaultBiometricFailed { epoch } => self.on_vault_biometric_failed(epoch),
            Msg::CredentialLoaded {
                panel,
                epoch,
                result,
            } => self.on_credential_loaded(panel, epoch, result),
            Msg::UpgradableReceived {
                panel,
                gen,
                summary,
            } => self.on_upgradable_received(panel, gen, summary),
        }
    }

    fn on_packet(
        &mut self,
        panel: usize,
        gen: u64,
        epoch: u64,
        payload: &multitop_agent::proto::Payload,
        dims: (u16, u16),
    ) -> bool {
        // Checked once, for every payload, before anything is written.
        // The `Monitor` arm below used to be reachable with no guard at
        // all, so a packet from a task bound to the previous panel list
        // painted whichever host had moved into its slot.
        if epoch != self.panels_epoch {
            return false;
        }
        let pal = self.current_theme();
        let sort = self.sort;
        let accepts = self.accepts(panel, gen);
        let Some(p) = self.panels.get_mut(panel) else {
            return false;
        };

        match &payload {
            // The banner host name is drawn on every panel whatever
            // view it is in, so a Monitor packet always changes what is
            // on screen.
            multitop_agent::proto::Payload::Monitor(snap) => {
                // Sampled here rather than in the Graphs view, so a
                // panel the user has not looked at yet still has a past
                // to draw when they do.
                p.history.record(snap);
                crate::history_store::save(&snap.host, &p.history);
                p.last_monitor = Some(payload.clone());
                let lines = crate::render_payload::render_payload(payload, dims, sort, pal);
                p.last_frame = Some(lines.clone());
                match p.mode {
                    Mode::Monitor => p.show_frame(lines),
                    Mode::Graphs => {
                        let g = crate::graphs::render_graphs(
                            &p.history,
                            dims.0 as usize,
                            dims.1 as usize,
                            pal,
                        );
                        p.show_frame(g);
                    }
                    _ => {}
                }
                if !self.alert_targets.is_empty() {
                    if let Some(t) = self.alert_cpu {
                        let val = crate::notify::pct_u8(snap.cpu_pct);
                        if val >= t {
                            crate::notify::dispatch_breach_notification(
                                &p.server,
                                "CPU",
                                val,
                                t,
                                &self.alert_targets,
                            );
                        }
                    }
                }
                true
            }
            multitop_agent::proto::Payload::Docker { .. } => {
                p.last_docker = Some(payload.clone());
                let shown = p.mode == Mode::Docker && accepts;
                if shown {
                    let lines = crate::render_payload::render_payload(payload, dims, sort, pal);
                    p.show_frame(lines);
                }
                shown
            }
            multitop_agent::proto::Payload::Fetch(snap) => {
                p.last_fetch = Some(snap.clone());
                let shown = p.mode == Mode::Fetch && accepts;
                if shown {
                    let lines = crate::fetch_render::render_fetch(
                        snap,
                        dims.0 as usize,
                        dims.1 as usize,
                        pal,
                    );
                    p.show_frame(lines);
                }
                shown
            }
            // An Exec frame does not belong on a stats stream. It
            // arrives on the upgrade channel, is read by the upgrade
            // task, and never reaches here.
            //
            // Ignored rather than rendered, and deliberately not
            // `unreachable!()`: the packet on the far end of that pipe
            // is written by a *remote* binary this build does not
            // control the version of, so "cannot happen" is a claim
            // about someone else's host. Nothing on screen changes, and
            // the panel keeps streaming.
            multitop_agent::proto::Payload::Exec(_) | multitop_agent::proto::Payload::Hello(_) => {
                false
            }
        }
    }

    fn on_frame(&mut self, panel: usize, epoch: u64, lines: Vec<String>) -> bool {
        if epoch != self.panels_epoch {
            return false;
        }
        let Some(p) = self.panels.get_mut(panel) else {
            return false;
        };
        p.last_frame = Some(lines);
        // Only paint it if stats is what the user is looking at.
        if p.mode == Mode::Monitor {
            p.show_last_frame();
            true
        } else {
            false
        }
    }

    fn on_status(&mut self, panel: usize, gen: u64, text: String) -> bool {
        if self.accepts(panel, gen) {
            let p = &mut self.panels[panel];
            if p.mode == Mode::Upgrade {
                // In the Upgrade view a status note is one more line in
                // the log. Replacing the whole view here wiped the
                // status header *and* every line of output collected so
                // far, which is what left panels showing nothing but
                // "sudo ready" in the middle of a run.
                p.last_upgrade.push(text);
            } else {
                p.show_body(std::iter::once(text));
            }
            true
        } else {
            false
        }
    }

    fn on_fetch_data(
        &mut self,
        panel: usize,
        gen: u64,
        snap: multitop_agent::fetch::FetchSnapshot,
        lines: Vec<String>,
    ) -> bool {
        if self.accepts(panel, gen) {
            self.panels[panel].last_fetch = Some(snap);
            self.panels[panel].show_frame(lines);
            true
        } else {
            false
        }
    }

    fn on_aux_begin(&mut self, panel: usize, gen: u64, header: Option<String>) -> bool {
        if self.accepts(panel, gen) {
            let p = &mut self.panels[panel];
            // In the Upgrade view the pane already has its status header
            // and is about to receive output; replacing the whole view
            // here threw that away on every single run, leaving nothing
            // but a bare "Upgrade on <host>" line that the panel banner
            // then overwrote. Other views use this as their reset.
            if p.mode == Mode::Upgrade {
                false
            } else {
                p.show_body(header);
                true
            }
        } else {
            false
        }
    }

    fn on_aux_repaint(
        &mut self,
        panel: usize,
        gen: u64,
        line: String,
        back: usize,
        erase_below: usize,
    ) -> bool {
        let Some(p) = self.panels.get_mut(panel) else {
            return false;
        };
        // Same ownership rule as `AuxLine`: the durable log is keyed on
        // `upgrade_gen`, so a repaint keeps landing while the user is
        // looking at another view.
        let belongs =
            p.upgrade_state == crate::panel::UpgradeState::STARTED && p.upgrade_gen == gen;
        if !belongs {
            return false;
        }
        // `back` counts rows up from where the next line would be
        // appended, so `back == 1` is the newest line in the log. Past
        // the end of what the log still holds, the block the tool is
        // addressing has already scrolled away; appending is the honest
        // answer, because the alternative is dropping output silently.
        if back == 0 || back > p.last_upgrade.len() {
            p.last_upgrade.push(line);
        } else {
            let from_newest = back - 1;
            p.last_upgrade.overwrite_from_end(from_newest, &line);
            // Rows the shrunken block left behind, blanked from the
            // write downwards. `checked_sub` stops at the newest line
            // rather than wrapping onto the far end of the ring.
            for row in 1..=erase_below {
                let Some(i) = from_newest.checked_sub(row) else {
                    break;
                };
                p.last_upgrade.overwrite_from_end(i, "");
            }
        }
        p.mode == Mode::Upgrade
    }

    fn on_aux_line(&mut self, panel: usize, gen: u64, line: String) -> bool {
        let cap = self.upgrade_history_lines;
        let Some(p) = self.panels.get_mut(panel) else {
            return false;
        };
        // `last_upgrade` is the durable log for this panel's upgrade. It
        // is keyed on `upgrade_gen`, not `gen`, so it keeps filling while
        // the user is looking at another view. Pushing into the ring
        // reuses the oldest slot's allocation in place -- no clone, no
        // shift -- and the pane renders from it, so there is no separate
        // view copy to keep in sync.
        let belongs =
            p.upgrade_state == crate::panel::UpgradeState::STARTED && p.upgrade_gen == gen;
        let visible = if belongs {
            p.mode == Mode::Upgrade
        } else {
            p.gen == gen
        };
        if belongs {
            p.last_upgrade.push(line);
        } else if p.gen == gen {
            // Output for the current view's own run (fetch/docker error
            // lines): it belongs to the visible pane only.
            push_capped(&mut p.view, line, cap);
        }
        visible
    }

    fn on_aux_done(&mut self, panel: usize, gen: u64, note: Option<String>, success: bool) -> bool {
        if !self.accepts(panel, gen)
            && !self.panels.get(panel).is_some_and(|p| {
                p.upgrade_gen == gen && p.upgrade_state == crate::panel::UpgradeState::STARTED
            })
        {
            return false;
        }
        let cap = self.upgrade_history_lines;
        // Captured before the state flips to DONE below.
        let belongs = self.panels[panel].upgrade_state == crate::panel::UpgradeState::STARTED
            && self.panels[panel].upgrade_gen == gen;
        if belongs {
            self.panels[panel].upgrade_state = crate::panel::UpgradeState::DONE;
            let now = Self::now_secs();

            // Record this host's outcome regardless of how the others
            // fared — the panel shows its own history, and a failure on
            // one host must not erase another host's success.
            let key = crate::password_store::account(&self.panels[panel].server);
            let entry = self.host_updates.entry(key).or_default();
            entry.finished_at = Some(now);
            entry.success = success;

            let duration = entry.duration_secs();
            if !self.alert_targets.is_empty() {
                crate::notify::dispatch_upgrade_notification(
                    &self.panels[panel].server,
                    success,
                    duration,
                    &self.alert_targets,
                );
            }

            if success && !self.upgrades_in_flight() {
                self.last_update = Some(now);
                self.upgrade_started_at = None;
            }
            if !self.upgrades_in_flight() {
                self.quit_armed = false;
            }
            self.persist_state();
        }
        if let Some(note) = note {
            let p = &mut self.panels[panel];
            // The completion marker belongs in the durable log too. It
            // used to go only into `view`, so finishing while the user
            // was on another view appended it to *that* view and lost it
            // from the upgrade output for good.
            if belongs {
                p.last_upgrade.push(note);
            } else if p.gen == gen {
                push_capped(&mut p.view, note, cap);
            }
        }
        // The pane's status header is recomposed by the renderer from
        // `upgrade_state` every frame, so a finished run stops
        // advertising itself as running without a rebuild here.
        true
    }

    fn on_credential_loaded(
        &mut self,
        panel: usize,
        epoch: u64,
        result: Result<Option<String>, String>,
    ) -> bool {
        if epoch != self.panels_epoch {
            return false;
        }
        let Some(p) = self.panels.get_mut(panel) else {
            return false;
        };
        p.answer_credential_load(result);
        true
    }

    fn on_upgradable_received(&mut self, panel: usize, gen: u64, summary: String) -> bool {
        if let Some(p) = self.panels.get_mut(panel) {
            if p.gen == gen {
                p.upgradable = Some(summary);
                return true;
            }
        }
        false
    }
}

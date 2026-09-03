#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;

mod passwords_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::app::App;
    use crate::config::Server;
    use crossterm::event::KeyCode;

    fn test_server(host: &str) -> Server {
        Server {
            host: host.to_string(),
            port: 22,
            user: "admin".to_string(),
            upgrade_cmd: Some("sudo apt update".to_string()),
            custom_command: None,
        }
    }

    #[test]
    fn server_key_draft_field_navigation() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));

        crate::passwords::handle_key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .field,
            1
        );

        crate::passwords::handle_key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .field,
            2
        );

        crate::passwords::handle_key(&mut app, KeyCode::Up);
        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .field,
            1
        );

        crate::passwords::handle_key(&mut app, KeyCode::Down);
        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .field,
            2
        );
    }

    #[test]
    fn server_key_draft_char_input() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));

        crate::passwords::handle_key(&mut app, KeyCode::Char('t'));
        crate::passwords::handle_key(&mut app, KeyCode::Char('e'));
        crate::passwords::handle_key(&mut app, KeyCode::Char('s'));
        crate::passwords::handle_key(&mut app, KeyCode::Char('t'));

        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .host,
            "test"
        );
    }

    #[test]
    fn server_key_draft_backspace() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .host = "test".to_string();

        crate::passwords::handle_key(&mut app, KeyCode::Backspace);
        assert_eq!(
            app.password_manager
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .host,
            "tes"
        );
    }

    #[test]
    fn server_key_draft_enter_valid() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .host = "newhost".to_string();
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .user = "user".to_string();
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .port = "22".to_string();
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .upgrade_cmd = "cmd".to_string();

        let action = crate::passwords::handle_key(&mut app, KeyCode::Enter);
        assert!(matches!(
            action,
            PasswordAction::ApplyServers(_) | PasswordAction::ApplyServerEdit { .. }
        ));
    }

    #[test]
    fn server_key_draft_enter_invalid() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .host = "host with spaces".to_string();

        let action = crate::passwords::handle_key(&mut app, KeyCode::Enter);
        assert_eq!(action, PasswordAction::None);
        assert!(app.password_manager.as_ref().unwrap().notice.is_some());
        assert!(app.password_manager.as_ref().unwrap().draft.is_some());
    }

    #[test]
    fn server_key_draft_esc_cancels() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);
        app.password_manager.as_mut().unwrap().draft = Some(ServerDraft::new(None, None, None));
        app.password_manager
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .host = "test".to_string();

        let action = crate::passwords::handle_key(&mut app, KeyCode::Esc);
        assert_eq!(action, PasswordAction::None);
        assert!(app.password_manager.as_ref().unwrap().draft.is_none());
    }

    /// `s` is unbound in Settings. It toggled sparklines, and sparklines are
    /// gone -- a key that silently does nothing is worse than one that is free,
    /// because the next feature to want `s` has to discover it is available.
    #[test]
    fn s_is_not_bound_to_anything() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('s'));
        assert_eq!(action, PasswordAction::None);
    }

    /// `d` removes the server, and is the only meaning `d` has now. It used to
    /// mean "delete the password" in one section and "delete the server" in the
    /// other, on the same key, over the same list of hosts.
    #[test]
    fn d_removes_a_server_after_confirmation() {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        crate::passwords::open(&mut app, 0, false);

        crate::passwords::handle_key(&mut app, KeyCode::Char('d'));
        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('y'));
        let PasswordAction::ApplyServers(remaining) = action else {
            panic!("expected the server list to change");
        };
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].host, "host2");
    }

    /// Removing a server rewrites config.toml and cannot be undone, so it takes
    /// two keys. It used to happen on the first press, with no question asked --
    /// while running an upgrade, the less destructive of the two, had a confirm
    /// modal.
    #[test]
    fn server_delete_asks_before_removing() {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        crate::passwords::open(&mut app, 0, false);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('d'));
        assert_eq!(action, PasswordAction::None, "the first press must not act");
        let manager = app.password_manager.as_ref().unwrap();
        assert_eq!(manager.pending_delete, Some(0));
        assert!(
            manager.notice.as_ref().unwrap().contains("host1"),
            "the question must name what is about to be removed"
        );
    }

    #[test]
    fn server_delete_goes_ahead_on_confirmation() {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        crate::passwords::open(&mut app, 0, false);
        crate::passwords::handle_key(&mut app, KeyCode::Char('d'));

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('y'));
        let PasswordAction::ApplyServers(remaining) = action else {
            panic!("expected the removal to be applied");
        };
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].host, "host2");
        assert_eq!(app.password_manager.as_ref().unwrap().pending_delete, None);
    }

    /// A destructive confirmation may only act on the key it names.
    ///
    /// The question reads `[y] confirm  [Esc] cancel`, and `Enter` confirmed
    /// too. `Enter` is *this panel's key for opening a row to edit it*, so
    /// `d` then `Enter` -- press `d` to read the question, then reach for the
    /// key you use to work on a row -- removed the host and, through
    /// `write_servers`, aborted any upgrade running on it. A `dpkg` transaction
    /// interrupted on a real machine by two keystrokes that never meant to.
    ///
    /// The quit confirmation dropped `Enter` for exactly this reason earlier in
    /// the same round; this was that fix's surviving sibling.
    #[test]
    fn enter_does_not_confirm_a_removal_it_was_never_offered_for() {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        crate::passwords::open(&mut app, 0, false);
        crate::passwords::handle_key(&mut app, KeyCode::Char('d'));

        let question = app
            .password_manager
            .as_ref()
            .unwrap()
            .notice
            .clone()
            .unwrap_or_default();
        assert!(
            !question.contains("Enter"),
            "the question does not offer Enter: {question}"
        );

        let action = crate::passwords::handle_key(&mut app, KeyCode::Enter);
        assert_eq!(
            action,
            PasswordAction::None,
            "a key the question never named must not remove a host"
        );
        assert_eq!(
            app.panels.len(),
            2,
            "and nothing may be taken out of the list"
        );
    }

    #[test]
    fn server_delete_can_be_cancelled() {
        for cancel in [KeyCode::Esc, KeyCode::Char('n')] {
            let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
            crate::passwords::open(&mut app, 0, false);
            crate::passwords::handle_key(&mut app, KeyCode::Char('d'));

            let action = crate::passwords::handle_key(&mut app, cancel);
            assert_eq!(action, PasswordAction::None, "{cancel:?} must not remove");
            let manager = app.password_manager.as_ref().unwrap();
            assert_eq!(manager.pending_delete, None, "{cancel:?} disarms");
            assert!(manager.notice.as_ref().unwrap().contains("cancelled"));
        }
    }

    /// A stray second press must not become the confirmation for a question the
    /// user never saw answered.
    #[test]
    fn a_cancelled_removal_stays_cancelled() {
        let mut app = App::new(vec![test_server("host1"), test_server("host2")]);
        crate::passwords::open(&mut app, 0, false);
        crate::passwords::handle_key(&mut app, KeyCode::Char('d'));
        crate::passwords::handle_key(&mut app, KeyCode::Esc);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('y'));
        assert_eq!(
            action,
            PasswordAction::None,
            "y after a cancel must not remove anything"
        );
        assert_eq!(app.panels.len(), 2);
    }

    #[test]
    fn the_last_server_still_cannot_be_removed() {
        let mut app = App::new(vec![test_server("host1")]);
        crate::passwords::open(&mut app, 0, false);

        let action = crate::passwords::handle_key(&mut app, KeyCode::Char('d'));
        assert_eq!(action, PasswordAction::None);
        let manager = app.password_manager.as_ref().unwrap();
        assert_eq!(manager.pending_delete, None, "nothing to confirm");
        assert!(manager.notice.as_ref().unwrap().contains("Cannot remove"));
    }

    #[test]
    fn credential_lookup_is_dispatched_once_and_cached_afterwards() {
        let mut panel = crate::panel::Panel::new(test_server("host1"));
        assert!(panel.needs_credential_load());
        assert!(!panel.password_checked);

        // Dispatching marks the host as answered-so it can never be dispatched
        // twice while the lookup is in flight.
        panel.mark_credential_load_dispatched();
        assert!(panel.password_checked);
        assert!(panel.password_checking);
        assert!(!panel.needs_credential_load());

        // Nothing stored: the answer lands as "no password", still an answer.
        panel.answer_credential_load(Ok(None));
        assert!(!panel.password_checking);
        assert!(panel.password_checked);
        assert_eq!(panel.sudo_password, None);

        // A stored answer is kept and shown as saved.
        let mut panel2 = crate::panel::Panel::new(test_server("host1"));
        panel2.mark_credential_load_dispatched();
        panel2.answer_credential_load(Ok(Some("secret".to_string())));
        assert_eq!(panel2.sudo_password.as_deref(), Some("secret"));
        assert!(panel2.password_saved);
        assert!(!panel2.password_checking);
    }
}

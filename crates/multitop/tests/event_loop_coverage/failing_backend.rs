use super::*;

// ------------------------------------------------------- a failing terminal

/// A backend whose draw fails, standing in for the terminal going away
/// mid-frame. The loop has to report that *and* still name the upgrades it
/// killed on the way out — the notice used to sit behind a `?` on the result.
struct FailingBackend {
    inner: TestBackend,
    draws_left: std::cell::Cell<usize>,
}

impl ratatui::backend::Backend for FailingBackend {
    type Error = std::io::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a ratatui::buffer::Cell)>,
    {
        if self.draws_left.get() == 0 {
            return Err(std::io::Error::other("the terminal went away"));
        }
        self.draws_left.set(self.draws_left.get() - 1);
        self.inner.draw(content).map_err(std::io::Error::other)
    }
    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor().map_err(std::io::Error::other)
    }
    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor().map_err(std::io::Error::other)
    }
    fn get_cursor_position(&mut self) -> Result<ratatui::layout::Position, Self::Error> {
        self.inner
            .get_cursor_position()
            .map_err(std::io::Error::other)
    }
    fn set_cursor_position<P: Into<ratatui::layout::Position>>(
        &mut self,
        position: P,
    ) -> Result<(), Self::Error> {
        self.inner
            .set_cursor_position(position)
            .map_err(std::io::Error::other)
    }
    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear().map_err(std::io::Error::other)
    }
    fn clear_region(&mut self, region: ratatui::backend::ClearType) -> Result<(), Self::Error> {
        self.inner
            .clear_region(region)
            .map_err(std::io::Error::other)
    }
    fn size(&self) -> Result<ratatui::layout::Size, Self::Error> {
        self.inner.size().map_err(std::io::Error::other)
    }
    fn window_size(&mut self) -> Result<ratatui::backend::WindowSize, Self::Error> {
        self.inner.window_size().map_err(std::io::Error::other)
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush().map_err(std::io::Error::other)
    }
}

#[tokio::test]
async fn a_terminal_that_fails_mid_frame_reports_it_and_the_upgrades_it_killed() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _rx) = tokio::sync::watch::channel((0, 0));

    // One good frame, then the terminal is gone.
    let backend = FailingBackend {
        inner: TestBackend::new(100, 30),
        draws_left: std::cell::Cell::new(1),
    };
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let mut stream =
        tokio_stream::iter(vec![Ok(Event::Resize(90, 28))]).chain(tokio_stream::pending());

    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            vec![test_server("alpha.example")],
            config_path,
            None,
        ),
    )
    .await
    .expect("a failing draw must end the loop");

    let error = outcome
        .error
        .expect("the terminal failure must be reported");
    assert!(
        error.to_string().contains("went away"),
        "the reason was replaced: {error}"
    );
}

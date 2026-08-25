use super::*;

// --------------------------------------------------------------- through the app

fn press(app: &mut App, code: KeyCode, tx: &mpsc::Sender<Msg>, tasks: &mut Tasks) {
    let (dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    std::mem::forget(dims_tx);
    handle_key(
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press),
        app,
        (80, 24),
        Arc::new(dims_rx),
        tx,
        tasks,
    );
}

fn monitor(cpu: f64) -> Payload {
    Payload::Monitor(snapshot(cpu, 40, 512.0, 512.0))
}

#[tokio::test]
async fn g_puts_every_panel_into_the_graph_view_and_spawns_nothing() {
    // The Monitor stream is already running and already feeding the history.
    // Spawning anything here would be a second source for the same numbers.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha"), test_server("beta")]);
    let (tx, mut rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(2);

    press(&mut app, KeyCode::Char('g'), &tx, &mut tasks);

    assert!(app.in_graphs());
    assert!(app.panels.iter().all(|p| p.mode == Mode::Graphs));
    assert!(rx.try_recv().is_err(), "the graph view spawned work");

    // Pressing it again is a documented no-op, not a rebuild.
    press(&mut app, KeyCode::Char('G'), &tx, &mut tasks);
    assert!(app.in_graphs());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn s_leaves_the_graph_view_for_the_stats_it_was_drawn_from() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);

    press(&mut app, KeyCode::Char('g'), &tx, &mut tasks);
    assert!(app.in_graphs());
    press(&mut app, KeyCode::Char('s'), &tx, &mut tasks);
    assert!(!app.in_graphs());
    assert_eq!(app.panels[0].mode, Mode::Monitor);
}

#[tokio::test]
async fn a_panel_fills_its_history_from_a_view_the_user_is_not_looking_at() {
    // The point of sampling where the packet lands: switching to the graphs
    // must not start from an empty history.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    let gen = app.panels[0].gen;
    assert_eq!(app.panels[0].mode, Mode::Monitor);

    for cpu in [10.0, 20.0, 30.0] {
        app.apply(Msg::Packet {
            panel: 0,
            gen,
            epoch: app.panels_epoch,
            payload: monitor(cpu),
            dims: (80, 12),
        });
    }
    assert_eq!(app.panels[0].history.cpu.tail(8), vec![10.0, 20.0, 30.0]);

    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    press(&mut app, KeyCode::Char('g'), &tx, &mut tasks);

    let shown = app.panels[0].view.join("\n");
    assert!(
        !shown.contains("no samples yet"),
        "the graph view started from nothing:\n{shown}"
    );
    assert!(
        shown.contains("30%"),
        "the newest reading is missing:\n{shown}"
    );
}

#[tokio::test]
async fn a_packet_arriving_while_the_graphs_are_up_redraws_the_graphs() {
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    press(&mut app, KeyCode::Char('g'), &tx, &mut tasks);
    let gen = app.panels[0].gen;

    app.apply(Msg::Packet {
        panel: 0,
        gen,
        epoch: app.panels_epoch,
        payload: monitor(88.0),
        dims: (80, 12),
    });

    let shown = app.panels[0].view.join("\n");
    assert!(
        shown.contains("88%"),
        "the graphs did not follow the new packet:\n{shown}"
    );
    // And it is the graphs, not the stats table underneath.
    assert!(
        shown.contains("CPU") && !shown.contains("PID"),
        "the stats frame was drawn over the graph view:\n{shown}"
    );
}

#[tokio::test]
async fn a_resize_redraws_the_graphs_at_the_new_width() {
    // Braille cells cannot be refitted -- stretching them turns a graph into
    // noise -- so a resize has to redraw from the history.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("alpha")]);
    let gen = app.panels[0].gen;
    for cpu in [10.0, 90.0] {
        app.apply(Msg::Packet {
            panel: 0,
            gen,
            epoch: app.panels_epoch,
            payload: monitor(cpu),
            dims: (80, 12),
        });
    }
    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(1);
    press(&mut app, KeyCode::Char('g'), &tx, &mut tasks);

    let braille = |app: &App| -> usize {
        app.panels[0]
            .view
            .iter()
            .map(|l| {
                l.chars()
                    .filter(|c| ('\u{2800}'..='\u{28ff}').contains(c))
                    .count()
            })
            .max()
            .unwrap_or(0)
    };

    let wide = braille(&app);
    assert!(wide > 0, "the graph drew no braille at all");
    app.rerender_all((40, 24));
    let narrow = braille(&app);
    assert!(
        narrow < wide,
        "the graphs kept their old width across a resize: {wide} then {narrow}"
    );
    app.rerender_all((120, 24));
    assert!(
        braille(&app) > wide,
        "the graphs did not grow back into the wider pane"
    );
}

#[tokio::test]
async fn the_keybar_offers_the_graph_view() {
    // A view nothing on screen mentions is a view nobody finds.
    let _g = isolate().await;
    let line = multitop::ui::keybar_line(
        multitop_agent::SortBy::Cpu,
        PLAIN,
        120,
        Mode::Monitor,
        multitop::ui::FilterHint::Off,
    );
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.contains("raphs"), "no way to discover `G`: {text:?}");
}

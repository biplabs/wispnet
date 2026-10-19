use super::*;
use ratatui::{Terminal, backend::TestBackend};
fn fixture() -> (App, mpsc::Receiver<Work>, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let api = Api {
        config: root.path().join("host.toml"),
        state: root.path().to_owned(),
    };
    let mut app = App::new(api);
    let (tx, rx) = mpsc::channel();
    app.tx = tx;
    let client = "b".repeat(64);
    let host = "a".repeat(64);
    app.snapshot = Some(
        json!({"api_version":1,"host_name":"Test computer","relay":"connected","connected_peers":{},"peer_pins":[],"recent_peers":{},"peer_telemetry":{},"invitations":[{"invitation":"c".repeat(64),"peer":client,"name":"Fixture phone","state":"pending","expires_at":now()+300}],"host":{"host":host,"revision":7,"services":[],"clients":[{"peer":client,"name":"Fixture phone"}]}}),
    );
    app.online = true;
    app.last_poll = Instant::now();
    (app, rx, root)
}
#[test]
fn approval_confirms_the_original_peer_invitation_and_revision() {
    let (mut app, rx, _) = fixture();
    let invitation = app.pending()[0].clone();
    app.action(Action::Pending(invitation.clone()));
    assert!(rx.try_recv().is_err());
    app.key(KeyCode::Enter.into()); // Review -> service selection form.
    assert!(rx.try_recv().is_err());
    app.snapshot.as_mut().unwrap()["host"]["revision"] = json!(8);
    app.key(KeyCode::Enter.into()); // Review chosen services.
    app.key(KeyCode::Enter.into()); // Confirm one atomic approval.
    let Work {
        task: Task::Request(v),
        ..
    } = rx.try_recv().unwrap()
    else {
        panic!("Expected owner mutation")
    };
    assert_eq!(
        v["operation"],
        json!({"op":"approve_invitation","invitation":invitation["invitation"],"expected_revision":7,"services":[]})
    );
    assert!(rx.try_recv().is_err()); // No automatic retry or replacement revision.
}
#[test]
fn service_form_explicit_grants_and_revision_survive_concurrent_refresh() {
    let (mut app, rx, _) = fixture();
    let mut form = app.service_form(None);
    form.fields[0].value = "Local dashboard".into();
    form.fields[1].value = "8310".into();
    form.fields[3].kind = FieldKind::Check(false);
    app.save_form(form).unwrap();
    app.snapshot.as_mut().unwrap()["host"]["revision"] = json!(9);
    app.key(KeyCode::Enter.into());
    let Work {
        task: Task::Request(v),
        ..
    } = rx.try_recv().unwrap()
    else {
        panic!("Expected service mutation")
    };
    assert_eq!(v["operation"]["allow"], json!([]));
    assert_eq!(v["operation"]["backend"], "127.0.0.1:8310");
    assert_eq!(v["operation"]["id"], "local-dashboard");
    assert_eq!(v["operation"]["expected_revision"], 7);
}
#[test]
fn cancellation_offline_state_and_private_paste_never_send_a_mutation() {
    let (mut app, rx, _) = fixture();
    app.action(Action::Machine("b".repeat(64)));
    app.key(KeyCode::Enter.into()); // Remove confirmation.
    app.key(KeyCode::Esc.into());
    assert!(rx.try_recv().is_err());
    app.online = false;
    app.send(app.mutate(json!({"op":"revoke","peer":"b".repeat(64),"expected_revision":7})));
    assert!(rx.try_recv().is_err());
    app.action(Action::PairInput(false));
    app.paste("wispnet1:private-code");
    assert!(!app.key(KeyCode::Char('q').into()));
    let mut terminal = Terminal::new(TestBackend::new(90, 25)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(!screen.contains("private-code"));
    assert!(screen.contains("private invitation"));
    assert!(rx.try_recv().is_err());
}
#[test]
fn qr_expires_or_moves_to_review_and_small_terminals_do_not_clip_it() {
    let (mut app, _, _) = fixture();
    app.page = Page::Qr;
    app.qr = Some(
        json!({"host":"a".repeat(64),"invitation":"c".repeat(64),"expires_at":now()+300,"matrix":vec![vec![false;101];101]}),
    );
    for (size_x, size_y) in [(40, 10), (80, 24), (120, 70)] {
        let mut terminal = Terminal::new(TestBackend::new(size_x, size_y)).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        if size_x == 80 {
            assert!(screen.contains("Enlarge the terminal"));
        }
    }
    app.receive();
    assert!(app.qr.is_none());
    assert!(app.page == Page::Pending);
    app.page = Page::Qr;
    app.qr = Some(json!({"invitation":"d".repeat(64),"expires_at":1}));
    app.receive();
    assert!(app.qr.is_none());
}
#[test]
fn owner_target_errors_are_not_replaced_with_an_unintended_local_target() {
    let (mut app, rx, root) = fixture();
    std::fs::write(root.path().join("admin-targets.json"), "invalid").unwrap();
    app.action(Action::OwnerNew(false));
    assert!(app.modal.is_none());
    assert!(!app.message.is_empty());
    assert!(rx.try_recv().is_err());
}
#[test]
fn confirmed_actions_wait_for_background_poll_without_replacing_their_revision() {
    let (mut app, rx, _) = fixture();
    let (done, replies) = mpsc::channel();
    app.rx = replies;
    app.busy = true;
    let work = app.mutate(json!({"op":"revoke","peer":"b".repeat(64),"expected_revision":7}));
    app.send(work);
    assert!(rx.try_recv().is_err());
    assert!(app.pending_work.is_some());
    let mut status = app.status();
    status["host"]["revision"] = json!(8);
    done.send(Reply {
        kind: Kind::Status,
        result: Ok(status),
    })
    .unwrap();
    app.receive();
    let Work {
        task: Task::Request(value),
        ..
    } = rx.try_recv().unwrap()
    else {
        panic!("Missing confirmed action")
    };
    assert_eq!(value["operation"]["expected_revision"], 7);
    assert!(rx.try_recv().is_err());
}

#[test]
fn approval_selects_services_and_locks_existing_grants_to_the_reviewed_revision() {
    let (mut app, rx, _) = fixture();
    app.snapshot.as_mut().unwrap()["host"]["services"] = json!([
        {"id":"alpha","display_name":"Alpha","kind":"http","allow":[]},
        {"id":"beta","display_name":"Beta","kind":"http","allow":["b".repeat(64)]},
        {"id":"gamma","display_name":"Gamma","kind":"tcp","allow":[]}
    ]);
    app.action(Action::Pending(app.pending()[0].clone()));
    app.key(KeyCode::Enter.into());
    let Some(Modal::Form(form)) = &app.modal else {
        panic!("Missing services form")
    };
    assert!(matches!(form.fields[2].kind, FieldKind::Check(false)));
    assert!(matches!(form.fields[3].kind, FieldKind::Granted));
    assert!(matches!(form.fields[4].kind, FieldKind::Check(false)));
    app.key(KeyCode::Char(' ').into()); // Alpha selected.
    app.key(KeyCode::Down.into());
    app.key(KeyCode::Char(' ').into()); // Existing Beta stays granted.
    app.key(KeyCode::Down.into()); // Gamma remains unchecked.
    app.key(KeyCode::Down.into()); // Review approval.
    app.key(KeyCode::Enter.into());
    assert!(rx.try_recv().is_err());
    app.snapshot.as_mut().unwrap()["host"]["revision"] = json!(8);
    app.key(KeyCode::Enter.into());
    let Work {
        task: Task::Request(request),
        ..
    } = rx.try_recv().unwrap()
    else {
        panic!("Missing atomic approval")
    };
    assert_eq!(request["operation"]["services"], json!(["alpha", "beta"]));
    assert_eq!(request["operation"]["expected_revision"], 7);
}

#[test]
fn release_version_stays_at_bottom_right_including_small_terminals() {
    let (mut app, _, _) = fixture();
    for (width, height) in [(100, 30), (65, 16), (60, 10)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let line: String = (0..width)
            .map(|x| terminal.backend().buffer()[(x, height - 1)].symbol())
            .collect();
        assert!(line.ends_with(concat!("v", env!("WISPNET_VERSION"))));
    }
}

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
    form.fields[0].value = "local-dashboard".into();
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
    assert!(v["operation"].get("display_name").is_none());
    assert_eq!(v["operation"]["expected_revision"], 7);
}
#[test]
fn cancellation_offline_state_and_visible_code_paste_never_send_a_mutation() {
    let (mut app, rx, _) = fixture();
    app.action(Action::Machine("b".repeat(64)));
    app.key(KeyCode::Enter.into()); // Remove confirmation.
    app.key(KeyCode::Esc.into());
    assert!(rx.try_recv().is_err());
    app.online = false;
    app.send(app.mutate(json!({"op":"revoke","peer":"b".repeat(64),"expected_revision":7})));
    assert!(rx.try_recv().is_err());
    app.action(Action::PairInput);
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
    assert!(screen.contains("wispnet1:private-code"));
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

#[test]
fn pairing_code_generation_is_available_and_expired_codes_cannot_be_copied() {
    let (mut app, rx, _) = fixture();
    app.key(KeyCode::Char('n').into());
    assert!(app.show_code);
    assert!(matches!(rx.try_recv().unwrap().kind, Kind::Qr));
    let code = format!("wispnet1:{}", "a".repeat(700));
    app.qr = Some(json!({"code":code,"expires_at":now()+300}));
    assert_eq!(app.active_pairing_code(), Some(code.as_str()));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(text.contains("wispnet1:"));
    assert!(text.contains("copy complete code"));
    app.qr.as_mut().unwrap()["expires_at"] = json!(1);
    assert!(app.active_pairing_code().is_none());
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(!text.contains("wispnet1:"));
}

#[test]
fn pairing_text_box_wraps_long_codes_and_normalizes_line_breaks() {
    let (mut app, rx, _) = fixture();
    app.action(Action::PairInput);
    let code = format!("wispnet1:{}tail", "ABCDEFGH".repeat(32));
    app.paste(&format!("{}\n{}", &code[..30], &code[30..]));
    let mut terminal = Terminal::new(TestBackend::new(90, 25)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let lines: Vec<String> = terminal
        .backend()
        .buffer()
        .content()
        .chunks(90)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect();
    let first = lines
        .iter()
        .position(|line| line.contains("wispnet1:"))
        .unwrap();
    let last = lines.iter().position(|line| line.contains("tail")).unwrap();
    assert!(last > first);
    app.key(KeyCode::Tab.into());
    app.key(KeyCode::Enter.into());
    let work = rx.try_recv().unwrap();
    match work.task {
        Task::Request(value) => assert_eq!(value["invitation"], code),
        _ => panic!("Expected pairing preview"),
    }
}

#[test]
fn settings_require_valid_values_and_explicit_confirmation() {
    let (mut app, rx, _) = fixture();
    let mut form = Form {
        title: "Settings".into(),
        kind: FormKind::Settings,
        fields: vec![
            Field::text("ID", "my-desktop", 64),
            Field::text("Port", "80", 5),
        ],
        selected: 0,
    };
    assert!(app.save_form(form.clone()).is_err());
    assert!(rx.try_recv().is_err());
    form.fields[1].value = "7842".into();
    form.fields[0].value = "Bad ID".into();
    assert!(app.save_form(form.clone()).is_err());
    form.fields[0].value = "my-desktop".into();
    app.save_form(form).unwrap();
    assert!(rx.try_recv().is_err());
    app.key(KeyCode::Enter.into());
    assert!(
        matches!(rx.try_recv().unwrap().task, Task::SaveSettings { name, port: 7842 } if name == "my-desktop")
    );
}

#[test]
fn remote_service_links_do_not_launch_a_browser() {
    let (mut app, rx, _) = fixture();
    let service = json!({"id":"llama","kind":"http","incarnation":"c".repeat(64)});
    app.action(Action::Remote("b".repeat(64), service));
    assert!(rx.try_recv().is_err());
    app.key(KeyCode::Down.into());
    app.key(KeyCode::Enter.into());
    assert!(matches!(rx.try_recv().unwrap().task, Task::Links { .. }));
}

#[test]
fn devices_always_list_remote_services_and_explicit_empty_states() {
    let (mut app, _, _) = fixture();
    let peer = "b".repeat(64);
    let other = "d".repeat(64);
    app.snapshot.as_mut().unwrap()["peer_pins"] = json!([peer, other]);
    app.catalogs.insert(
        peer.clone(),
        json!({"host":peer,"services":[{"id":"dashboard","kind":"http","incarnation":"one"}]}),
    );
    app.catalogs
        .insert(other.clone(), json!({"host":other,"services":[]}));
    let rows = app.entries();
    assert_eq!(rows[0].id, "this-device");
    let sector = rows
        .iter()
        .position(|row| row.id == "remote-sector")
        .unwrap();
    assert!(matches!(rows[sector].action, Action::Section));
    let machine = rows.iter().position(|row| row.id == peer).unwrap();
    assert!(machine > sector);
    assert_eq!(rows[machine + 1].label, "  dashboard · http");
    app.selected = sector - 1;
    app.key(KeyCode::Down.into());
    assert_eq!(app.selected, sector + 1);
    app.key(KeyCode::Up.into());
    assert_eq!(app.selected, sector - 1);
    assert!(
        matches!(&rows[machine + 1].action, Action::Remote(host, service) if host == &peer && service["incarnation"] == "one")
    );
    let empty = rows.iter().position(|row| row.id == other).unwrap();
    assert_eq!(rows[empty + 1].label, "  No services");
    let local = rows.iter().position(|row| row.id == "this-device").unwrap();
    assert_eq!(rows[local + 1].label, "  No services");
    // A failed or still-loading catalog must not masquerade as an empty one.
    app.catalogs.remove(&other);
    assert_eq!(
        app.remote_service_entries(&other)[0].label,
        "  Loading services…"
    );
    app.catalog_errors.insert(other.clone(), "timed out".into());
    assert_eq!(
        app.remote_service_entries(&other)[0].label,
        "  Services unavailable"
    );
    app.catalog_errors
        .insert(other.clone(), "awaiting approval".into());
    assert_eq!(
        app.remote_service_entries(&other)[0].label,
        "  Awaiting approval"
    );
    app.online = false;
    assert_eq!(
        app.remote_service_entries(&peer)[0].label,
        "  Services unavailable · node offline"
    );
}

#[test]
fn background_catalog_results_preserve_selection_and_do_not_interrupt_actions() {
    let (mut app, _, _) = fixture();
    let peer = "b".repeat(64);
    app.snapshot.as_mut().unwrap()["peer_pins"] = json!([peer]);
    app.catalog_checked.insert(peer.clone(), Instant::now());
    app.selected = app
        .entries()
        .iter()
        .position(|row| row.id == "this-device")
        .unwrap();
    app.busy = true;
    app.modal = Some(Modal::Info {
        title: "Keep this open".into(),
        lines: vec![],
        scroll: 0,
    });
    app.catalog_inflight.insert(peer.clone(), 1);
    app.catalog_tx
        .send(CatalogReply {
            peer: peer.clone(),
            generation: 1,
            result: Ok(json!({"host":peer,"services":[{"id":"llama","kind":"http"}]})),
        })
        .unwrap();
    app.refresh_catalogs();
    assert_eq!(app.entries()[app.selected].id, "this-device");
    assert!(app.busy);
    assert!(matches!(&app.modal, Some(Modal::Info { title, .. }) if title == "Keep this open"));
    assert!(app.catalogs.contains_key(&peer));
    // A result for a removed machine is discarded, including after re-pairing.
    app.catalog_inflight.insert(peer.clone(), 2);
    app.snapshot.as_mut().unwrap()["peer_pins"] = json!([]);
    app.refresh_catalogs();
    app.snapshot.as_mut().unwrap()["peer_pins"] = json!([peer]);
    app.catalog_checked.insert(peer.clone(), Instant::now());
    app.catalog_tx
        .send(CatalogReply {
            peer: peer.clone(),
            generation: 2,
            result: Ok(json!({"host":peer,"services":[{"id":"stale","kind":"http"}]})),
        })
        .unwrap();
    app.refresh_catalogs();
    assert!(!app.catalogs.contains_key(&peer));
    // A catalog for the wrong host is never rendered as usable services.
    app.catalog_inflight.insert(peer.clone(), 3);
    app.catalog_tx
        .send(CatalogReply {
            peer: peer.clone(),
            generation: 3,
            result: Ok(json!({"host":"wrong-host","services":[]})),
        })
        .unwrap();
    app.refresh_catalogs();
    assert_eq!(app.catalog_errors[&peer], "Invalid service catalog");
}

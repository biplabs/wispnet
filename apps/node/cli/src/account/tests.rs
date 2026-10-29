use super::*;
use std::{
    io::{BufRead, BufReader},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    sync::{Arc, Mutex},
};

const BASE: &str = "https://cmd.wispnet.test";
const CODE: &str = "A1B2-C3D4-E5F6";
const URL: &str = "https://cmd.wispnet.test/connect?code=A1B2-C3D4-E5F6";

fn options(args: &[&str]) -> Options {
    Options::parse(&args.iter().map(OsString::from).collect::<Vec<_>>()).unwrap()
}
fn unlinked() -> Value {
    json!({"enabled":true,"base_url":BASE,"linked":false,"account":null,"pending":null})
}
fn pending() -> Value {
    json!({"state":"pending","user_code":CODE,"verification_url":URL})
}
fn waiting() -> Value {
    let mut value = unlinked();
    value["pending"] = pending();
    value
}
fn linked() -> Value {
    json!({"enabled":true,"base_url":BASE,"linked":true,"account":{"id":"fixture-account","name":"Fixture user"},"pending":null})
}
type Reply = (Value, Result<Value, &'static str>);
struct Node {
    api: Api,
    requests: Arc<Mutex<Vec<Value>>>,
    server: Option<std::thread::JoinHandle<()>>,
    _root: tempfile::TempDir,
}
impl Node {
    fn new(replies: Vec<Reply>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("custom-host.toml");
        std::fs::write(&config, toml::to_string(&json!({"schema_version":2,"device_id":"cli-fixture","state_dir":root.path(),"log_level":"info","transport":{"bind_addr":"127.0.0.1:0"}})).unwrap()).unwrap();
        let api = Api::new(config).unwrap();
        let socket = root.path().join("node.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let received = requests.clone();
        let server = std::thread::spawn(move || {
            for (expected, reply) in replies {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline, "Missing {expected}");
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("Fake node failed: {error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                received.lock().unwrap().push(request.clone());
                assert_eq!(request, expected);
                let response = match reply {
                    Ok(result) => json!({"ok":true,"result":result}),
                    Err(error) => json!({"ok":false,"error":error}),
                };
                writeln!(stream, "{response}").unwrap();
            }
        });
        Self {
            api,
            requests,
            server: Some(server),
            _root: root,
        }
    }
    fn finish(mut self) {
        self.server.take().unwrap().join().unwrap();
    }
}
fn status_request() -> Value {
    json!({"op":"command_center_status"})
}
fn poll_request() -> Value {
    json!({"op":"command_center_poll","expected_user_code":CODE,"expected_verification_url":URL})
}

#[test]
fn account_parser_supports_custom_config_and_rejects_ambiguous_or_unbounded_options() {
    let parsed = options(&[
        "login",
        "--no-browser",
        "--json",
        "--config",
        "/tmp/custom.toml",
        "--url",
        BASE,
        "--timeout",
        "45",
        "--restart",
    ]);
    assert_eq!(parsed.operation, Operation::Login);
    assert_eq!(parsed.config.unwrap(), PathBuf::from("/tmp/custom.toml"));
    assert_eq!(parsed.url.as_deref(), Some(BASE));
    assert_eq!(parsed.timeout, Duration::from_secs(45));
    assert!(parsed.no_browser && parsed.json && parsed.restart);
    assert_eq!(
        options(&["account", "status", "--json"]).operation,
        Operation::Status
    );
    assert!(options(&["login", "--help"]).help);
    assert!(options(&["logout", "--help"]).help);
    for args in [
        vec!["login", "--timeout", "0"],
        vec!["login", "--timeout", "601"],
        vec!["login", "--timeout", "-1"],
        vec!["login", "--config"],
        vec!["login", "--json", "--json"],
        vec!["login", "--config", "/tmp/a", "--config", "/tmp/b"],
        vec!["logout", "--no-browser"],
        vec!["account", "delete"],
        vec!["account", "--url", BASE],
        vec!["login", "--url", "http://remote.example"],
        vec!["login", "--url", "https://cmd.example/path"],
    ] {
        assert!(
            Options::parse(&args.iter().map(OsString::from).collect::<Vec<_>>()).is_err(),
            "accepted {args:?}"
        );
    }
    for private in ["shell", "host-catalog", "tunnel", "node-status"] {
        assert!(!handles(private));
    }
}

#[test]
fn browser_handoff_is_bound_to_the_origin_path_and_displayed_code() {
    assert_eq!(Pending::read(&pending(), BASE).unwrap().url, URL);
    for destination in [
        "https://elsewhere.example/connect?code=A1B2-C3D4-E5F6",
        "https://cmd.wispnet.test:444/connect?code=A1B2-C3D4-E5F6",
        "https://user@cmd.wispnet.test/connect?code=A1B2-C3D4-E5F6",
        "https://cmd.wispnet.test/other?code=A1B2-C3D4-E5F6",
        "https://cmd.wispnet.test/connect?code=OTHER",
        "https://cmd.wispnet.test/connect?code=A1B2-C3D4-E5F6&next=https://other.example",
        "https://cmd.wispnet.test/connect?code=A1B2-C3D4-E5F6&code=A1B2-C3D4-E5F6",
        "https://cmd.wispnet.test/connect?code=A1B2-C3D4-E5F6#fragment",
        "http://cmd.wispnet.test/connect?code=A1B2-C3D4-E5F6",
        "javascript:alert(1)",
    ] {
        assert!(
            Pending::read(
                &json!({"user_code":CODE,"verification_url":destination}),
                BASE
            )
            .is_err(),
            "accepted {destination}"
        );
    }
    assert!(
        Pending::read(
            &json!({"user_code":"BAD\nCODE","verification_url":URL}),
            BASE
        )
        .is_err()
    );
    assert!(Pending::read(&json!({"user_code":CODE,"verification_url":format!("http://127.0.0.1:8123/connect?code={CODE}")}), "http://127.0.0.1:8123").is_ok());
}

#[test]
fn account_output_is_allowlisted_and_sanitizes_terminal_controls() {
    let mut value = linked();
    value["publisher_token"] = json!("SECRET-PUBLISHER");
    value["credentials"] = json!([{"token":"SECRET-BACKEND"}]);
    value["account"]["token"] = json!("SECRET-ACCOUNT");
    value["account"]["name"] = json!("Fixture\u{1b}[31m\u{202e}user\n");
    value["warning"] = json!("SECRET-WARNING");
    let safe = status(&value).unwrap();
    assert!(!safe.to_string().contains("SECRET"));
    assert_eq!(safe["account"]["name"], "Fixture[31muser");
    let text = lines(&safe).unwrap().join("\n");
    assert!(text.starts_with("Signed in as Fixture[31muser"));
    assert!(!text.contains('\u{1b}') && !text.contains('\u{202e}'));
    value["remote_revocation"] = json!("unconfirmed");
    assert!(
        status(&value).unwrap()["warning"]
            .as_str()
            .unwrap()
            .contains("could not be confirmed")
    );
    assert!(!status(&value).unwrap().to_string().contains("SECRET"));
}

#[tokio::test]
async fn fresh_login_uses_the_owner_node_and_binds_poll_to_the_displayed_request() {
    let node = Node::new(vec![
        (status_request(), Ok(unlinked())),
        (
            json!({"op":"command_center_link","base_url":BASE}),
            Ok(pending()),
        ),
        (status_request(), Ok(waiting())),
        (
            poll_request(),
            Ok(
                json!({"state":"linked","account":{"id":"fixture-account","name":"Fixture user","publisher_token":"SECRET"}}),
            ),
        ),
    ]);
    let mut progress = Vec::new();
    let result = login(
        &node.api,
        &options(&["login", "--no-browser"]),
        &mut progress,
        |_| panic!("browser must stay closed"),
    )
    .await
    .unwrap();
    assert_eq!(result["linked"], true);
    assert!(!result.to_string().contains("SECRET"));
    let progress = String::from_utf8(progress).unwrap();
    assert!(progress.contains(CODE) && progress.contains(URL));
    assert!(
        !node
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r["op"] == "command_center_unlink")
    );
    node.finish();
}

#[tokio::test]
async fn retry_reuses_pending_sign_in_and_can_recover_an_unconfirmed_poll_reply() {
    let node = Node::new(vec![
        (status_request(), Ok(waiting())),
        (status_request(), Ok(waiting())),
        (
            poll_request(),
            Err("Node reply unconfirmed; refresh before retrying"),
        ),
        (status_request(), Ok(linked())),
    ]);
    let result = login(
        &node.api,
        &options(&["login", "--no-browser"]),
        &mut Vec::new(),
        |_| panic!("browser must stay closed"),
    )
    .await
    .unwrap();
    assert_eq!(result["linked"], true);
    assert!(
        !node
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r["op"] == "command_center_link")
    );
    node.finish();
}

#[tokio::test]
async fn explicit_restart_replaces_the_old_pending_flow_and_browser_failure_allows_manual_login() {
    let node = Node::new(vec![
        (status_request(), Ok(waiting())),
        (
            json!({"op":"command_center_link","base_url":BASE}),
            Ok(pending()),
        ),
        (status_request(), Ok(waiting())),
        (
            poll_request(),
            Ok(json!({"state":"linked","account":{"id":"fixture-account","name":"Fixture user"}})),
        ),
    ]);
    let opened = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let browser_called = opened.clone();
    let mut progress = Vec::new();
    let result = login(
        &node.api,
        &options(&["login", "--restart"]),
        &mut progress,
        move |url| {
            assert_eq!(url, URL);
            browser_called.store(true, std::sync::atomic::Ordering::Relaxed);
            Err("Fake browser unavailable".into())
        },
    )
    .await
    .unwrap();
    assert_eq!(result["linked"], true);
    assert!(opened.load(std::sync::atomic::Ordering::Relaxed));
    assert!(String::from_utf8(progress).unwrap().contains("manually"));
    node.finish();
}

#[tokio::test]
async fn rejected_pending_login_gives_restart_help_without_blindly_creating_more_codes() {
    let node = Node::new(vec![
        (status_request(), Ok(waiting())),
        (status_request(), Ok(waiting())),
        (poll_request(), Err("Command center denied the request")),
        (status_request(), Ok(waiting())),
    ]);
    let error = login(
        &node.api,
        &options(&["login", "--no-browser"]),
        &mut Vec::new(),
        |_| panic!("browser must stay closed"),
    )
    .await
    .unwrap_err();
    assert!(error.contains("--restart"));
    assert!(
        !node
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r["op"] == "command_center_link")
    );
    node.finish();
}

#[tokio::test]
async fn changed_pending_request_is_not_polled_and_already_linked_login_does_not_restart() {
    let mut changed = waiting();
    changed["pending"] = json!({"user_code":"1111-2222-3333","verification_url":format!("{BASE}/connect?code=1111-2222-3333")});
    let node = Node::new(vec![
        (status_request(), Ok(waiting())),
        (status_request(), Ok(changed)),
    ]);
    let error = login(
        &node.api,
        &options(&["login", "--no-browser"]),
        &mut Vec::new(),
        |_| panic!("browser must stay closed"),
    )
    .await
    .unwrap_err();
    assert!(error.contains("changed"));
    node.finish();
    let node = Node::new(vec![(status_request(), Ok(linked()))]);
    let result = login(&node.api, &options(&["login"]), &mut Vec::new(), |_| {
        panic!("already linked must not open browser")
    })
    .await
    .unwrap();
    assert_eq!(result["linked"], true);
    node.finish();
    let node = Node::new(vec![(status_request(), Ok(linked()))]);
    let error = login(
        &node.api,
        &options(&["login", "--url", "https://another.example"]),
        &mut Vec::new(),
        |_| panic!("already linked must not open browser"),
    )
    .await
    .unwrap_err();
    assert!(error.contains("Log out"));
    node.finish();
}

#[tokio::test]
async fn timeout_stops_waiting_without_unlinking_or_starting_another_request() {
    let node = Node::new(vec![
        (status_request(), Ok(waiting())),
        (status_request(), Ok(waiting())),
        (poll_request(), Ok(json!({"state":"pending"}))),
    ]);
    let mut options = options(&["login", "--no-browser"]);
    options.timeout = Duration::from_millis(100);
    let error = bounded_login(
        &node.api,
        &options,
        &mut Vec::new(),
        |_| panic!("browser must stay closed"),
        std::future::pending(),
    )
    .await
    .unwrap_err();
    assert!(error.contains("timed out"));
    assert_eq!(node.requests.lock().unwrap().len(), 3);
    node.finish();
}

#[tokio::test]
async fn cancellation_is_prompt_and_never_unlinks_the_account() {
    let node = Node::new(vec![
        (status_request(), Ok(waiting())),
        (status_request(), Ok(waiting())),
        (poll_request(), Ok(json!({"state":"pending"}))),
    ]);
    let received = node.requests.clone();
    let cancelled = async move {
        loop {
            if received.lock().unwrap().len() >= 3 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    };
    let error = bounded_login(
        &node.api,
        &options(&["login", "--no-browser"]),
        &mut Vec::new(),
        |_| panic!("browser must stay closed"),
        cancelled,
    )
    .await
    .unwrap_err();
    assert!(error.contains("cancelled"));
    assert_eq!(node.requests.lock().unwrap().len(), 3);
    node.finish();
}

#[tokio::test]
async fn overall_deadline_interrupts_a_blocked_owner_socket_read() {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("node.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let api = Api {
        config: root.path().join("custom.toml"),
        state: root.path().into(),
    };
    let (release, hold) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&request).unwrap(),
            status_request()
        );
        // Hold the reply until the login future has already returned, proving
        // the CLI deadline does not wait for Api's27-second read timeout.
        hold.recv_timeout(Duration::from_secs(3)).unwrap();
    });
    let mut options = options(&["login", "--no-browser"]);
    options.timeout = Duration::from_millis(50);
    let started = Instant::now();
    let result = bounded_login(
        &api,
        &options,
        &mut Vec::new(),
        |_| panic!("browser must stay closed"),
        std::future::pending(),
    )
    .await;
    assert!(result.unwrap_err().contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
    server.join().unwrap();
}

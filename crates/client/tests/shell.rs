#![cfg(unix)]
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
use wispnet_client::Client;
use wispnet_core::{config::TransportConfig, identity::SecretKey};
use wispnet_registry::host;
async fn peer() -> (tempfile::TempDir, Arc<Client>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let config: TransportConfig = serde_json::from_value(
        json!({"bind_addr":"127.0.0.1:0","relay_url":"https://relay.example"}),
    )
    .unwrap();
    let client = Client::new_peer(SecretKey::generate(), dir.path().into(), config)
        .await
        .unwrap();
    (dir, client)
}
async fn input(client: &Arc<Client>, id: &Value, data: &[u8]) {
    client
        .command(&json!({"op":"shell_input","session":id,"data_b64":STANDARD.encode(data)}))
        .await
        .unwrap();
}
async fn poll(client: &Arc<Client>, id: &Value) -> Value {
    client
        .command(&json!({"op":"shell_poll","session":id}))
        .await
        .unwrap()
}
// A real terminal answers primary device attributes. In particular, modern fish
// waits for this handshake; tests must not depend on the account shell being sh.
async fn startup(client: &Arc<Client>, id: &Value) {
    let mut quiet = 0;
    let mut pending = Vec::new();
    for _ in 0..100 {
        let result = poll(client, id).await;
        let events = result["events"].as_array().unwrap();
        if events.is_empty() {
            quiet += 1;
        } else {
            quiet = 0;
        }
        for event in events {
            if let Some(encoded) = event["data_b64"].as_str() {
                pending.extend(STANDARD.decode(encoded).unwrap());
            }
        }
        let attributes = pending.windows(4).any(|s| s == b"\x1b[0c");
        let cursor = pending.windows(4).any(|s| s == b"\x1b[6n");
        if cursor {
            input(client, id, b"\x1b[1;1R").await;
        }
        if attributes {
            input(client, id, b"\x1b[?1;2c").await;
        }
        if cursor || attributes {
            pending.clear();
            quiet = 0;
        }
        if quiet >= 15 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn until_exit(client: &Arc<Client>, id: &Value) -> (Vec<u8>, u64) {
    let mut bytes = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let result = poll(client, id).await;
            for event in result["events"].as_array().unwrap() {
                if let Some(encoded) = event["data_b64"].as_str() {
                    bytes.extend(STANDARD.decode(encoded).unwrap());
                }
                if event["type"] == "exit" {
                    return event["code"].as_u64().unwrap();
                }
                assert_ne!(event["type"], "error", "{event}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let code =
        result.unwrap_or_else(|_| panic!("No exit; output {:?}", String::from_utf8_lossy(&bytes)));
    (bytes, code)
}

#[tokio::test]
async fn owner_local_bridge_is_binary_safe_reports_exit_and_rejects_bad_requests() {
    let (state, client) = peer().await;
    let own = client.endpoint.id();
    let authority = host::Authority::open(state.path(), own).unwrap();
    let shell = authority
        .services()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == "shell")
        .unwrap();
    let request = json!({"op":"shell_open","host":own,"service":"shell","incarnation":shell.incarnation,"session":"bridge","cols":80,"rows":24});
    let mut bad = request.clone();
    bad["cols"] = json!(1);
    assert!(client.command(&bad).await.is_err());
    bad = request.clone();
    bad["incarnation"] = json!("ab".repeat(32));
    assert!(client.command(&bad).await.is_err());
    let session = client.command(&request).await.unwrap();
    assert_eq!(session["path"], "local");
    let id = &session["session"];
    startup(&client, id).await;
    assert!(
        client
            .command(&json!({"op":"shell_input","session":id,"data_b64":"!bad"}))
            .await
            .is_err()
    );
    assert!(
        client
            .command(&json!({"op":"shell_input","session":id,"data_b64":"A".repeat(12*1024+1)}))
            .await
            .is_err()
    );
    assert!(
        client
            .command(&json!({"op":"shell_resize","session":id,"cols":0,"rows":24}))
            .await
            .is_err()
    );
    client
        .command(&json!({"op":"shell_resize","session":id,"cols":71,"rows":19}))
        .await
        .unwrap();
    startup(&client, id).await;
    input(
        &client,
        id,
        b"stty size; sh -c 'printf \"\\000\\377\\101\\102\"'; exit 7\n",
    )
    .await;
    let (bytes, code) = until_exit(&client, id).await;
    assert_eq!(code, 7);
    assert!(
        bytes.windows(4).any(|b| b == [0, 255, b'A', b'B']),
        "{bytes:?}"
    );
    assert!(String::from_utf8_lossy(&bytes).contains("19 71"));
    client
        .command(&json!({"op":"shell_close","session":id}))
        .await
        .unwrap();
    assert!(
        client
            .command(&json!({"op":"shell_poll","session":id}))
            .await
            .is_err()
    );
    client.endpoint.close().await;
}
#[tokio::test]
async fn remote_bridge_requires_shell_grant_and_handles_real_peer_stream() {
    let (_phone_state, phone) = peer().await;
    let (target_state, target) = peer().await;
    let own = target.endpoint.id();
    let (mut store, owner) = host::owner_store(target_state.path(), own).unwrap();
    let mut invitation = store
        .pairing_invite(
            own,
            &owner,
            "https://relay.example",
            wispnet_registry::enrollment::now(),
            300,
        )
        .unwrap();
    invitation.direct_hint = Some(target.endpoint.bound_sockets()[0]);
    phone.command(&json!({"op":"pair","host":own,"invitation":serde_json::to_string(&invitation).unwrap(),"name":"Phone"})).await.unwrap();
    store
        .host_mutate(
            own,
            &owner,
            host::Mutation::ApproveInvitation {
                expected_revision: 0,
                invitation: invitation.invitation.clone(),
                services: vec![],
            },
        )
        .unwrap();
    let shell = store
        .host_snapshot(own)
        .unwrap()
        .services
        .into_iter()
        .find(|s| s.id == "shell")
        .unwrap();
    let request = json!({"op":"shell_open","host":own,"service":"shell","incarnation":shell.incarnation,"session":"remote","cols":80,"rows":24});
    assert!(phone.command(&request).await.is_err());
    store
        .host_mutate(
            own,
            &owner,
            host::Mutation::Shell {
                expected_revision: 1,
                allow: vec![phone.endpoint.id()],
            },
        )
        .unwrap();
    let session = phone.command(&request).await.unwrap();
    let id = &session["session"];
    assert_ne!(session["path"], "local");
    startup(&phone, id).await;
    input(
        &phone,
        id,
        b"sh -c 'printf \"\\122\\105\\115\\117\\124\\105\"'; exit 3\n",
    )
    .await;
    let (bytes, code) = until_exit(&phone, id).await;
    assert_eq!(code, 3);
    assert!(String::from_utf8_lossy(&bytes).contains("REMOTE"));
    phone
        .command(&json!({"op":"shell_close","session":id}))
        .await
        .unwrap();
    phone.endpoint.close().await;
    target.endpoint.close().await;
}

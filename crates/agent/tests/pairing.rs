use std::time::Duration;
use tokio::time::timeout;
use wipsnet_agent::{pairing, tunnel};
use wipsnet_core::identity::Identity;

#[tokio::test]
async fn hello_checks_transport_pin_before_request_and_rejects_false_responses() {
    let server_key = Identity::generate();
    let client_key = Identity::generate();
    let wrong = Identity::generate();
    let server = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
        .secret_key(server_key.secret_key().clone())
        .alpns(vec![wipsnet_core::pairing::ALPN.to_vec()])
        .bind_addr("127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap())
        .unwrap()
        .bind()
        .await
        .unwrap();
    let client = tunnel::bind_client(client_key.secret_key()).await.unwrap();
    let target =
        iroh::EndpointAddr::new(server_key.endpoint_id()).with_ip_addr(server.bound_sockets()[0]);
    let (outgoing, incoming) = tokio::join!(
        client.connect(target.clone(), wipsnet_core::pairing::ALPN),
        async { server.accept().await.unwrap().await.unwrap() }
    );
    let outgoing = outgoing.unwrap();
    assert!(
        pairing::hello_on(&outgoing, wrong.endpoint_id())
            .await
            .is_err()
    );
    assert!(
        timeout(Duration::from_millis(250), incoming.accept_bi())
            .await
            .is_err()
    );
    outgoing.close(0u32.into(), b"test complete");
    let responses = [
        serde_json::json!({"version": 1, "host": wrong.endpoint_id()}),
        serde_json::json!({"version": 2, "host": server_key.endpoint_id()}),
        serde_json::json!({"version": 1, "host": server_key.endpoint_id(), "approved": true}),
    ];
    for response in responses {
        let responder = async {
            let connection = server.accept().await.unwrap().await.unwrap();
            let (mut send, mut recv) = connection.accept_bi().await.unwrap();
            let request = recv.read_to_end(260).await.unwrap();
            assert_eq!(&request[4..], br#"{"op":"hello"}"#);
            let bytes = serde_json::to_vec(&response).unwrap();
            send.write_all(&(bytes.len() as u32).to_be_bytes())
                .await
                .unwrap();
            send.write_all(&bytes).await.unwrap();
            send.finish().unwrap();
            let _ = send.stopped().await;
            connection.close(0u32.into(), b"test complete");
        };
        let (result, ()) = timeout(Duration::from_secs(3), async {
            tokio::join!(pairing::hello(&client, target.clone()), responder)
        })
        .await
        .unwrap();
        assert!(result.is_err());
    }
    client.close().await;
    server.close().await;
}

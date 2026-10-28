//! Bounded loopback HTTP transport for the account and authorization service.
use crate::Shared;
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use std::{future::Future, io, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinSet};

const MAX_CONNECTIONS: usize = 256;
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(35);

pub async fn serve_until<F: Future<Output = ()>>(
    hub: Shared,
    listener: TcpListener,
    shutdown: F,
) -> io::Result<()> {
    serve_with_slots(
        hub,
        listener,
        shutdown,
        Arc::new(Semaphore::new(MAX_CONNECTIONS)),
    )
    .await
}

async fn serve_with_slots<F: Future<Output = ()>>(
    hub: Shared,
    listener: TcpListener,
    shutdown: F,
    slots: Arc<Semaphore>,
) -> io::Result<()> {
    tokio::pin!(shutdown);
    let mut tasks = JoinSet::new();
    let result = loop {
        // Completed tasks no longer hold a connection permit. Reap them before
        // accepting more, so the JoinSet itself remains bounded as well.
        while tasks.try_join_next().is_some() {}
        tokio::select! {
            _ = &mut shutdown => break Ok(()),
            accepted = listener.accept() => {
                let (socket, _) = match accepted {
                    Ok(value) => value,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => break Err(error),
                };
                let Ok(permit) = slots.clone().try_acquire_owned() else {
                    // Do not allocate a task or queue a waiter for overload.
                    drop(socket);
                    continue;
                };
                let router = hub.router();
                tasks.spawn(async move {
                    let _permit = permit;
                    let mut builder = hyper::server::conn::http1::Builder::new();
                    builder.timer(TokioTimer::new())
                        .header_read_timeout(HEADER_TIMEOUT)
                        .max_buf_size(32768)
                        .keep_alive(false);
                    let connection = builder.serve_connection(
                        TokioIo::new(socket), TowerToHyperService::new(router),
                    );
                    // The hub serves finite authentication responses, never MCP
                    // streams. Bound slow readers as well as request uploads.
                    let _ = tokio::time::timeout(CONNECTION_TIMEOUT, connection).await;
                });
            },
            _ = tasks.join_next(), if !tasks.is_empty() => {},
        }
    };
    // Dropping the listener stops new accepts. Dropping every connection future
    // closes its socket and releases its permit before shutdown returns.
    drop(listener);
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::SocketAddr, os::unix::fs::PermissionsExt};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        sync::oneshot,
    };

    struct Fixture {
        _state: tempfile::TempDir,
        address: SocketAddr,
        slots: Arc<Semaphore>,
        shutdown: Option<oneshot::Sender<()>>,
        server: Option<tokio::task::JoinHandle<io::Result<()>>>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(server) = &self.server {
                server.abort();
            }
        }
    }
    impl Fixture {
        async fn new(limit: usize) -> Self {
            let state = tempfile::tempdir().unwrap();
            std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            let hub = crate::Hub::open(crate::Config {
                base_url: "https://cmd.wispnet.test".into(),
                state_dir: state.path().into(),
                public_domain: "wispnet.test".into(),
                github_client_id: "fixture-client".into(),
                github_client_secret: "fixture-secret".into(),
                test_mode: true,
            })
            .unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let slots = Arc::new(Semaphore::new(limit));
            let server_slots = slots.clone();
            let (shutdown, receiver) = oneshot::channel();
            let server = tokio::spawn(serve_with_slots(
                hub,
                listener,
                async {
                    let _ = receiver.await;
                },
                server_slots,
            ));
            Self {
                _state: state,
                address,
                slots,
                shutdown: Some(shutdown),
                server: Some(server),
            }
        }
        async fn wait_slots(&self, expected: usize) {
            tokio::time::timeout(Duration::from_secs(1), async {
                while self.slots.available_permits() != expected {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("connection slot state did not settle");
            // Give newly accepted connection tasks a poll to arm their timers.
            tokio::task::yield_now().await;
        }
        async fn stop(&mut self) {
            self.shutdown.take().unwrap().send(()).unwrap();
            self.server.take().unwrap().await.unwrap().unwrap();
        }
    }

    async fn closed(socket: &mut TcpStream) -> Vec<u8> {
        let mut response = Vec::new();
        let result =
            tokio::time::timeout(Duration::from_secs(1), socket.read_to_end(&mut response))
                .await
                .expect("server kept a bounded connection open");
        if let Err(error) = result {
            assert!(
                matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe
                ),
                "{error}"
            );
        }
        response
    }

    #[tokio::test(start_paused = true)]
    async fn incomplete_headers_close_at_the_header_deadline() {
        let mut fixture = Fixture::new(2).await;
        let mut socket = TcpStream::connect(fixture.address).await.unwrap();
        socket
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: cmd.wispnet.test\r\n")
            .await
            .unwrap();
        fixture.wait_slots(1).await;
        tokio::time::advance(Duration::from_secs(9)).await;
        tokio::task::yield_now().await;
        let mut byte = [0];
        assert_eq!(
            socket.try_read(&mut byte).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        tokio::time::advance(Duration::from_secs(2)).await;
        let response = closed(&mut socket).await;
        assert!(!String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200"));
        fixture.wait_slots(2).await;
        fixture.stop().await;
    }

    #[tokio::test]
    async fn overload_drops_excess_connections_slots_recover_and_shutdown_drains() {
        let mut fixture = Fixture::new(2).await;
        let first = TcpStream::connect(fixture.address).await.unwrap();
        let mut second = TcpStream::connect(fixture.address).await.unwrap();
        fixture.wait_slots(0).await;
        let mut excess = TcpStream::connect(fixture.address).await.unwrap();
        assert!(closed(&mut excess).await.is_empty());
        assert_eq!(fixture.slots.available_permits(), 0);
        drop(first);
        fixture.wait_slots(1).await;
        let mut healthy = TcpStream::connect(fixture.address).await.unwrap();
        healthy
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: cmd.wispnet.test\r\n\r\n")
            .await
            .unwrap();
        let response = String::from_utf8(closed(&mut healthy).await).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("\"ok\":true"), "{response}");
        fixture.wait_slots(1).await;
        fixture.stop().await;
        assert!(closed(&mut second).await.is_empty());
        assert_eq!(fixture.slots.available_permits(), 2);
    }
}

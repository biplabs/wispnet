//! Direct authenticated service transport. Browser access protection belongs to #10.
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, QuicTransportConfig, RecvStream, SendStream, presets},
};
use std::{
    fmt,
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    net::TcpStream,
    sync::{Semaphore, watch},
    task::{JoinHandle, JoinSet},
    time::timeout,
};
use wipsnet_core::{
    config::{ServiceConfig, TransportConfig, validate_services},
    identity::{EndpointId, SecretKey},
    protocol::{self, Status},
};

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
pub const BACKEND_TIMEOUT: Duration = Duration::from_secs(3);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_SESSION: Duration = Duration::from_secs(3600);
const BUFFER: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum TunnelError {
    Status(Status),
    Transport(&'static str),
}
impl fmt::Display for TunnelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(status) => f.write_str(status.as_str()),
            Self::Transport(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for TunnelError {}

#[derive(Clone, Default)]
pub struct ActiveCounts {
    connections: Arc<AtomicUsize>,
    streams: Arc<AtomicUsize>,
}
impl ActiveCounts {
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }
    pub fn streams(&self) -> usize {
        self.streams.load(Ordering::Relaxed)
    }
}
struct CountGuard(Arc<AtomicUsize>);
impl CountGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}
impl Drop for CountGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
struct CloseConnection(Connection);
impl Drop for CloseConnection {
    fn drop(&mut self) {
        self.0.close(0u32.into(), b"tunnel closed");
    }
}

fn quic_config(streams: usize) -> QuicTransportConfig {
    QuicTransportConfig::builder()
        .max_concurrent_bidi_streams((streams as u32).into())
        .max_concurrent_uni_streams(0u32.into())
        .stream_receive_window((64u32 * 1024).into())
        .receive_window((1024u32 * 1024).into())
        .send_window(1024 * 1024)
        .build()
}

#[derive(Clone)]
enum Services {
    Static(Arc<Vec<ServiceConfig>>),
    Host(wipsnet_registry::host::Authority),
}
impl Services {
    async fn load(&self) -> Result<Vec<ServiceConfig>, Status> {
        match self {
            Self::Static(services) => Ok(services.as_ref().clone()),
            Self::Host(host) => host.services().await.map_err(|_| Status::Denied),
        }
    }
    async fn approved(&self, peer: EndpointId) -> bool {
        match self {
            Self::Static(services) => services.iter().any(|s| s.allow.contains(&peer)),
            Self::Host(host) => host.approved(peer).await,
        }
    }
}

pub struct TunnelHost {
    endpoint: Endpoint,
    stop: watch::Sender<bool>,
    worker: Option<JoinHandle<()>>,
    counts: ActiveCounts,
}
impl TunnelHost {
    pub async fn bind(
        secret: &SecretKey,
        config: &TransportConfig,
        services: &[ServiceConfig],
    ) -> Result<Self, TunnelError> {
        config
            .validate()
            .map_err(|_| TunnelError::Transport("invalid transport limits"))?;
        validate_services(services)
            .map_err(|_| TunnelError::Transport("invalid host service configuration"))?;
        Self::bind_services(
            secret,
            config,
            Services::Static(Arc::new(services.to_vec())),
        )
        .await
    }
    pub async fn bind_host(
        secret: &SecretKey,
        config: &TransportConfig,
        authority: wipsnet_registry::host::Authority,
    ) -> Result<Self, TunnelError> {
        config
            .validate()
            .map_err(|_| TunnelError::Transport("invalid transport limits"))?;
        if authority.host() != secret.public() {
            return Err(TunnelError::Transport("host authority identity mismatch"));
        }
        Self::bind_services(secret, config, Services::Host(authority)).await
    }
    async fn bind_services(
        secret: &SecretKey,
        config: &TransportConfig,
        services: Services,
    ) -> Result<Self, TunnelError> {
        let mut alpns = vec![protocol::ALPN.to_vec()];
        if matches!(services, Services::Host(_)) {
            alpns.push(wipsnet_core::host::ALPN.to_vec());
        }
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(secret.clone())
            .clear_ip_transports()
            .bind_addr(config.bind_addr)
            .map_err(|_| TunnelError::Transport("invalid transport bind address"))?
            .alpns(alpns)
            .transport_config(quic_config(config.max_streams_per_connection))
            .bind()
            .await
            .map_err(|_| {
                TunnelError::Transport("cannot bind Iroh endpoint; check UDP address/port")
            })?;
        let counts = ActiveCounts::default();
        let (stop, receiver) = watch::channel(false);
        let worker = tokio::spawn(serve(
            endpoint.clone(),
            config.clone(),
            services,
            counts.clone(),
            receiver,
        ));
        Ok(Self {
            endpoint,
            stop,
            worker: Some(worker),
            counts,
        })
    }
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
    pub fn counts(&self) -> ActiveCounts {
        self.counts.clone()
    }
    pub async fn shutdown(mut self) {
        let _ = self.stop.send(true);
        if let Some(worker) = self.worker.take() {
            let _ = worker.await;
        }
        self.endpoint.close().await;
    }
}
impl Drop for TunnelHost {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(worker) = &self.worker {
            worker.abort();
        }
    }
}

async fn serve(
    endpoint: Endpoint,
    config: TransportConfig,
    services: Services,
    counts: ActiveCounts,
    mut stop: watch::Receiver<bool>,
) {
    let connections = Arc::new(Semaphore::new(config.max_connections));
    let streams = Arc::new(Semaphore::new(config.max_streams));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let Ok(permit) = connections.clone().try_acquire_owned() else { incoming.refuse(); continue };
                let services = services.clone();
                let streams = streams.clone();
                let counts = counts.clone();
                let max = config.max_streams_per_connection;
                // Count before spawning so it includes in-flight handshakes.
                let guard = CountGuard::new(counts.connections.clone());
                tasks.spawn(async move {
                    let _permit = permit;
                    let _guard = guard;
                    let Ok(Ok(connection)) = timeout(HANDSHAKE_TIMEOUT, incoming).await else { return };
                    // Only the completed transport handshake establishes peer authority.
                    let peer = connection.remote_id();
                    if !timeout(HANDSHAKE_TIMEOUT,services.approved(peer)).await.unwrap_or(false) {
                        connection.close((Status::Denied as u32).into(), b"access denied");
                        return;
                    }
                    serve_connection(connection, peer, services, streams, max, counts).await;
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

async fn serve_connection(
    connection: Connection,
    peer: EndpointId,
    services: Services,
    streams: Arc<Semaphore>,
    max: usize,
    counts: ActiveCounts,
) {
    let connection_control = connection.alpn() == wipsnet_core::host::ALPN;
    let _close = CloseConnection(connection.clone());
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = connection.closed() => break,
            _ = tokio::time::sleep(IDLE_TIMEOUT), if tasks.is_empty() => break,
            Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
            stream = connection.accept_bi() => {
                let Ok((mut send, mut recv)) = stream else { break };
                let permit = if tasks.len() < max { streams.clone().try_acquire_owned().ok() } else { None };
                let Some(permit) = permit else {
                    let _ = reply(&mut send, Status::Busy).await;
                    let _ = recv.stop(1u32.into());
                    continue;
                };
                let guard = CountGuard::new(counts.streams.clone());
                let services = services.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    let _guard = guard;
                    if connection_control {
                        if let Services::Host(host)=services {crate::control::serve_stream(send,recv,peer,host).await;}
                    } else {
                        serve_stream(send, recv, peer, &services).await;
                    }
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

async fn read_frame(recv: &mut RecvStream) -> Result<Vec<u8>, Status> {
    let mut length = [0u8; 2];
    recv.read_exact(&mut length)
        .await
        .map_err(|_| Status::Malformed)?;
    let length = u16::from_be_bytes(length) as usize;
    if length == 0 || length > protocol::MAX_FRAME {
        return Err(Status::Malformed);
    }
    let mut data = vec![0u8; length];
    recv.read_exact(&mut data)
        .await
        .map_err(|_| Status::Malformed)?;
    Ok(data)
}
async fn reply(send: &mut SendStream, status: Status) -> Result<(), TunnelError> {
    timeout(HANDSHAKE_TIMEOUT, async {
        send.write_all(&protocol::response(status))
            .await
            .map_err(|_| TunnelError::Transport("response stream closed"))?;
        send.finish()
            .map_err(|_| TunnelError::Transport("response stream closed"))
    })
    .await
    .map_err(|_| TunnelError::Status(Status::Timeout))?
}
async fn serve_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    peer: EndpointId,
    services: &Services,
) {
    let opened = async {
        let frame = read_frame(&mut recv).await?;
        let id = protocol::decode_request(&frame)?;
        let services = services.load().await?;
        let service = services
            .iter()
            .find(|service| service.id == id)
            .ok_or(Status::UnknownService)?;
        if !service.allow.contains(&peer) {
            return Err(Status::Denied);
        }
        // This is deliberately after parsing + authenticating + checking the host-local grant.
        timeout(BACKEND_TIMEOUT, TcpStream::connect(service.backend))
            .await
            .map_err(|_| Status::BackendUnavailable)?
            .map_err(|_| Status::BackendUnavailable)
    };
    let backend = match timeout(HANDSHAKE_TIMEOUT, opened).await {
        Ok(Ok(backend)) => backend,
        outcome => {
            let status = match outcome {
                Ok(Err(status)) => status,
                _ => Status::Timeout,
            };
            let _ = reply(&mut send, status).await;
            let _ = recv.stop(1u32.into());
            return;
        }
    };
    if !matches!(
        timeout(
            HANDSHAKE_TIMEOUT,
            send.write_all(&protocol::response(Status::Ok)),
        )
        .await,
        Ok(Ok(()))
    ) {
        return;
    }
    let mut tunnel = ServiceStream::new(send, recv);
    let (mut reader, mut writer) = backend.into_split();
    let (mut tunnel_read, mut tunnel_write) = tokio::io::split(&mut tunnel);
    let _ = timeout(MAX_SESSION, async {
        tokio::try_join!(
            pump(&mut tunnel_read, &mut writer),
            pump(&mut reader, &mut tunnel_write)
        )
    })
    .await;
}

/// Fixed per-direction buffer; timeout includes both slow reads and slow writes.
/// EOF half-closes the destination and permits the opposite direction to finish.
async fn pump(
    reader: &mut (impl AsyncRead + Unpin),
    writer: &mut (impl AsyncWrite + Unpin),
) -> io::Result<()> {
    let mut buffer = [0u8; BUFFER];
    loop {
        let n = timeout(IDLE_TIMEOUT, reader.read(&mut buffer))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "tunnel idle"))??;
        if n == 0 {
            return timeout(IDLE_TIMEOUT, writer.shutdown())
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "tunnel half-close timeout")
                })?;
        }
        timeout(IDLE_TIMEOUT, writer.write_all(&buffer[..n]))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "tunnel slow writer"))??;
    }
}

/// Dropping an unfinished stream cancels both directions. A clean EOF/shutdown
/// preserves QUIC half-close semantics and does not reset a completed direction.
pub struct ServiceStream {
    send: SendStream,
    recv: RecvStream,
    send_finished: bool,
    recv_finished: bool,
    completion: Option<Pin<Box<dyn Future<Output = io::Result<()>> + Send>>>,
}
impl ServiceStream {
    fn new(send: SendStream, recv: RecvStream) -> Self {
        Self {
            send,
            recv,
            send_finished: false,
            recv_finished: false,
            completion: None,
        }
    }
}
impl AsyncRead for ServiceStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let capacity = buf.remaining();
        let result = Pin::new(&mut self.recv).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) && capacity > 0 && buf.filled().len() == before {
            self.recv_finished = true;
        }
        result
    }
}
impl AsyncWrite for ServiceStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.send), cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.send), cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.send_finished {
            return Poll::Ready(Ok(()));
        }
        if self.completion.is_none() {
            if self.send.finish().is_err() {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "tunnel send closed",
                )));
            }
            let stopped = self.send.stopped();
            self.completion = Some(Box::pin(async move {
                match stopped.await {
                    Ok(None) => Ok(()),
                    _ => Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "tunnel send not acknowledged",
                    )),
                }
            }));
        }
        let result = self
            .completion
            .as_mut()
            .expect("completion was just initialized")
            .as_mut()
            .poll(cx);
        if result.is_ready() {
            self.completion = None;
        }
        if matches!(result, Poll::Ready(Ok(()))) {
            self.send_finished = true;
        }
        result
    }
}
impl Drop for ServiceStream {
    fn drop(&mut self) {
        if !self.send_finished {
            let _ = self.send.reset(1u32.into());
        }
        if !self.recv_finished {
            let _ = self.recv.stop(1u32.into());
        }
    }
}

pub async fn bind_client(secret: &SecretKey) -> Result<Endpoint, TunnelError> {
    Endpoint::builder(presets::Minimal)
        .secret_key(secret.clone())
        .transport_config(quic_config(8))
        .bind()
        .await
        .map_err(|_| TunnelError::Transport("cannot bind client endpoint"))
}
pub async fn connect(
    endpoint: &Endpoint,
    peer: EndpointId,
    address: std::net::SocketAddr,
) -> Result<Connection, TunnelError> {
    timeout(
        HANDSHAKE_TIMEOUT,
        endpoint.connect(
            EndpointAddr::new(peer).with_ip_addr(address),
            protocol::ALPN,
        ),
    )
    .await
    .map_err(|_| TunnelError::Status(Status::Timeout))?
    .map_err(|_| TunnelError::Transport("peer unavailable or identity/ALPN mismatch"))
}
pub async fn open_service(connection: &Connection, id: &str) -> Result<ServiceStream, TunnelError> {
    let request = protocol::request(id).map_err(TunnelError::Status)?;
    timeout(HANDSHAKE_TIMEOUT, async {
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|_| TunnelError::Transport("peer closed the connection"))?;
        send.write_all(&request)
            .await
            .map_err(|_| TunnelError::Transport("request stream closed"))?;
        let payload = read_frame(&mut recv).await.map_err(TunnelError::Status)?;
        let status = protocol::decode_response(&payload).map_err(TunnelError::Status)?;
        if status != Status::Ok {
            return Err(TunnelError::Status(status));
        }
        Ok(ServiceStream::new(send, recv))
    })
    .await
    .map_err(|_| TunnelError::Status(Status::Timeout))?
}

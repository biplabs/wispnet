use wispnet_command_center::{Config, Hub};
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Command center startup failed: {}", error.message);
        std::process::exit(1);
    }
}
async fn run() -> wispnet_command_center::Result<()> {
    let hub = Hub::open(Config::from_env()?)?;
    let port = std::env::var("WISPNET_COMMAND_PORT")
        .unwrap_or_else(|_| "8325".into())
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| wispnet_command_center::Error::new(400, "Invalid command center port"))?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|_| wispnet_command_center::Error::new(500, "Cannot bind command center"))?;
    println!("Wispnet command center listening on loopback port {port}");
    hub.serve_until(listener, async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("signal handler");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
    })
    .await
    .map_err(|_| wispnet_command_center::Error::new(500, "Command center server stopped"))
}

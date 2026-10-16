use std::{env, path::PathBuf, process::ExitCode};
use tracing_subscriber::prelude::*;
use wipsnet_agent::{identity::FileIdentityStore, tunnel};
use wipsnet_core::{
    config::Config,
    identity::{EndpointId, IdentityStore},
};

const USAGE: &str = "Wipsnet development CLI
Usage: wipsnet <check|identity-init|status|agent|registry> --config /absolute/path/config.toml
Reset: wipsnet identity-reset --config /absolute/path/config.toml --expect-id PUBLIC_ID
Tunnel: wipsnet tunnel --config /absolute/path/config.toml --peer PUBLIC_ID --address IP:UDP_PORT --service SERVICE_ID
Tunnel mode relays raw stdin/stdout. It is not a browser gateway. Enrollment is not implemented.";

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            eprintln!("wipsnet: cannot start runtime");
            return ExitCode::FAILURE;
        }
    };
    let outcome = runtime.block_on(execute());
    // Tokio stdio uses blocking workers; interrupted stdin must not keep this CLI
    // alive indefinitely. Network tasks are explicitly closed before this point.
    runtime.shutdown_timeout(std::time::Duration::from_millis(100));
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("wipsnet: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn execute() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let reset = args.first().is_some_and(|a| a == "identity-reset");
    let tunnel_mode = args.first().is_some_and(|a| a == "tunnel");
    let invalid_arguments = if tunnel_mode {
        args.len() != 9
            || args[1] != "--config"
            || args[3] != "--peer"
            || args[5] != "--address"
            || args[7] != "--service"
    } else if reset {
        args.len() != 5 || args[1] != "--config" || args[3] != "--expect-id"
    } else {
        args.len() != 3 || args[1] != "--config"
    };
    if invalid_arguments {
        return Err(USAGE.into());
    }
    let mode = args[0].to_str().ok_or(USAGE)?;
    if !matches!(
        mode,
        "check" | "identity-init" | "identity-reset" | "status" | "agent" | "registry" | "tunnel"
    ) {
        return Err(USAGE.into());
    }
    let target = if tunnel_mode {
        let peer: EndpointId = args[4]
            .to_str()
            .ok_or("invalid public endpoint ID")?
            .parse()
            .map_err(|_| "invalid public endpoint ID")?;
        let address: std::net::SocketAddr = args[6]
            .to_str()
            .ok_or("invalid peer socket address")?
            .parse()
            .map_err(|_| "invalid peer socket address")?;
        let service = args[8].to_str().ok_or("invalid service ID")?.to_owned();
        wipsnet_core::protocol::request(&service).map_err(tunnel::TunnelError::Status)?;
        Some((peer, address, service))
    } else {
        None
    };
    let config = Config::load(&PathBuf::from(&args[2]))?;
    if mode == "check" {
        println!("Configuration valid");
        return Ok(());
    }
    if matches!(mode, "identity-init" | "identity-reset" | "status") {
        if mode == "identity-init" {
            config.prepare_state()?;
        }
        let store = FileIdentityStore::new(&config.state_dir)?;
        if mode == "identity-reset" {
            let expected: EndpointId = args[4]
                .to_str()
                .ok_or("invalid public endpoint ID")?
                .parse()
                .map_err(|_| "invalid public endpoint ID")?;
            store.reset(expected)?;
            println!(
                "Identity removed. Revoke the old device before explicitly initializing and enrolling a replacement."
            );
        } else {
            let identity = if mode == "identity-init" {
                store.initialize()?
            } else {
                store.load()?
            };
            println!(
                "Device: {}\nEndpoint ID: {}",
                config.device_name,
                identity.endpoint_id()
            );
        }
        return Ok(());
    }
    // Only explicit, fixed lifecycle fields are logged; never config, args or payloads.
    let level = match config.log_level {
        wipsnet_core::config::LogLevel::Error => tracing::Level::ERROR,
        wipsnet_core::config::LogLevel::Warn => tracing::Level::WARN,
        wipsnet_core::config::LogLevel::Info => tracing::Level::INFO,
        wipsnet_core::config::LogLevel::Debug => tracing::Level::DEBUG,
    };
    // Keep third-party protocol traces out of normal diagnostics.
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(std::io::stderr)
                .with_filter(
                    tracing_subscriber::filter::Targets::new()
                        .with_target("wipsnet_agent", level)
                        .with_target("wipsnet_cli", level)
                        .with_target("wipsnet_registry", level),
                ),
        )
        .init();
    config.prepare_state()?;
    // Register signal handlers before reporting started.
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let shutdown = async move {
        tokio::select! { _ = term.recv() => {}, _ = interrupt.recv() => {} }
    };
    match mode {
        "agent" => wipsnet_agent::run(&config, shutdown).await?,
        "registry" => wipsnet_registry::run(&config, shutdown).await?,
        "tunnel" => {
            let (peer, address, service) = target.ok_or("missing tunnel target")?;
            let store = FileIdentityStore::new(&config.state_dir)?;
            let _lease = store.lease()?;
            let identity = store.load()?;
            let endpoint = tunnel::bind_client(identity.secret_key()).await?;
            let operation = async {
                let connection = tunnel::connect(&endpoint, peer, address).await?;
                let stream = tunnel::open_service(&connection, &service).await?;
                let (mut reader, mut writer) = tokio::io::split(stream);
                let mut input = tokio::io::stdin();
                let mut output = tokio::io::stdout();
                tokio::try_join!(
                    async {
                        tokio::io::copy(&mut input, &mut writer).await?;
                        tokio::io::AsyncWriteExt::shutdown(&mut writer).await
                    },
                    async {
                        tokio::io::copy(&mut reader, &mut output).await?;
                        tokio::io::AsyncWriteExt::flush(&mut output).await
                    }
                )
                .map_err(|_| tunnel::TunnelError::Transport("tunnel I/O interrupted"))?;
                Ok::<(), tunnel::TunnelError>(())
            };
            let outcome = tokio::select! {
                outcome = operation => outcome,
                _ = shutdown => Err(tunnel::TunnelError::Transport("tunnel cancelled")),
            };
            endpoint.close().await;
            outcome?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

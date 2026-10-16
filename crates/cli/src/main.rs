use std::{env, path::PathBuf, process::ExitCode};
mod retry;
use tracing_subscriber::prelude::*;
use wipsnet_agent::{diagnostics, identity::FileIdentityStore, tunnel};
use wipsnet_core::{
    config::Config,
    identity::{EndpointId, IdentityStore},
};

const USAGE: &str = "Wipsnet development CLI
Usage: wipsnet <check|identity-init|status|agent|registry|registry-init|registry-backup|registry-directory> --config /absolute/path/config.toml
Backup: wipsnet registry-backup --config /absolute/path/config.toml --output /absolute/path/new-backup.sqlite3
Enroll: wipsnet enroll --config /absolute/path/device.toml --invitation /private/invitation.json
Host: wipsnet <host-init|host-status> --config /absolute/path/host.toml
Host change: wipsnet host-mutate --config /absolute/path/host.toml --request /absolute/path/request.json
Catalog: wipsnet host-catalog --config /absolute/path/client.toml --peer PUBLIC_ID [--address IP:UDP_PORT]
Pairing bootstrap: wipsnet peer-hello --config /absolute/path/client.toml --peer PINNED_HOST_ID [--address IP:UDP_PORT]
Probe: wipsnet <peer-probe|peer-watch> --config /absolute/path/client.toml --peer PUBLIC_ID [--address IP:UDP_PORT]
Reset: wipsnet identity-reset --config /absolute/path/config.toml --expect-id PUBLIC_ID
Tunnel: wipsnet tunnel --config /absolute/path/config.toml --peer PUBLIC_ID [--address IP:UDP_PORT] --service SERVICE_ID
Tunnel mode relays raw stdin/stdout. It is not a browser gateway. Independent hosts use local approval; target-bound pairing is tracked in #17. See docs/per-host-control.md.";

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
    let host_mutate = args.first().is_some_and(|a| a == "host-mutate");
    let peer_read = args.first().is_some_and(|a| {
        matches!(
            a.to_str(),
            Some("host-catalog" | "peer-hello" | "peer-probe" | "peer-watch")
        )
    });
    let enroll = args.first().is_some_and(|a| a == "enroll");
    let backup = args.first().is_some_and(|a| a == "registry-backup");
    let reset = args.first().is_some_and(|a| a == "identity-reset");
    let tunnel_mode = args.first().is_some_and(|a| a == "tunnel");
    let invalid_arguments = if host_mutate {
        args.len() != 5 || args[1] != "--config" || args[3] != "--request"
    } else if peer_read {
        !matches!(args.len(), 5 | 7)
            || args[1] != "--config"
            || args[3] != "--peer"
            || (args.len() == 7 && args[5] != "--address")
    } else if tunnel_mode {
        !matches!(args.len(), 7 | 9)
            || args[1] != "--config"
            || args[3] != "--peer"
            || (args.len() == 9 && args[5] != "--address")
            || args[args.len() - 2] != "--service"
    } else if enroll {
        args.len() != 5 || args[1] != "--config" || args[3] != "--invitation"
    } else if backup {
        args.len() != 5 || args[1] != "--config" || args[3] != "--output"
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
        "check"
            | "identity-init"
            | "identity-reset"
            | "status"
            | "agent"
            | "registry"
            | "registry-init"
            | "registry-backup"
            | "registry-directory"
            | "enroll"
            | "host-init"
            | "host-status"
            | "host-mutate"
            | "host-catalog"
            | "peer-hello"
            | "peer-probe"
            | "peer-watch"
            | "tunnel"
    ) {
        return Err(USAGE.into());
    }
    let target = if tunnel_mode {
        let peer: EndpointId = args[4]
            .to_str()
            .ok_or("invalid public endpoint ID")?
            .parse()
            .map_err(|_| "invalid public endpoint ID")?;
        let address = if args.len() == 9 {
            Some(
                args[6]
                    .to_str()
                    .ok_or("invalid peer socket address")?
                    .parse::<std::net::SocketAddr>()
                    .map_err(|_| "invalid peer socket address")?,
            )
        } else {
            None
        };
        let service = args[args.len() - 1]
            .to_str()
            .ok_or("invalid service ID")?
            .to_owned();
        wipsnet_core::protocol::request(&service).map_err(tunnel::TunnelError::Status)?;
        Some((peer, address, service))
    } else {
        None
    };
    let backup_path = if backup {
        let path = PathBuf::from(&args[4]);
        if !path.is_absolute()
            || path.parent().is_none()
            || path
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir))
        {
            return Err("backup path must be absolute without .. components".into());
        }
        Some(path)
    } else {
        None
    };
    let config = Config::load(&PathBuf::from(&args[2]))?;
    if config.schema_version == 2
        && matches!(
            mode,
            "registry" | "registry-init" | "registry-directory" | "enroll"
        )
    {
        return Err("legacy controller commands are not available in independent host mode; pairing migration is tracked in #17".into());
    }
    if matches!(mode, "host-init" | "host-status" | "host-mutate") {
        if config.schema_version != 2 {
            return Err("host commands require schema_version=2; use config/host.toml".into());
        }
        if mode == "host-init" {
            config.prepare_state()?;
        }
        let identity_store = FileIdentityStore::new(&config.state_dir)?;
        let _lease = if mode == "host-init" {
            Some(identity_store.lease()?)
        } else {
            None
        };
        let initialized = config
            .state_dir
            .join("host-owner.token")
            .try_exists()
            .map_err(|_| "cannot inspect host credential")?
            || config
                .state_dir
                .join("host.sqlite3")
                .try_exists()
                .map_err(|_| "cannot inspect host authority")?;
        let identity = if mode == "host-init" && !initialized {
            identity_store.initialize()?
        } else {
            identity_store.load()?
        };
        if mode == "host-init" {
            wipsnet_registry::host::initialize(&config.state_dir, identity.endpoint_id())?;
            println!(
                "Independent host initialized\nEndpoint ID: {}",
                identity.endpoint_id()
            );
            return Ok(());
        }
        let (mut store, secret) =
            wipsnet_registry::host::owner_store(&config.state_dir, identity.endpoint_id())?;
        if mode == "host-status" {
            println!(
                "{}",
                serde_json::to_string_pretty(&store.host_snapshot(identity.endpoint_id())?)?
            );
        }
        if mode == "host-mutate" {
            use std::io::Read;
            let file = std::fs::File::open(PathBuf::from(&args[4]))
                .map_err(|_| "cannot read owner request")?;
            if !file.metadata()?.is_file() {
                return Err("owner request must be a regular file".into());
            }
            let mut bytes = Vec::new();
            file.take(32769)
                .read_to_end(&mut bytes)
                .map_err(|_| "cannot read owner request")?;
            if bytes.len() > 32768 {
                return Err("owner request exceeds 32 KiB".into());
            }
            let operation = serde_json::from_slice(&bytes).map_err(|_| "invalid owner request")?;
            let revision = store.host_mutate(identity.endpoint_id(), &secret, operation)?;
            println!("Host revision: {revision}");
        }
        return Ok(());
    }
    if matches!(
        mode,
        "host-catalog" | "peer-hello" | "peer-probe" | "peer-watch"
    ) {
        let peer: EndpointId = args[4]
            .to_str()
            .ok_or("invalid peer")?
            .parse()
            .map_err(|_| "invalid peer")?;
        let address = if args.len() == 7 {
            Some(
                args[6]
                    .to_str()
                    .ok_or("invalid address")?
                    .parse::<std::net::SocketAddr>()
                    .map_err(|_| "invalid address")?,
            )
        } else {
            None
        };
        let target = tunnel::peer_target(peer, config.transport.as_ref(), address)?;
        let store = FileIdentityStore::new(&config.state_dir)?;
        let _lease = store.lease()?;
        let identity = store.load()?;
        let endpoint =
            tunnel::bind_configured_client(identity.secret_key(), config.transport.as_ref())
                .await?;
        let relay_configured = config
            .transport
            .as_ref()
            .is_some_and(|c| c.relay_url.is_some());
        if mode == "peer-watch" {
            // One identity lease and endpoint for the whole watch. Only fresh,
            // bounded read-only catalog probes are repeated; never service ops.
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            let mut interrupt =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
            let mut network =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1())?;
            let mut retry = retry::Retry::default();
            loop {
                let mut report = tokio::select! {
                    _ = term.recv() => break,
                    _ = interrupt.recv() => break,
                    _ = network.recv() => {
                        // Android's native route monitor cannot deliver system
                        // callbacks. This bridge represents the app notification.
                        endpoint.network_change().await;
                        retry.reset();
                        continue;
                    }
                    result = diagnostics::probe(&endpoint, peer, target.clone(), relay_configured) => {
                        match result {
                            Ok(report) => report,
                            Err(error) => serde_json::json!({"event":"probe", "ok":false, "host":peer,
                                "client":endpoint.id(), "relay_status":diagnostics::relay_state(&endpoint, relay_configured),
                                "error":error.to_string()})
                        }
                    }
                };
                let delay = retry.completed(report["ok"] == true);
                report["retry_after_ms"] = serde_json::json!(delay.as_millis());
                println!("{report}");
                tokio::select! {
                    _ = term.recv() => break,
                    _ = interrupt.recv() => break,
                    _ = network.recv() => {
                        endpoint.network_change().await;
                        retry.reset();
                    }
                    _ = tokio::time::sleep(delay) => {}
                }
            }
            endpoint.close().await;
            return Ok(());
        }
        let result = async {
            if mode == "peer-hello" {
                let hello = wipsnet_agent::pairing::hello(&endpoint, target).await?;
                return serde_json::to_value(hello)
                    .map_err(|_| tunnel::TunnelError::Transport("invalid pairing response"));
            }
            if mode == "peer-probe" {
                return diagnostics::probe(&endpoint, peer, target, relay_configured).await;
            }
            let connection = wipsnet_agent::control::connect_target(&endpoint, target).await?;
            let catalog = wipsnet_agent::control::catalog_on(&connection, peer).await?;
            serde_json::to_value(catalog)
                .map_err(|_| tunnel::TunnelError::Transport("invalid catalog"))
        }
        .await;
        endpoint.close().await;
        println!("{}", serde_json::to_string_pretty(&result?)?);
        return Ok(());
    }
    if mode == "check" {
        println!("Configuration valid");
        return Ok(());
    }
    if matches!(mode, "registry-init" | "registry-backup") {
        if mode == "registry-init" && config.registry.is_none() {
            return Err("registry-init requires a registry section".into());
        }
        config.prepare_state()?;
        let mut store =
            wipsnet_registry::store::Store::open(&config.state_dir.join("registry.sqlite3"))?;
        if mode == "registry-init" {
            let mesh = &config
                .registry
                .as_ref()
                .ok_or("registry-init requires a registry section")?
                .mesh;
            wipsnet_registry::control::initialize(
                &mut store,
                &config.state_dir.join("owner.token"),
                mesh,
            )?;
            println!(
                "Registry initialized; owner credential is in state_dir/owner.token (keep private)"
            );
        } else {
            store.backup(backup_path.as_ref().ok_or("backup path required")?)?;
            println!("Registry backup complete");
        }
        return Ok(());
    }
    if mode == "enroll" {
        use std::io::Write;
        let controller = config
            .controller
            .as_ref()
            .ok_or("controller section required")?;
        let invitation = wipsnet_registry::control::read_invitation(&PathBuf::from(&args[4]))?;
        if wipsnet_core::config::controller_origin(&controller.origin)?.as_str()
            != invitation.controller
        {
            return Err("invitation controller differs from configured HTTPS origin".into());
        }
        config.prepare_state()?;
        let store = FileIdentityStore::new(&config.state_dir)?;
        let _lease = store.lease()?;
        let identity = store.initialize()?;
        if controller
            .credential_file
            .try_exists()
            .map_err(|_| "cannot inspect device credential")?
        {
            let secret = wipsnet_registry::control::read_token(&controller.credential_file)?;
            let client = wipsnet_registry::client::Client::configured(controller, secret)?;
            client.directory(Some(identity.endpoint_id())).await?;
            println!("Already enrolled; existing identity and credential retained");
            return Ok(());
        }
        let mut temporary =
            wipsnet_registry::control::credential_temp(&controller.credential_file)?;
        let client = wipsnet_registry::client::Client::configured(
            controller,
            zeroize::Zeroizing::new(invitation.code.clone()),
        )?;
        let mut outcome = client
            .enroll(&invitation, &config.device_name, identity.secret_key())
            .await?;
        if outcome.state == "enrolled" {
            let secret = zeroize::Zeroizing::new(
                outcome
                    .token
                    .take()
                    .ok_or("missing enrollment credential")?,
            );
            temporary
                .write_all(secret.as_bytes())
                .map_err(|_| "cannot write device credential")?;
            temporary
                .as_file()
                .sync_all()
                .map_err(|_| "cannot sync device credential")?;
            temporary
                .persist_noclobber(&controller.credential_file)
                .map_err(
                    |_| "cannot commit device credential; ask owner to revoke before retrying",
                )?;
            std::fs::File::open(
                controller
                    .credential_file
                    .parent()
                    .ok_or("credential parent required")?,
            )
            .and_then(|f| f.sync_all())
            .map_err(|_| "cannot sync credential directory")?;
            println!(
                "Enrolled device: {}\nEndpoint ID: {}",
                config.device_name,
                identity.endpoint_id()
            );
        } else {
            println!(
                "Pending owner approval\nDevice: {}\nEndpoint ID: {}\nRerun enroll with the same invitation after approval",
                config.device_name,
                identity.endpoint_id()
            );
        }
        return Ok(());
    }
    if mode == "registry-directory" {
        let controller = config
            .controller
            .as_ref()
            .ok_or("controller section required")?;
        let identity = FileIdentityStore::new(&config.state_dir)?.load()?;
        let secret = wipsnet_registry::control::read_token(&controller.credential_file)?;
        let client = wipsnet_registry::client::Client::configured(controller, secret)?;
        let directory = client.directory(Some(identity.endpoint_id())).await?;
        println!(
            "Mesh: {}\nRevision: {}\nAuthorized services: {}",
            directory.mesh,
            directory.revision,
            directory.services.len()
        );
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
            let endpoint =
                tunnel::bind_configured_client(identity.secret_key(), config.transport.as_ref())
                    .await?;
            let operation = async {
                let connection = tunnel::connect_target(
                    &endpoint,
                    tunnel::peer_target(peer, config.transport.as_ref(), address)?,
                )
                .await?;
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

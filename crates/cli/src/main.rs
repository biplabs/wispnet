use std::{env, path::PathBuf, process::ExitCode};
use wipsnet_agent::identity::FileIdentityStore;
use wipsnet_core::{
    config::Config,
    identity::{EndpointId, IdentityStore},
};

const USAGE: &str = "Wipsnet development CLI
Usage: wipsnet <check|identity-init|status|agent|registry> --config /absolute/path/config.toml
Reset: wipsnet identity-reset --config /absolute/path/config.toml --expect-id PUBLIC_ID
No networking, enrollment, or service access is implemented yet.";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match execute().await {
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
    let invalid_arguments = if reset {
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
        "check" | "identity-init" | "identity-reset" | "status" | "agent" | "registry"
    ) {
        return Err(USAGE.into());
    }
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
    tracing_subscriber::fmt()
        .json()
        .with_max_level(match config.log_level {
            wipsnet_core::config::LogLevel::Error => tracing::Level::ERROR,
            wipsnet_core::config::LogLevel::Warn => tracing::Level::WARN,
            wipsnet_core::config::LogLevel::Info => tracing::Level::INFO,
            wipsnet_core::config::LogLevel::Debug => tracing::Level::DEBUG,
        })
        .with_writer(std::io::stderr)
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
        "registry" => wipsnet_registry::run(&config, shutdown).await,
        _ => unreachable!(),
    }
    Ok(())
}

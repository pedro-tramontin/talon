//! `bk-proxy` binary entry point.

use std::process::ExitCode;

use anyhow::Context;
use bk_proxy::cli::{resolve_config_dir, Cli};
use bk_proxy::{Proxy, ProxyConfig};
use clap::Parser;
use tokio::sync::watch;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // Resolve config dir + load config BEFORE installing the signal
    // handler so a config parse error doesn't leave us holding a
    // SignalKind that never gets used.
    let config_dir = resolve_config_dir(&cli);
    let mut config = match ProxyConfig::load_from_dir(&config_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "bk-proxy: failed to load config from {}: {e:#}",
                config_dir.display()
            );
            return ExitCode::from(2);
        }
    };

    // CLI flags override the file (last-wins for the four flags we expose).
    config.listener_addr = cli.listen;
    config.max_concurrent_connections = cli.max_connections;
    // log_level from the file or default; we still want env to win if set.
    if std::env::var_os("RUST_LOG").is_none() {
        config.log_level = config.log_level.clone();
    }

    // Build the env-filter from RUST_LOG or the config.
    let filter = std::env::var("RUST_LOG")
        .ok()
        .unwrap_or_else(|| config.log_level.clone());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .try_init();

    // Signal-driven shutdown. We watch a bool so other tasks can
    // subscribe too.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let proxy = Proxy::new(config);

    // Spawn a small task that flips the shutdown bool when the OS
    // tells us to stop. Unix gets SIGINT + SIGTERM; Windows gets
    // Ctrl-C only (no portable SIGTERM equivalent in
    // `tokio::signal::windows`).
    let signal_task = tokio::spawn(async move {
        wait_for_shutdown().await;
        let _ = shutdown_tx.send(true);
    });

    let run_res = proxy.run(shutdown_rx).await.context("proxy run failed");

    // Make sure the signal task is reaped.
    let _ = signal_task.await;

    match run_res {
        Ok(()) => {
            info!("bk-proxy exited cleanly");
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!(error = %e, "bk-proxy exited with error");
            ExitCode::from(1)
        }
    }
}

/// Block until the OS tells us to shut down. Unix gets SIGINT and
/// SIGTERM; Windows gets Ctrl-C only.
async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = match signal(SignalKind::interrupt()) {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "failed to install SIGINT handler");
                return;
            }
        };
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "failed to install SIGTERM handler");
                return;
            }
        };
        tokio::select! {
            _ = sigint.recv() => info!("received SIGINT"),
            _ = sigterm.recv() => info!("received SIGTERM"),
        }
    }

    #[cfg(windows)]
    {
        // Ctrl-C is the only universally-portable shutdown signal
        // Windows exposes. `tokio::signal::windows::ctrl_break()`
        // exists for services but isn't user-friendly; the
        // overwhelming majority of Windows users will close with
        // Ctrl-C (or the window-close button, which the Tauri shell
        // in Phase 4 will translate to a graceful shutdown signal).
        match tokio::signal::ctrl_c().await {
            Ok(()) => info!("received Ctrl-C"),
            Err(e) => error!(error = %e, "failed to install Ctrl-C handler"),
        }
    }
}

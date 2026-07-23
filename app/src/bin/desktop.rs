//! Desktop binary entry point (Windows, macOS, Linux).
//!
//! ## Why this file exists separately from `lib.rs`
//!
//! `app_lib::run()` is the shared application entry; it is
//! reused by the desktop binary (this file) and, eventually,
//! by Tauri 2's mobile tooling on iOS / Android (via the
//! `#[cfg_attr(mobile, tauri::mobile_entry_point)]` attribute
//! on `run()` — see `lib.rs`). Keeping the entry in `lib.rs`
//! is what lets the same setup code drive every target.
//!
//! ## What this file does
//!
//! Parses the CLI (via `clap`) and routes to one of:
//!
//! - **`talon token`** — print the auth token (Phase 8).
//! - **`talon --browser`** — run in browser-access mode
//!   (the new `bk-server` HTTP/WS server, Phase 8).
//! - **`talon`** (no flag) — run the Tauri desktop shell
//!   (the v1 default).
//!
//! The `windows_subsystem` attribute is preserved from
//! before Phase 8; it suppresses the console window in
//! release builds on Windows.

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::path::PathBuf;
use std::sync::Arc;

use bk_server::{AuthToken, Server, WsHub};
use clap::{Parser, Subcommand};

/// Talon — solo bug-bounty toolkit. The same `talon` binary
/// drives the Tauri desktop shell AND the browser-access
/// HTTP/WS server (Phase 8). The flag picks the mode.
#[derive(Parser, Debug)]
#[command(name = "talon", version, about = "Talon — solo bug-bounty toolkit")]
struct Cli {
    /// Run in browser-access mode (headless HTTP server
    /// instead of the Tauri window).
    #[arg(long)]
    browser: bool,

    /// Override the browser-mode port (default 7331).
    #[arg(long, default_value_t = 7331)]
    port: u16,

    /// Override the browser-mode bind address (default
    /// 127.0.0.1; refuses non-loopback unless --allow-remote
    /// is also set).
    #[arg(long)]
    bind: Option<std::net::IpAddr>,

    /// Opt-in: lift the loopback bind to 0.0.0.0 and
    /// require --tls-cert + --tls-key + a valid auth token.
    #[arg(long)]
    allow_remote: bool,

    /// Path to the PEM-encoded TLS cert (required when
    /// --allow-remote is set).
    #[arg(long)]
    tls_cert: Option<PathBuf>,

    /// Path to the PEM-encoded TLS key (required when
    /// --allow-remote is set).
    #[arg(long)]
    tls_key: Option<PathBuf>,

    /// Path to the auth-token file (default
    /// `~/.config/talon/auth-token`). Auto-generated on
    /// first launch if missing.
    #[arg(long)]
    auth_token_path: Option<PathBuf>,

    /// Opt-in: announce the server via mDNS so other LAN
    /// machines can discover it. Auto-enabled when
    /// --allow-remote is set.
    #[arg(long)]
    mdns_announce: bool,

    /// Path to a project DB file (overrides the
    /// most-recently-used).
    #[arg(long)]
    project: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print the auth token to stdout (used to copy the
    /// token into a browser's Authorization header or a
    /// curl command). Reads from --auth-token-path
    /// (default `~/.config/talon/auth-token`).
    Token,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Token) => run_token(&cli),
        None if cli.browser => run_browser(&cli),
        None => run_desktop(),
    }
}

/// Print the auth token to stdout. The user can pipe it
/// into a curl command or paste it into a browser's
/// auth header. If the token file doesn't exist, generate
/// it (mirrors the first-launch flow).
fn run_token(cli: &Cli) {
    let path = cli
        .auth_token_path
        .clone()
        .unwrap_or_else(bk_server::default_auth_token_path);
    let token = match AuthToken::load(&path) {
        Ok(t) => t,
        Err(_) => {
            // First launch: generate + save.
            let t = AuthToken::generate();
            if let Err(e) = t.save(&path) {
                eprintln!("talon token: failed to write {path:?}: {e}");
                std::process::exit(1);
            }
            t
        }
    };
    println!("{}", token.to_hex());
}

/// Run the Tauri desktop shell. The v1 default.
fn run_desktop() {
    app_lib::run();
}

/// Run the browser-access HTTP/WS server. Phase 8.
fn run_browser(cli: &Cli) {
    // Initialize tracing so the bk-server logs are visible.
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    let config_dir = app_lib::default_config_dir();
    // Phase 8 uses the same `Engine` the desktop shell
    // does (the `Engine::new` constructor handles the
    // config dir). The wire-event bus is shared with the
    // WS hub.
    let engine = match bk_engine::Engine::new(config_dir.clone()) {
        Ok(e) => Arc::new(e),
        Err(err) => {
            eprintln!(
                "talon --browser: failed to create engine at {}: {err}",
                config_dir.display()
            );
            std::process::exit(1);
        }
    };
    let proxy = Arc::new(app_lib::proxy_handle::ProxyHandle::new(&config_dir));
    let ws = WsHub::new();
    let proxy_clone = proxy.clone();
    let start_proxy: bk_server::StartProxyFn = Arc::new(move |scope, m_r| {
        let proxy = proxy_clone.clone();
        Box::pin(async move {
            use bk_proxy::ProxyConfig;
            proxy
                .start_with_rules(ProxyConfig::default(), scope, m_r)
                .await
                .map_err(|e| e.to_string())?;
            let status = proxy.status();
            // Serialize the status to a JSON value.
            serde_json::to_value(&status).map_err(|e| e.to_string())
        })
    });
    let proxy_clone2 = proxy.clone();
    let stop_proxy: bk_server::StopProxyFn = Arc::new(move || {
        proxy_clone2.stop();
    });
    let proxy_clone3 = proxy.clone();
    let proxy_status: bk_server::ProxyStatusFn = Arc::new(move || {
        let s = proxy_clone3.status();
        serde_json::to_value(&s).unwrap_or(serde_json::json!({"error": "serialize"}))
    });

    let mut server = Server::new(engine, std::path::PathBuf::from("ui/dist"));
    if let Some(addr) = cli.bind {
        server = server.with_bind_addr(addr);
    }
    server = server.with_port(cli.port);
    if cli.allow_remote {
        server = server.with_allow_remote(true);
        let cert = cli
            .tls_cert
            .clone()
            .expect("--allow-remote requires --tls-cert");
        let key = cli
            .tls_key
            .clone()
            .expect("--allow-remote requires --tls-key");
        server = server.with_tls(cert, key);
        let token_path = cli
            .auth_token_path
            .clone()
            .unwrap_or_else(bk_server::default_auth_token_path);
        let token = match AuthToken::load(&token_path) {
            Ok(t) => t,
            Err(_) => {
                let t = AuthToken::generate();
                if let Err(e) = t.save(&token_path) {
                    eprintln!("talon --browser: failed to write auth token at {token_path:?}: {e}");
                    std::process::exit(1);
                }
                t
            }
        };
        server = server.with_auth_token(Arc::new(token));
    }
    if cli.mdns_announce || cli.allow_remote {
        server = server.with_mdns(true);
    }
    server = server.with_ws_hub(ws);
    server = server.with_proxy_handlers(start_proxy, stop_proxy, proxy_status);

    // Build a tokio runtime and run the server.
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("talon --browser: failed to start tokio runtime: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = rt.block_on(server.run()) {
        eprintln!("talon --browser: server error: {e}");
        std::process::exit(1);
    }
}

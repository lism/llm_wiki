//! Standalone LLM Wiki API server binary.
//!
//! Runs the same HTTP API as the desktop app (search, chat, files,
//! graph, reviews) without the Tauri shell, WebView, or tray icon.
//!
//! ```bash
//! # Build
//! cargo build --release --bin llm-wiki-api
//!
//! # Run (reads config from the same app-state.json the desktop app uses)
//! ./llm-wiki-api --data-dir ~/Library/Application\ Support/com.llm-wiki.app
//!
//! # Run with a standalone config directory
//! ./llm-wiki-api --data-dir ~/.llm-wiki --port 19828
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use llm_wiki_lib::api_context::ApiContext;
use llm_wiki_lib::api_server;

fn main() {
    let mut data_dir: Option<PathBuf> = None;
    let mut port: u16 = 19828;
    let mut host_override: Option<String> = None;

    // ── simple arg parser (no external crate needed) ──────────────
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" => {
                i += 1;
                data_dir = Some(PathBuf::from(&args[i]));
            }
            "--port" => {
                i += 1;
                port = args[i].parse().unwrap_or(19828);
            }
            "--host" => {
                i += 1;
                host_override = Some(args[i].clone());
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            other => {
                eprintln!("Unknown flag: {other}");
                print_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // Default data dir: ~/.llm-wiki or from env var
    let data_dir = data_dir.unwrap_or_else(|| {
        if let Ok(dir) = std::env::var("LLM_WIKI_DATA_DIR") {
            return PathBuf::from(dir);
        }
        dirs_fallback()
    });

    // Ensure the data directory exists
    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("Failed to create data directory {}: {e}", data_dir.display());
        std::process::exit(1);
    }

    if let Some(host) = &host_override {
        std::env::set_var("LLM_WIKI_BIND_HOST", host);
    }

    // Create a tokio runtime for async operations (search, chat, …)
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let handle = rt.handle().clone();

    let ctx = Arc::new(ApiContext::new(data_dir, None, handle));

    eprintln!("=== LLM Wiki API Server ===");
    eprintln!("Data dir:  {}", ctx.data_dir.display());
    eprintln!("Port:      {port}");
    eprintln!("Host:      {}", ctx.configured_bind_host());
    eprintln!("API token: {}", if ctx.api_token().is_some() { "configured" } else { "none" });

    // Run the event loop on the tokio runtime so the async background
    // work (SSE chat, vector search, …) has a reactor to drive.
    let _guard = rt.enter();
    api_server::start_api_server(ctx, port);

    // Block forever — start_api_server spawns a background thread
    // that owns the server loop. The main thread parks here so the
    // process doesn't exit.
    std::thread::park();
}

fn dirs_fallback() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join("Library/Application Support/com.llmwiki.app")
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".local/share/com.llmwiki.app")
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("com.llmwiki.app")
    }
}

fn print_help() {
    eprintln!(
        "\
Usage: llm-wiki-api [OPTIONS]

Options:
  --data-dir DIR   Path to the config directory containing
                   app-state.json (default: platform-specific
                   app data directory, or $LLM_WIKI_DATA_DIR)
  --port PORT      Port to listen on (default: 19828)
  --host HOST      Bind host override (also settable via
                   $LLM_WIKI_BIND_HOST)
  --help, -h       Show this help

Environment variables:
  LLM_WIKI_DATA_DIR     Default --data-dir
  LLM_WIKI_BIND_HOST    Bind host override
  LLM_WIKI_API_TOKEN    API authentication token
"
    );
}

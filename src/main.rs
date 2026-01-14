//! FGP Browser Gateway - Pure Rust browser automation via CDP.

mod browser;
mod models;
mod service;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use fgp_daemon::{cleanup_socket, FgpServer};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use crate::service::BrowserService;

const DEFAULT_SOCKET: &str = "~/.fgp/services/browser/daemon.sock";

#[derive(Parser)]
#[command(name = "browser-gateway")]
#[command(about = "FGP browser automation daemon via Chrome DevTools Protocol")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output JSON (for agent consumption)
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the browser daemon
    Start {
        /// Socket path
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,

        /// Run in foreground
        #[arg(short, long)]
        foreground: bool,

        /// Run browser in headed mode (visible)
        #[arg(long)]
        headed: bool,

        /// Use system Chrome instead of bundled Chromium
        #[arg(long)]
        channel: Option<String>,
    },

    /// Stop the browser daemon
    Stop {
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
    },

    /// Check daemon status
    Status {
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
    },

    /// Navigate to URL
    Open {
        url: String,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Get ARIA accessibility tree snapshot
    Snapshot {
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Click an element
    Click {
        /// Element selector (@e5 for ARIA ref, or CSS selector)
        selector: String,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Fill an input field
    Fill {
        /// Element selector
        selector: String,
        /// Value to fill
        value: String,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Press a key
    Press {
        /// Key to press (e.g., Enter, Tab, Escape)
        key: String,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Take a screenshot
    Screenshot {
        /// Output file path (optional, returns base64 if not specified)
        path: Option<String>,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Select an option from a dropdown
    Select {
        /// Element selector
        selector: String,
        /// Option value to select
        value: String,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Set checkbox/radio state
    Check {
        /// Element selector
        selector: String,
        /// Whether to check (true) or uncheck (false)
        #[arg(long, default_value = "true")]
        checked: bool,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Hover over an element
    Hover {
        /// Element selector
        selector: String,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Scroll to element or by amount
    Scroll {
        /// Element selector to scroll to (optional)
        selector: Option<String>,
        /// Horizontal scroll amount
        #[arg(long, default_value = "0")]
        x: i32,
        /// Vertical scroll amount
        #[arg(long, default_value = "0")]
        y: i32,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Press key with modifiers (Ctrl+A, Shift+Tab, etc.)
    PressCombo {
        /// Key to press
        key: String,
        /// Modifiers (Ctrl, Shift, Alt, Meta)
        #[arg(short, long, value_delimiter = ',')]
        modifiers: Vec<String>,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Upload a file to a file input
    Upload {
        /// Element selector
        selector: String,
        /// File path to upload
        path: String,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },

    /// Auth state management
    State {
        #[command(subcommand)]
        action: StateAction,
    },

    /// Session management for parallel requests
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
}

#[derive(Subcommand)]
enum StateAction {
    /// Save current auth state
    Save {
        name: String,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },
    /// Load saved auth state
    Load {
        name: String,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
        /// Session ID (optional)
        #[arg(long)]
        session: Option<String>,
    },
    /// List saved auth states
    List {
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// Create a new isolated session
    New {
        /// Session ID
        #[arg(long)]
        id: String,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
    },
    /// List active sessions
    List {
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
    },
    /// Close a session
    Close {
        /// Session ID to close
        #[arg(long)]
        id: String,
        #[arg(short, long, default_value = DEFAULT_SOCKET)]
        socket: String,
    },
}

/// Build params with optional session_id
fn with_session(mut params: serde_json::Value, session: Option<String>) -> serde_json::Value {
    if let Some(sid) = session {
        params.as_object_mut().unwrap().insert("session_id".to_string(), serde_json::Value::String(sid));
    }
    params
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start { socket, foreground, headed, channel: _ } => {
            cmd_start(socket, foreground, !headed)
        }
        Commands::Stop { socket } => cmd_stop(socket),
        Commands::Status { socket } => cmd_status(socket),
        Commands::Open { url, socket, session } => {
            let params = with_session(serde_json::json!({"url": url}), session);
            cmd_call_daemon(&socket, "browser.open", params, cli.json)
        }
        Commands::Snapshot { socket, session } => {
            let params = with_session(serde_json::json!({}), session);
            cmd_call_daemon(&socket, "browser.snapshot", params, cli.json)
        }
        Commands::Click { selector, socket, session } => {
            let params = with_session(serde_json::json!({"selector": selector}), session);
            cmd_call_daemon(&socket, "browser.click", params, cli.json)
        }
        Commands::Fill { selector, value, socket, session } => {
            let params = with_session(serde_json::json!({"selector": selector, "value": value}), session);
            cmd_call_daemon(&socket, "browser.fill", params, cli.json)
        }
        Commands::Press { key, socket, session } => {
            let params = with_session(serde_json::json!({"key": key}), session);
            cmd_call_daemon(&socket, "browser.press", params, cli.json)
        }
        Commands::Screenshot { path, socket, session } => {
            let base = match path {
                Some(p) => serde_json::json!({"path": p}),
                None => serde_json::json!({}),
            };
            let params = with_session(base, session);
            cmd_call_daemon(&socket, "browser.screenshot", params, cli.json)
        }
        Commands::Select { selector, value, socket, session } => {
            let params = with_session(serde_json::json!({"selector": selector, "value": value}), session);
            cmd_call_daemon(&socket, "browser.select", params, cli.json)
        }
        Commands::Check { selector, checked, socket, session } => {
            let params = with_session(serde_json::json!({"selector": selector, "checked": checked}), session);
            cmd_call_daemon(&socket, "browser.check", params, cli.json)
        }
        Commands::Hover { selector, socket, session } => {
            let params = with_session(serde_json::json!({"selector": selector}), session);
            cmd_call_daemon(&socket, "browser.hover", params, cli.json)
        }
        Commands::Scroll { selector, x, y, socket, session } => {
            let mut base = serde_json::json!({"x": x, "y": y});
            if let Some(sel) = selector {
                base.as_object_mut().unwrap().insert("selector".to_string(), serde_json::Value::String(sel));
            }
            let params = with_session(base, session);
            cmd_call_daemon(&socket, "browser.scroll", params, cli.json)
        }
        Commands::PressCombo { key, modifiers, socket, session } => {
            let params = with_session(serde_json::json!({"key": key, "modifiers": modifiers}), session);
            cmd_call_daemon(&socket, "browser.press_combo", params, cli.json)
        }
        Commands::Upload { selector, path, socket, session } => {
            let params = with_session(serde_json::json!({"selector": selector, "path": path}), session);
            cmd_call_daemon(&socket, "browser.upload", params, cli.json)
        }
        Commands::State { action } => match action {
            StateAction::Save { name, socket, session } => {
                let params = with_session(serde_json::json!({"name": name}), session);
                cmd_call_daemon(&socket, "browser.state.save", params, cli.json)
            }
            StateAction::Load { name, socket, session } => {
                let params = with_session(serde_json::json!({"name": name}), session);
                cmd_call_daemon(&socket, "browser.state.load", params, cli.json)
            }
            StateAction::List { socket } => {
                cmd_call_daemon(&socket, "browser.state.list", serde_json::json!({}), cli.json)
            }
        },
        Commands::Session { action } => match action {
            SessionAction::New { id, socket } => {
                cmd_call_daemon(&socket, "browser.session.new", serde_json::json!({"id": id}), cli.json)
            }
            SessionAction::List { socket } => {
                cmd_call_daemon(&socket, "browser.session.list", serde_json::json!({}), cli.json)
            }
            SessionAction::Close { id, socket } => {
                cmd_call_daemon(&socket, "browser.session.close", serde_json::json!({"id": id}), cli.json)
            }
        },
    }
}

fn cmd_start(socket: String, foreground: bool, headless: bool) -> Result<()> {
    let socket_path = shellexpand::tilde(&socket).to_string();

    // Create parent directory
    if let Some(parent) = Path::new(&socket_path).parent() {
        std::fs::create_dir_all(parent).context("Failed to create socket directory")?;
    }

    let pid_file = format!("{}.pid", socket_path);

    println!("Starting browser-gateway daemon...");
    println!("Socket: {}", socket_path);
    println!("Mode: {}", if headless { "headless" } else { "headed" });

    if foreground {
        tracing_subscriber::fmt()
            .with_env_filter("fgp_browser=debug,fgp_daemon=debug,chromiumoxide=warn")
            .init();

        let service = BrowserService::new(headless)
            .context("Failed to create BrowserService")?;
        let server = FgpServer::new(service, &socket_path)
            .context("Failed to create FGP server")?;
        server.serve().context("Server error")?;
    } else {
        use daemonize::Daemonize;

        let daemonize = Daemonize::new()
            .pid_file(&pid_file)
            .working_directory("/tmp");

        match daemonize.start() {
            Ok(_) => {
                tracing_subscriber::fmt()
                    .with_env_filter("fgp_browser=debug,fgp_daemon=debug,chromiumoxide=warn")
                    .init();

                let service = BrowserService::new(headless)
                    .context("Failed to create BrowserService")?;
                let server = FgpServer::new(service, &socket_path)
                    .context("Failed to create FGP server")?;
                server.serve().context("Server error")?;
            }
            Err(e) => {
                eprintln!("Failed to daemonize: {}", e);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

fn cmd_stop(socket: String) -> Result<()> {
    let socket_path = shellexpand::tilde(&socket).to_string();
    let pid_file = format!("{}.pid", socket_path);

    let pid_str = std::fs::read_to_string(&pid_file)
        .context("Failed to read PID file - daemon may not be running")?;
    let pid: i32 = pid_str.trim().parse().context("Invalid PID in file")?;

    println!("Stopping browser-gateway daemon (PID: {})...", pid);

    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }

    std::thread::sleep(std::time::Duration::from_millis(500));

    let _ = cleanup_socket(&socket_path, Some(Path::new(&pid_file)));
    let _ = std::fs::remove_file(&pid_file);

    println!("Daemon stopped.");
    Ok(())
}

fn cmd_status(socket: String) -> Result<()> {
    let socket_path = shellexpand::tilde(&socket).to_string();

    if !Path::new(&socket_path).exists() {
        println!("Status: NOT RUNNING");
        println!("Socket {} does not exist", socket_path);
        return Ok(());
    }

    match UnixStream::connect(&socket_path) {
        Ok(mut stream) => {
            let request = r#"{"id":"status","v":1,"method":"health","params":{}}"#;
            writeln!(stream, "{}", request)?;
            stream.flush()?;

            let mut reader = BufReader::new(stream);
            let mut response = String::new();
            reader.read_line(&mut response)?;

            println!("Status: RUNNING");
            println!("Socket: {}", socket_path);
            println!("Health: {}", response.trim());
        }
        Err(e) => {
            println!("Status: NOT RESPONDING");
            println!("Socket exists but connection failed: {}", e);
        }
    }

    Ok(())
}

fn cmd_call_daemon(socket: &str, method: &str, params: serde_json::Value, json_output: bool) -> Result<()> {
    let socket_path = shellexpand::tilde(socket).to_string();

    let mut stream = UnixStream::connect(&socket_path)
        .context("Failed to connect to daemon. Is it running? Try: browser-gateway start")?;

    let request = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "v": 1,
        "method": method,
        "params": params,
    });

    writeln!(stream, "{}", request)?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;

    if json_output {
        println!("{}", response.trim());
    } else {
        // Pretty print for humans
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&response) {
            if let Some(result) = parsed.get("result") {
                println!("{}", serde_json::to_string_pretty(result)?);
            } else if let Some(error) = parsed.get("error") {
                eprintln!("Error: {}", error);
                std::process::exit(1);
            } else {
                println!("{}", serde_json::to_string_pretty(&parsed)?);
            }
        } else {
            println!("{}", response.trim());
        }
    }

    Ok(())
}

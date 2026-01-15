//! Browser Gateway Quickstart Example
//!
//! This example demonstrates basic browser automation with FGP.
//!
//! # Prerequisites
//! - Chrome/Chromium installed
//! - browser-gateway daemon running: `browser-gateway start`
//!
//! # Running
//! ```bash
//! cargo run --example quickstart
//! ```

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

const SOCKET_PATH: &str = "~/.fgp/services/browser/daemon.sock";

fn expand_path(path: &str) -> String {
    if path.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return path.replacen("~", home.to_string_lossy().as_ref(), 1);
        }
    }
    path.to_string()
}

fn call(method: &str, params: &str) -> Result<String, Box<dyn std::error::Error>> {
    let socket_path = expand_path(SOCKET_PATH);
    let mut stream = UnixStream::connect(&socket_path)?;

    let request = format!(
        r#"{{"id":"1","v":1,"method":"{}","params":{}}}"#,
        method, params
    );
    writeln!(stream, "{}", request)?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;

    Ok(response)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("FGP Browser Gateway Quickstart");
    println!("==============================\n");

    // Step 1: Navigate to a page
    println!("1. Navigating to example.com...");
    let response = call("browser.open", r#"{"url":"https://example.com"}"#)?;
    println!("   Response: {}\n", response.trim());

    // Step 2: Get the page snapshot (ARIA tree)
    println!("2. Getting page snapshot...");
    let response = call("browser.snapshot", "{}")?;
    println!("   Snapshot received (truncated): {}...\n", &response[..response.len().min(200)]);

    // Step 3: Check health
    println!("3. Checking daemon health...");
    let response = call("health", "{}")?;
    println!("   Health: {}\n", response.trim());

    println!("Done! The browser daemon is working correctly.");
    println!("\nTry more commands:");
    println!("  browser-gateway click \"a\"           # Click first link");
    println!("  browser-gateway fill \"input\" \"text\" # Fill input field");
    println!("  browser-gateway screenshot /tmp/page.png");

    Ok(())
}

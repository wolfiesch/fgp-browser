//! Extension Bridge - WebSocket server for Chrome extension communication
//!
//! Enables the FGP Chrome extension to communicate with the daemon,
//! providing access to Chrome Extension APIs (tab groups, cookies, etc.)
//!
//! # Architecture
//!
//! ```text
//! Chrome Extension <--WebSocket--> ExtensionBridge <--Channel--> BrowserService
//! ```
//!
//! # CHANGELOG (recent first, max 5 entries)
//! 01/15/2026 - Added sync call_blocking() for service integration (Claude)
//! 01/15/2026 - Initial implementation (Claude)

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_tungstenite::{accept_async, tungstenite::Message};

const DEFAULT_WS_PORT: u16 = 9223;

/// Request from daemon to extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionRequest {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
}

/// Response from extension to daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionResponse {
    pub id: String,
    #[serde(default)]
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Extension connection state
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connected,
}

/// Methods that should be routed to the extension (not CDP)
/// These use `browser.` prefix to pass FGP namespace validation,
/// but the actual Chrome Extension API method is extracted when calling.
pub const EXTENSION_METHODS: &[&str] = &[
    // Tab Management (for grouping workflow)
    "browser.tabs.create",
    "browser.tabs.query",
    // Tab Groups (CDP can't do this!)
    "browser.tabs.group",
    "browser.tabs.ungroup",
    "browser.tabGroups.update",
    "browser.tabGroups.query",
    "browser.tabGroups.collapse",
    // Cookies (cleaner API than CDP)
    "browser.cookies.get",
    "browser.cookies.getAll",
    "browser.cookies.set",
    // Notifications (extension-only)
    "browser.notifications.create",
    // Storage (extension-only)
    "browser.storage.get",
    "browser.storage.set",
    // Utility
    "browser.version",
];

/// Extract the Chrome Extension API method from a browser.* method
/// e.g., "browser.tabs.group" -> "tabs.group"
pub fn extension_method_name(method: &str) -> &str {
    method.strip_prefix("browser.").unwrap_or(method)
}

/// Check if a method should be handled by the extension
pub fn is_extension_method(method: &str) -> bool {
    EXTENSION_METHODS.iter().any(|m| *m == method)
}

/// Extension Bridge manages WebSocket connection to Chrome extension
pub struct ExtensionBridge {
    /// Current connection state
    state: Arc<RwLock<ConnectionState>>,
    /// Atomic flag for sync access to connection state
    connected: Arc<AtomicBool>,
    /// Channel to send requests to extension
    request_tx: broadcast::Sender<ExtensionRequest>,
    /// Channel to receive responses from extension
    response_tx: mpsc::Sender<ExtensionResponse>,
    response_rx: Arc<RwLock<mpsc::Receiver<ExtensionResponse>>>,
    /// Pending requests waiting for responses
    pending: Arc<RwLock<HashMap<String, tokio::sync::oneshot::Sender<ExtensionResponse>>>>,
    /// WebSocket port
    port: u16,
}

impl ExtensionBridge {
    /// Create a new extension bridge
    pub fn new(port: Option<u16>) -> Self {
        let (request_tx, _) = broadcast::channel(100);
        let (response_tx, response_rx) = mpsc::channel(100);

        Self {
            state: Arc::new(RwLock::new(ConnectionState::Disconnected)),
            connected: Arc::new(AtomicBool::new(false)),
            request_tx,
            response_tx,
            response_rx: Arc::new(RwLock::new(response_rx)),
            pending: Arc::new(RwLock::new(HashMap::new())),
            port: port.unwrap_or(DEFAULT_WS_PORT),
        }
    }

    /// Start the WebSocket server
    pub async fn start(&self) -> Result<()> {
        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr)
            .await
            .with_context(|| format!("Failed to bind WebSocket server to {}", addr))?;

        tracing::info!("Extension bridge listening on ws://{}", addr);

        let state = self.state.clone();
        let connected = self.connected.clone();
        let request_tx = self.request_tx.clone();
        let response_tx = self.response_tx.clone();
        let pending = self.pending.clone();

        // Spawn connection acceptor
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, peer)) => {
                        tracing::info!("Extension connected from {}", peer);
                        let state = state.clone();
                        let connected = connected.clone();
                        let request_rx = request_tx.subscribe();
                        let response_tx = response_tx.clone();
                        let pending = pending.clone();

                        tokio::spawn(async move {
                            if let Err(e) =
                                handle_connection(stream, state, connected, request_rx, response_tx, pending)
                                    .await
                            {
                                tracing::warn!("Extension connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        tracing::error!("Failed to accept connection: {}", e);
                    }
                }
            }
        });

        // Spawn response router
        let pending = self.pending.clone();
        let response_rx = self.response_rx.clone();
        tokio::spawn(async move {
            let mut rx = response_rx.write().await;
            while let Some(response) = rx.recv().await {
                let mut pending = pending.write().await;
                if let Some(tx) = pending.remove(&response.id) {
                    let _ = tx.send(response);
                }
            }
        });

        Ok(())
    }

    /// Check if extension is connected
    pub async fn is_connected(&self) -> bool {
        *self.state.read().await == ConnectionState::Connected
    }

    /// Blocking version of is_connected() for use from synchronous code
    /// Uses atomic bool for lock-free access
    pub fn is_connected_blocking(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Send a request to the extension and wait for response
    pub async fn call(
        &self,
        method: &str,
        params: HashMap<String, serde_json::Value>,
    ) -> Result<ExtensionResponse> {
        if !self.is_connected().await {
            anyhow::bail!("Extension not connected");
        }

        let id = uuid::Uuid::new_v4().to_string();
        let request = ExtensionRequest {
            id: id.clone(),
            method: method.to_string(),
            params,
        };

        // Create oneshot channel for response
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = self.pending.write().await;
            pending.insert(id.clone(), tx);
        }

        // Send request to extension
        self.request_tx
            .send(request)
            .map_err(|_| anyhow::anyhow!("Failed to send request to extension"))?;

        // Wait for response with timeout
        let response = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .context("Extension request timed out")?
            .context("Response channel closed")?;

        Ok(response)
    }

    /// Get connection state for status reporting
    pub async fn connection_state(&self) -> ConnectionState {
        self.state.read().await.clone()
    }

    /// Get the WebSocket port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Blocking version of call() for use from synchronous code.
    /// Uses tokio's Handle::block_on when called from outside async context.
    pub fn call_blocking(
        &self,
        method: &str,
        params: HashMap<String, serde_json::Value>,
    ) -> Result<ExtensionResponse> {
        // Try to get current tokio handle, otherwise create a new runtime
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                // We're in an async context, use block_in_place
                tokio::task::block_in_place(|| handle.block_on(self.call(method, params)))
            }
            Err(_) => {
                // Not in async context, create a temporary runtime
                let rt = tokio::runtime::Runtime::new()
                    .context("Failed to create tokio runtime for blocking call")?;
                rt.block_on(self.call(method, params))
            }
        }
    }

    /// Convert ExtensionResponse to serde_json::Value for FGP protocol
    pub fn response_to_value(response: ExtensionResponse) -> Result<serde_json::Value> {
        if response.ok {
            Ok(response.result.unwrap_or(serde_json::Value::Null))
        } else {
            Err(anyhow::anyhow!(
                "Extension error: {}",
                response.error.unwrap_or_else(|| "Unknown error".to_string())
            ))
        }
    }
}

/// Handle a single WebSocket connection from the extension
async fn handle_connection(
    stream: TcpStream,
    state: Arc<RwLock<ConnectionState>>,
    connected: Arc<AtomicBool>,
    mut request_rx: broadcast::Receiver<ExtensionRequest>,
    response_tx: mpsc::Sender<ExtensionResponse>,
    _pending: Arc<RwLock<HashMap<String, tokio::sync::oneshot::Sender<ExtensionResponse>>>>,
) -> Result<()> {
    let ws_stream = accept_async(stream)
        .await
        .context("Failed to accept WebSocket connection")?;

    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Mark as connected (both async state and atomic flag)
    *state.write().await = ConnectionState::Connected;
    connected.store(true, Ordering::SeqCst);
    tracing::info!("Extension WebSocket connected");

    // Handle incoming messages from extension
    let response_tx_clone = response_tx.clone();
    let read_handle = tokio::spawn(async move {
        while let Some(msg) = ws_read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    tracing::debug!("Received from extension: {}", text);
                    match serde_json::from_str::<ExtensionResponse>(&text) {
                        Ok(response) => {
                            if let Err(e) = response_tx_clone.send(response).await {
                                tracing::error!("Failed to forward response: {}", e);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse extension message: {}", e);
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    tracing::info!("Extension closed connection");
                    break;
                }
                Ok(Message::Ping(data)) => {
                    tracing::debug!("Ping from extension");
                    // Pong is handled automatically by tungstenite
                    let _ = data;
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!("WebSocket read error: {}", e);
                    break;
                }
            }
        }
    });

    // Forward requests to extension
    let write_handle = tokio::spawn(async move {
        loop {
            match request_rx.recv().await {
                Ok(request) => {
                    let json = serde_json::to_string(&request).unwrap();
                    tracing::debug!("Sending to extension: {}", json);
                    if let Err(e) = ws_write.send(Message::Text(json)).await {
                        tracing::error!("Failed to send to extension: {}", e);
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    // Wait for either task to complete
    tokio::select! {
        _ = read_handle => {},
        _ = write_handle => {},
    }

    // Mark as disconnected (both async state and atomic flag)
    *state.write().await = ConnectionState::Disconnected;
    connected.store(false, Ordering::SeqCst);
    tracing::info!("Extension WebSocket disconnected");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_extension_method() {
        assert!(is_extension_method("browser.tabs.group"));
        assert!(is_extension_method("browser.tabGroups.update"));
        assert!(is_extension_method("browser.cookies.getAll"));
        assert!(!is_extension_method("browser.open"));
        assert!(!is_extension_method("browser.snapshot"));
    }

    #[test]
    fn test_extension_method_name() {
        assert_eq!(extension_method_name("browser.tabs.group"), "tabs.group");
        assert_eq!(extension_method_name("browser.cookies.getAll"), "cookies.getAll");
        assert_eq!(extension_method_name("tabs.group"), "tabs.group"); // Already stripped
    }
}

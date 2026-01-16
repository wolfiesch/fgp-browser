//! BrowserService implementing FgpService trait.
//!
//! Supports session-based browser automation for parallel requests.
//! Each session has isolated context (cookies, localStorage, cache).
//!
//! # Connection Modes
//!
//! - **Launch mode** (`new()`): Spawns a new Chrome instance with isolated profile
//! - **Connect mode** (`new_connect()`): Attaches to existing Chrome with user's sessions
//!
//! # CHANGELOG (recent first, max 5 entries)
//! 01/15/2026 - Added extension bridge routing for Chrome Extension API methods (Claude)
//! 01/15/2026 - Added connect mode for user's Chrome sessions (Claude)
//! 01/15/2026 - Added rich JSON Schema definitions for all methods (Claude)
//! 01/14/2026 - Initial implementation (Claude)

use anyhow::{Context, Result};
use chrono::Utc;
use fgp_daemon::schema::SchemaBuilder;
use fgp_daemon::service::MethodInfo;
use fgp_daemon::FgpService;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::RwLock;

use crate::browser::BrowserClient;
use crate::extension_bridge::{is_extension_method, ExtensionBridge};
use crate::models::*;

/// Browser automation service.
pub struct BrowserService {
    runtime: Runtime,
    client: Arc<RwLock<Option<Arc<BrowserClient>>>>,
    user_data_dir: PathBuf,
    auth_dir: PathBuf,
    headless: bool,
    /// If Some, connect to existing Chrome instead of launching
    connect_url: Option<String>,
    /// Optional extension bridge for Chrome Extension API methods
    extension_bridge: Option<Arc<ExtensionBridge>>,
}

impl BrowserService {
    /// Create a new browser service with pre-warmed browser.
    pub fn new(headless: bool) -> Result<Self> {
        let runtime = Runtime::new().context("Failed to create tokio runtime")?;

        let base_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".fgp")
            .join("services")
            .join("browser");

        let user_data_dir = base_dir.join("user-data");
        let auth_dir = base_dir.join("auth");

        // Create directories
        std::fs::create_dir_all(&user_data_dir)?;
        std::fs::create_dir_all(&auth_dir)?;

        // Pre-warm browser for instant response on first request
        let client = runtime.block_on(async {
            tracing::info!("Pre-warming browser...");
            BrowserClient::new(user_data_dir.clone(), headless).await
        })?;

        tracing::info!("Browser pre-warmed and ready");

        Ok(Self {
            runtime,
            client: Arc::new(RwLock::new(Some(Arc::new(client)))),
            user_data_dir,
            auth_dir,
            headless,
            connect_url: None,
            extension_bridge: None,
        })
    }

    /// Set the extension bridge for routing extension methods
    pub fn with_extension_bridge(mut self, bridge: Arc<ExtensionBridge>) -> Self {
        self.extension_bridge = Some(bridge);
        self
    }

    /// Create a browser service that connects to user's existing Chrome.
    ///
    /// This mode attaches to a Chrome instance running with `--remote-debugging-port`.
    /// All user sessions, cookies, and localStorage are preserved.
    ///
    /// # Arguments
    /// * `connect_url` - Chrome debugging URL (e.g., "http://localhost:9222")
    pub fn new_connect(connect_url: &str) -> Result<Self> {
        let runtime = Runtime::new().context("Failed to create tokio runtime")?;

        let base_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".fgp")
            .join("services")
            .join("browser");

        let user_data_dir = base_dir.join("user-data");
        let auth_dir = base_dir.join("auth");

        // Create directories (for auth state storage)
        std::fs::create_dir_all(&auth_dir)?;

        // Connect to existing Chrome
        let url = connect_url.to_string();
        let client = runtime.block_on(async {
            tracing::info!("Connecting to user's Chrome at: {}", url);
            BrowserClient::connect(&url).await
        })?;

        tracing::info!("Connected to user's Chrome - sessions available!");

        Ok(Self {
            runtime,
            client: Arc::new(RwLock::new(Some(Arc::new(client)))),
            user_data_dir,
            auth_dir,
            headless: false, // User's browser is always headed
            connect_url: Some(connect_url.to_string()),
            extension_bridge: None,
        })
    }

    // Note: get_client() was removed - we now use session-based approach
    // where each handler directly accesses the client via the RwLock.

    /// Extract session_id from params (optional).
    fn get_session_id(params: &HashMap<String, Value>) -> Option<String> {
        params
            .get("session_id")
            .or_else(|| params.get("session"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    async fn get_or_init_client(
        client: &Arc<RwLock<Option<Arc<BrowserClient>>>>,
        user_data_dir: &Path,
        headless: bool,
        connect_url: Option<&str>,
    ) -> Result<Arc<BrowserClient>> {
        if let Some(existing) = client.read().await.as_ref() {
            return Ok(Arc::clone(existing));
        }

        let mut client_lock = client.write().await;
        if client_lock.is_none() {
            let new_client = if let Some(url) = connect_url {
                // Connect mode: attach to existing Chrome
                BrowserClient::connect(url).await?
            } else {
                // Launch mode: spawn new Chrome
                BrowserClient::new(user_data_dir.to_path_buf(), headless).await?
            };
            *client_lock = Some(Arc::new(new_client));
        }

        client_lock
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| anyhow::anyhow!("Failed to get browser client"))
    }

    fn handle_open(&self, params: HashMap<String, Value>) -> Result<Value> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .context("Missing 'url' parameter")?;
        let session_id = Self::get_session_id(&params);

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();

        let result = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client.navigate(url, session_id.as_deref()).await
        })?;

        Ok(serde_json::to_value(result)?)
    }

    fn handle_snapshot(&self, params: HashMap<String, Value>) -> Result<Value> {
        let session_id = Self::get_session_id(&params);
        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();

        let result = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client.snapshot(session_id.as_deref()).await
        })?;

        Ok(serde_json::to_value(result)?)
    }

    fn handle_screenshot(&self, params: HashMap<String, Value>) -> Result<Value> {
        let path = params.get("path").and_then(|v| v.as_str());
        let session_id = Self::get_session_id(&params);

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();

        let result = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client.screenshot(path, session_id.as_deref()).await
        })?;

        Ok(serde_json::to_value(result)?)
    }

    fn handle_click(&self, params: HashMap<String, Value>) -> Result<Value> {
        let selector = params
            .get("selector")
            .and_then(|v| v.as_str())
            .context("Missing 'selector' parameter")?;
        let session_id = Self::get_session_id(&params);

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();
        let selector = selector.to_string();

        let result = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client.click(&selector, session_id.as_deref()).await
        })?;

        Ok(serde_json::to_value(result)?)
    }

    fn handle_fill(&self, params: HashMap<String, Value>) -> Result<Value> {
        let selector = params
            .get("selector")
            .and_then(|v| v.as_str())
            .context("Missing 'selector' parameter")?;
        let value = params
            .get("value")
            .and_then(|v| v.as_str())
            .context("Missing 'value' parameter")?;
        let session_id = Self::get_session_id(&params);

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();
        let selector = selector.to_string();
        let value = value.to_string();

        let result = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client
                .fill(&selector, &value, session_id.as_deref())
                .await
        })?;

        Ok(serde_json::to_value(result)?)
    }

    fn handle_press(&self, params: HashMap<String, Value>) -> Result<Value> {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .context("Missing 'key' parameter")?;
        let session_id = Self::get_session_id(&params);

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();
        let key = key.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client.press(&key, session_id.as_deref()).await
        })?;

        Ok(serde_json::json!({"success": true}))
    }

    fn handle_state_save(&self, params: HashMap<String, Value>) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .context("Missing 'name' parameter")?;
        let session_id = Self::get_session_id(&params);

        let state_path = self.auth_dir.join(format!("{}.json", name));
        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();

        let state = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            let cookies = browser_client.get_cookies(session_id.as_deref()).await?;
            let local_storage = browser_client
                .get_local_storage(session_id.as_deref())
                .await?;
            Ok::<AuthState, anyhow::Error>(AuthState {
                cookies,
                local_storage,
                saved_at: Utc::now().to_rfc3339(),
            })
        })?;

        let serialized = serde_json::to_vec_pretty(&state)?;
        std::fs::write(&state_path, serialized)?;

        Ok(serde_json::json!({
            "success": true,
            "path": state_path.to_string_lossy()
        }))
    }

    fn handle_state_load(&self, params: HashMap<String, Value>) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .context("Missing 'name' parameter")?;
        let session_id = Self::get_session_id(&params);

        let state_path = self.auth_dir.join(format!("{}.json", name));

        if !state_path.exists() {
            anyhow::bail!("State '{}' not found", name);
        }

        let state_bytes = std::fs::read(&state_path)?;
        let state: AuthState = serde_json::from_slice(&state_bytes)?;

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client
                .set_cookies(&state.cookies, session_id.as_deref())
                .await?;
            browser_client
                .set_local_storage(&state.local_storage, session_id.as_deref())
                .await?;
            Ok::<(), anyhow::Error>(())
        })?;

        Ok(serde_json::json!({
            "success": true,
            "name": name
        }))
    }

    fn handle_state_list(&self, _params: HashMap<String, Value>) -> Result<Value> {
        let mut states = Vec::new();

        if self.auth_dir.exists() {
            for entry in std::fs::read_dir(&self.auth_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Some(stem) = path.file_stem() {
                        let name = stem.to_string_lossy().to_string();
                        let mut domains = Vec::new();
                        let mut saved_at = String::new();

                        if let Ok(contents) = std::fs::read(&path) {
                            if let Ok(state) = serde_json::from_slice::<AuthState>(&contents) {
                                let mut seen = std::collections::HashSet::new();
                                for cookie in state.cookies {
                                    if seen.insert(cookie.domain.clone()) {
                                        domains.push(cookie.domain);
                                    }
                                }
                                saved_at = state.saved_at;
                            }
                        }

                        states.push(SavedState {
                            name,
                            domains,
                            saved_at,
                        });
                    }
                }
            }
        }

        Ok(serde_json::to_value(states)?)
    }

    fn handle_health(&self, _params: HashMap<String, Value>) -> Result<Value> {
        let client = self.client.clone();

        let healthy = self.runtime.block_on(async {
            let client_lock = client.read().await;
            if let Some(ref browser_client) = *client_lock {
                browser_client.health_check().await.unwrap_or(false)
            } else {
                true // No browser yet is OK
            }
        });

        Ok(serde_json::json!({
            "healthy": healthy,
            "service": "browser",
            "version": env!("CARGO_PKG_VERSION")
        }))
    }

    // =========================================================================
    // SESSION MANAGEMENT HANDLERS
    // =========================================================================

    fn handle_session_new(&self, params: HashMap<String, Value>) -> Result<Value> {
        let session_id = params
            .get("id")
            .or_else(|| params.get("session_id"))
            .and_then(|v| v.as_str())
            .context("Missing 'id' parameter")?;

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();

        let id = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client.create_session(session_id).await
        })?;

        Ok(serde_json::json!({
            "success": true,
            "session_id": id
        }))
    }

    fn handle_session_list(&self, _params: HashMap<String, Value>) -> Result<Value> {
        let client = self.client.clone();

        let sessions = self.runtime.block_on(async {
            let client_lock = client.read().await;
            if let Some(ref browser_client) = *client_lock {
                browser_client.list_sessions().await
            } else {
                vec![]
            }
        });

        Ok(serde_json::json!({
            "sessions": sessions
        }))
    }

    fn handle_session_close(&self, params: HashMap<String, Value>) -> Result<Value> {
        let session_id = params
            .get("id")
            .or_else(|| params.get("session_id"))
            .and_then(|v| v.as_str())
            .context("Missing 'id' parameter")?;

        let client = self.client.clone();

        self.runtime.block_on(async {
            let client_lock = client.read().await;
            if let Some(ref browser_client) = *client_lock {
                browser_client.close_session(session_id).await
            } else {
                Ok(())
            }
        })?;

        Ok(serde_json::json!({
            "success": true,
            "session_id": session_id
        }))
    }

    // =========================================================================
    // FEATURE PARITY HANDLERS
    // =========================================================================

    fn handle_select(&self, params: HashMap<String, Value>) -> Result<Value> {
        let selector = params
            .get("selector")
            .and_then(|v| v.as_str())
            .context("Missing 'selector' parameter")?;
        let value = params
            .get("value")
            .and_then(|v| v.as_str())
            .context("Missing 'value' parameter")?;
        let session_id = Self::get_session_id(&params);

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();
        let selector = selector.to_string();
        let value = value.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client
                .select(&selector, &value, session_id.as_deref())
                .await
        })?;

        Ok(serde_json::json!({
            "success": true,
            "selector": selector,
            "value": value
        }))
    }

    fn handle_check(&self, params: HashMap<String, Value>) -> Result<Value> {
        let selector = params
            .get("selector")
            .and_then(|v| v.as_str())
            .context("Missing 'selector' parameter")?;
        let checked = params
            .get("checked")
            .and_then(|v| v.as_bool())
            .unwrap_or(true); // Default to checking
        let session_id = Self::get_session_id(&params);

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();
        let selector = selector.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client
                .check(&selector, checked, session_id.as_deref())
                .await
        })?;

        Ok(serde_json::json!({
            "success": true,
            "selector": selector,
            "checked": checked
        }))
    }

    fn handle_hover(&self, params: HashMap<String, Value>) -> Result<Value> {
        let selector = params
            .get("selector")
            .and_then(|v| v.as_str())
            .context("Missing 'selector' parameter")?;
        let session_id = Self::get_session_id(&params);

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();
        let selector = selector.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client.hover(&selector, session_id.as_deref()).await
        })?;

        Ok(serde_json::json!({
            "success": true,
            "selector": selector
        }))
    }

    fn handle_scroll(&self, params: HashMap<String, Value>) -> Result<Value> {
        let selector = params.get("selector").and_then(|v| v.as_str());
        let x = params.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let y = params.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let session_id = Self::get_session_id(&params);

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();
        let selector = selector.map(|s| s.to_string());

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client
                .scroll(selector.as_deref(), x, y, session_id.as_deref())
                .await
        })?;

        Ok(serde_json::json!({
            "success": true,
            "x": x,
            "y": y
        }))
    }

    fn handle_press_combo(&self, params: HashMap<String, Value>) -> Result<Value> {
        let key = params
            .get("key")
            .and_then(|v| v.as_str())
            .context("Missing 'key' parameter")?;
        let modifiers: Vec<String> = params
            .get("modifiers")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let session_id = Self::get_session_id(&params);

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();
        let key = key.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            let mod_refs: Vec<&str> = modifiers.iter().map(|s| s.as_str()).collect();
            browser_client
                .press_combo(&mod_refs, &key, session_id.as_deref())
                .await
        })?;

        Ok(serde_json::json!({
            "success": true,
            "key": key,
            "modifiers": modifiers
        }))
    }

    fn handle_upload(&self, params: HashMap<String, Value>) -> Result<Value> {
        let selector = params
            .get("selector")
            .and_then(|v| v.as_str())
            .context("Missing 'selector' parameter")?;
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .context("Missing 'path' parameter")?;
        let session_id = Self::get_session_id(&params);

        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;
        let connect_url = self.connect_url.clone();
        let selector = selector.to_string();
        let path = path.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless, connect_url.as_deref()).await?;
            browser_client
                .upload(&selector, &path, session_id.as_deref())
                .await
        })?;

        Ok(serde_json::json!({
            "success": true,
            "selector": selector,
            "path": path
        }))
    }

    // =========================================================================
    // EXTENSION BRIDGE ROUTING
    // =========================================================================

    /// Route extension-specific methods to the Chrome extension via WebSocket.
    /// Called by dispatch() when the method is an extension-only method.
    fn dispatch_to_extension(&self, method: &str, params: HashMap<String, Value>) -> Result<Value> {
        let bridge = self.extension_bridge.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "Extension method '{}' requires Chrome extension. \
                 Start daemon with --extension-bridge and install the FGP extension.",
                method
            )
        })?;

        if !bridge.is_connected_blocking() {
            anyhow::bail!(
                "Extension not connected. Install the FGP Browser Bridge extension \
                 from chrome://extensions and ensure it's enabled."
            );
        }

        tracing::debug!("Routing '{}' to Chrome extension", method);
        let response = bridge.call_blocking(method, params)?;
        ExtensionBridge::response_to_value(response)
    }
}

impl FgpService for BrowserService {
    fn name(&self) -> &str {
        "browser"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn dispatch(&self, method: &str, params: HashMap<String, Value>) -> Result<Value> {
        // Check if this is an extension-only method that should be routed to the Chrome extension
        if is_extension_method(method) {
            return self.dispatch_to_extension(method, params);
        }

        match method {
            "health" => self.handle_health(params),
            // Navigation and state
            "browser.open" | "open" => self.handle_open(params),
            "browser.snapshot" | "snapshot" => self.handle_snapshot(params),
            "browser.screenshot" | "screenshot" => self.handle_screenshot(params),
            // Interaction
            "browser.click" | "click" => self.handle_click(params),
            "browser.fill" | "fill" => self.handle_fill(params),
            "browser.press" | "press" => self.handle_press(params),
            "browser.select" | "select" => self.handle_select(params),
            "browser.check" | "check" => self.handle_check(params),
            "browser.hover" | "hover" => self.handle_hover(params),
            "browser.scroll" | "scroll" => self.handle_scroll(params),
            "browser.press_combo" | "press_combo" => self.handle_press_combo(params),
            "browser.upload" | "upload" => self.handle_upload(params),
            // Auth state
            "browser.state.save" | "state.save" => self.handle_state_save(params),
            "browser.state.load" | "state.load" => self.handle_state_load(params),
            "browser.state.list" | "state.list" => self.handle_state_list(params),
            // Session management
            "browser.session.new" | "session.new" => self.handle_session_new(params),
            "browser.session.list" | "session.list" => self.handle_session_list(params),
            "browser.session.close" | "session.close" => self.handle_session_close(params),
            _ => Err(anyhow::anyhow!("Unknown method: {}", method)),
        }
    }

    fn method_list(&self) -> Vec<MethodInfo> {
        // Common session parameter schema
        let session_param = || {
            SchemaBuilder::string()
                .description("Session ID for isolated browser context (optional)")
        };

        vec![
            // ================================================================
            // Navigation and State
            // ================================================================
            MethodInfo::new("browser.open", "Navigate to a URL")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "url",
                            SchemaBuilder::string()
                                .format("uri")
                                .description("URL to navigate to"),
                        )
                        .property(
                            "wait_until",
                            SchemaBuilder::string()
                                .enum_values(&["load", "domcontentloaded", "networkidle"])
                                .default_value(json!("load"))
                                .description("When to consider navigation complete"),
                        )
                        .property("session_id", session_param())
                        .required(&["url"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("url", SchemaBuilder::string().format("uri"))
                        .property("title", SchemaBuilder::string())
                        .property("load_time_ms", SchemaBuilder::number())
                        .build(),
                )
                .example("Navigate to Google", json!({"url": "https://google.com"}))
                .example(
                    "Wait for network idle",
                    json!({"url": "https://example.com", "wait_until": "networkidle"}),
                )
                .errors(&["NAVIGATION_FAILED", "TIMEOUT"]),

            MethodInfo::new("browser.snapshot", "Get ARIA accessibility tree with @eN refs for element targeting")
                .schema(
                    SchemaBuilder::object()
                        .property("session_id", session_param())
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property(
                            "snapshot",
                            SchemaBuilder::string()
                                .description("ARIA tree with @eN refs for clicking/filling"),
                        )
                        .property("url", SchemaBuilder::string().format("uri"))
                        .property("title", SchemaBuilder::string())
                        .build(),
                )
                .example("Get page snapshot", json!({})),

            MethodInfo::new("browser.screenshot", "Capture screenshot as base64 or save to file")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "path",
                            SchemaBuilder::string()
                                .description("File path to save screenshot (optional, returns base64 if omitted)"),
                        )
                        .property(
                            "full_page",
                            SchemaBuilder::boolean()
                                .default_value(json!(false))
                                .description("Capture full scrollable page"),
                        )
                        .property("session_id", session_param())
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property(
                            "base64",
                            SchemaBuilder::string()
                                .description("Base64-encoded PNG (if no path specified)"),
                        )
                        .property(
                            "path",
                            SchemaBuilder::string()
                                .description("Saved file path (if path was specified)"),
                        )
                        .property("width", SchemaBuilder::integer())
                        .property("height", SchemaBuilder::integer())
                        .build(),
                )
                .example("Get base64 screenshot", json!({}))
                .example("Save to file", json!({"path": "/tmp/screenshot.png", "full_page": true})),

            // ================================================================
            // Interaction
            // ================================================================
            MethodInfo::new("browser.click", "Click element by @eN ref or CSS selector")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "selector",
                            SchemaBuilder::string()
                                .description("@eN ref from snapshot or CSS selector"),
                        )
                        .property(
                            "button",
                            SchemaBuilder::string()
                                .enum_values(&["left", "right", "middle"])
                                .default_value(json!("left")),
                        )
                        .property(
                            "click_count",
                            SchemaBuilder::integer()
                                .minimum(1)
                                .maximum(3)
                                .default_value(json!(1))
                                .description("1=click, 2=double-click, 3=triple-click"),
                        )
                        .property("session_id", session_param())
                        .required(&["selector"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("clicked", SchemaBuilder::boolean())
                        .property("selector", SchemaBuilder::string())
                        .build(),
                )
                .example("Click by ref", json!({"selector": "@e15"}))
                .example("Double-click", json!({"selector": "@e20", "click_count": 2}))
                .errors(&["ELEMENT_NOT_FOUND", "ELEMENT_NOT_VISIBLE"]),

            MethodInfo::new("browser.fill", "Fill input field with value")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "selector",
                            SchemaBuilder::string()
                                .description("@eN ref from snapshot or CSS selector"),
                        )
                        .property(
                            "value",
                            SchemaBuilder::string().description("Text to fill"),
                        )
                        .property(
                            "clear",
                            SchemaBuilder::boolean()
                                .default_value(json!(true))
                                .description("Clear existing content before filling"),
                        )
                        .property("session_id", session_param())
                        .required(&["selector", "value"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("filled", SchemaBuilder::boolean())
                        .property("selector", SchemaBuilder::string())
                        .build(),
                )
                .example("Fill search box", json!({"selector": "@e5", "value": "search query"}))
                .errors(&["ELEMENT_NOT_FOUND", "ELEMENT_NOT_EDITABLE"]),

            MethodInfo::new("browser.press", "Press a keyboard key")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "key",
                            SchemaBuilder::string()
                                .description("Key name: Enter, Tab, Escape, ArrowDown, etc."),
                        )
                        .property("session_id", session_param())
                        .required(&["key"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("pressed", SchemaBuilder::boolean())
                        .property("key", SchemaBuilder::string())
                        .build(),
                )
                .example("Press Enter", json!({"key": "Enter"}))
                .example("Press Escape", json!({"key": "Escape"})),

            MethodInfo::new("browser.select", "Select an option from a dropdown")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "selector",
                            SchemaBuilder::string()
                                .description("@eN ref or CSS selector for <select> element"),
                        )
                        .property(
                            "value",
                            SchemaBuilder::string().description("Option value to select"),
                        )
                        .property("session_id", session_param())
                        .required(&["selector", "value"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("selected", SchemaBuilder::boolean())
                        .property("value", SchemaBuilder::string())
                        .build(),
                )
                .example("Select option", json!({"selector": "@e10", "value": "option2"}))
                .errors(&["ELEMENT_NOT_FOUND", "OPTION_NOT_FOUND"]),

            MethodInfo::new("browser.check", "Set checkbox or radio button state")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "selector",
                            SchemaBuilder::string()
                                .description("@eN ref or CSS selector"),
                        )
                        .property(
                            "checked",
                            SchemaBuilder::boolean()
                                .default_value(json!(true))
                                .description("Desired checked state"),
                        )
                        .property("session_id", session_param())
                        .required(&["selector"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("checked", SchemaBuilder::boolean())
                        .build(),
                )
                .example("Check checkbox", json!({"selector": "@e8"}))
                .example("Uncheck", json!({"selector": "@e8", "checked": false})),

            MethodInfo::new("browser.hover", "Hover over an element")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "selector",
                            SchemaBuilder::string()
                                .description("@eN ref or CSS selector"),
                        )
                        .property("session_id", session_param())
                        .required(&["selector"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("hovered", SchemaBuilder::boolean())
                        .build(),
                )
                .example("Hover over menu", json!({"selector": "@e12"}))
                .errors(&["ELEMENT_NOT_FOUND"]),

            MethodInfo::new("browser.scroll", "Scroll page or element")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "selector",
                            SchemaBuilder::string()
                                .description("@eN ref or CSS selector to scroll into view"),
                        )
                        .property(
                            "direction",
                            SchemaBuilder::string()
                                .enum_values(&["up", "down", "left", "right"])
                                .description("Scroll direction (if no selector)"),
                        )
                        .property(
                            "amount",
                            SchemaBuilder::integer()
                                .minimum(0)
                                .default_value(json!(500))
                                .description("Pixels to scroll (if using direction)"),
                        )
                        .property("session_id", session_param())
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("scrolled", SchemaBuilder::boolean())
                        .build(),
                )
                .example("Scroll to element", json!({"selector": "@e50"}))
                .example("Scroll down", json!({"direction": "down", "amount": 1000})),

            MethodInfo::new("browser.press_combo", "Press key with modifiers (Ctrl, Shift, Alt, Meta)")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "key",
                            SchemaBuilder::string().description("Main key to press"),
                        )
                        .property(
                            "modifiers",
                            SchemaBuilder::array()
                                .items(
                                    SchemaBuilder::string()
                                        .enum_values(&["ctrl", "shift", "alt", "meta"]),
                                )
                                .description("Modifier keys to hold"),
                        )
                        .property("session_id", session_param())
                        .required(&["key", "modifiers"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("pressed", SchemaBuilder::boolean())
                        .build(),
                )
                .example("Select all (Ctrl+A)", json!({"key": "a", "modifiers": ["ctrl"]}))
                .example("Copy (Cmd+C on Mac)", json!({"key": "c", "modifiers": ["meta"]})),

            MethodInfo::new("browser.upload", "Upload a file to a file input element")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "selector",
                            SchemaBuilder::string()
                                .description("@eN ref or CSS selector for file input"),
                        )
                        .property(
                            "path",
                            SchemaBuilder::string()
                                .description("Absolute path to file to upload"),
                        )
                        .property("session_id", session_param())
                        .required(&["selector", "path"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("uploaded", SchemaBuilder::boolean())
                        .property("filename", SchemaBuilder::string())
                        .build(),
                )
                .example("Upload file", json!({"selector": "@e30", "path": "/tmp/document.pdf"}))
                .errors(&["ELEMENT_NOT_FOUND", "FILE_NOT_FOUND"]),

            // ================================================================
            // Auth State Management
            // ================================================================
            MethodInfo::new("browser.state.save", "Save auth state (cookies + localStorage) for reuse")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "name",
                            SchemaBuilder::string()
                                .min_length(1)
                                .max_length(64)
                                .pattern("^[a-zA-Z0-9_-]+$")
                                .description("Name for this auth state"),
                        )
                        .property("session_id", session_param())
                        .required(&["name"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("saved", SchemaBuilder::boolean())
                        .property("name", SchemaBuilder::string())
                        .property("path", SchemaBuilder::string())
                        .build(),
                )
                .example("Save GitHub auth", json!({"name": "github-prod"})),

            MethodInfo::new("browser.state.load", "Load previously saved auth state")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "name",
                            SchemaBuilder::string().description("Name of saved auth state"),
                        )
                        .property("session_id", session_param())
                        .required(&["name"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("loaded", SchemaBuilder::boolean())
                        .property("name", SchemaBuilder::string())
                        .build(),
                )
                .example("Load GitHub auth", json!({"name": "github-prod"}))
                .errors(&["STATE_NOT_FOUND"]),

            MethodInfo::new("browser.state.list", "List all saved auth states")
                .schema(SchemaBuilder::object().build())
                .returns(
                    SchemaBuilder::object()
                        .property(
                            "states",
                            SchemaBuilder::array().items(
                                SchemaBuilder::object()
                                    .property("name", SchemaBuilder::string())
                                    .property("created_at", SchemaBuilder::string().format("date-time"))
                                    .property("size_bytes", SchemaBuilder::integer()),
                            ),
                        )
                        .property("count", SchemaBuilder::integer())
                        .build(),
                )
                .example("List auth states", json!({})),

            // ================================================================
            // Session Management
            // ================================================================
            MethodInfo::new("browser.session.new", "Create isolated session with separate cookies/storage")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "name",
                            SchemaBuilder::string()
                                .description("Optional friendly name for the session"),
                        )
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("session_id", SchemaBuilder::string())
                        .property("name", SchemaBuilder::string())
                        .build(),
                )
                .example("Create named session", json!({"name": "shopping-cart"}))
                .example("Create anonymous session", json!({})),

            MethodInfo::new("browser.session.list", "List all active browser sessions")
                .schema(SchemaBuilder::object().build())
                .returns(
                    SchemaBuilder::object()
                        .property(
                            "sessions",
                            SchemaBuilder::array().items(
                                SchemaBuilder::object()
                                    .property("session_id", SchemaBuilder::string())
                                    .property("name", SchemaBuilder::string())
                                    .property("created_at", SchemaBuilder::string().format("date-time"))
                                    .property("current_url", SchemaBuilder::string().format("uri")),
                            ),
                        )
                        .property("count", SchemaBuilder::integer())
                        .build(),
                )
                .example("List sessions", json!({})),

            MethodInfo::new("browser.session.close", "Close and dispose a browser session")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "session_id",
                            SchemaBuilder::string().description("Session ID to close"),
                        )
                        .required(&["session_id"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("closed", SchemaBuilder::boolean())
                        .property("session_id", SchemaBuilder::string())
                        .build(),
                )
                .example("Close session", json!({"session_id": "abc123"}))
                .errors(&["SESSION_NOT_FOUND"]),

            // ================================================================
            // Extension Methods (requires Chrome extension)
            // ================================================================
            MethodInfo::new("tabs.group", "[Extension] Group tabs together")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "tabIds",
                            SchemaBuilder::array()
                                .items(SchemaBuilder::integer())
                                .description("Tab IDs to group"),
                        )
                        .property(
                            "groupId",
                            SchemaBuilder::integer()
                                .description("Existing group ID to add tabs to (optional)"),
                        )
                        .required(&["tabIds"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("groupId", SchemaBuilder::integer())
                        .build(),
                )
                .example("Group tabs", json!({"tabIds": [1, 2, 3]}))
                .errors(&["EXTENSION_NOT_CONNECTED"]),

            MethodInfo::new("tabGroups.update", "[Extension] Update tab group properties")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "groupId",
                            SchemaBuilder::integer().description("Tab group ID"),
                        )
                        .property(
                            "title",
                            SchemaBuilder::string().description("Group title"),
                        )
                        .property(
                            "color",
                            SchemaBuilder::string()
                                .enum_values(&["grey", "blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange"])
                                .description("Group color"),
                        )
                        .property(
                            "collapsed",
                            SchemaBuilder::boolean().description("Collapse the group"),
                        )
                        .required(&["groupId"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("updated", SchemaBuilder::boolean())
                        .build(),
                )
                .example("Update group", json!({"groupId": 1, "title": "Research", "color": "blue"}))
                .errors(&["EXTENSION_NOT_CONNECTED", "GROUP_NOT_FOUND"]),

            MethodInfo::new("cookies.getAll", "[Extension] Get all cookies matching criteria")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "domain",
                            SchemaBuilder::string().description("Cookie domain filter"),
                        )
                        .property(
                            "url",
                            SchemaBuilder::string().format("uri").description("URL to get cookies for"),
                        )
                        .build(),
                )
                .returns(
                    SchemaBuilder::array().items(
                        SchemaBuilder::object()
                            .property("name", SchemaBuilder::string())
                            .property("value", SchemaBuilder::string())
                            .property("domain", SchemaBuilder::string())
                            .property("path", SchemaBuilder::string())
                            .property("secure", SchemaBuilder::boolean())
                            .property("httpOnly", SchemaBuilder::boolean()),
                    ).build(),
                )
                .example("Get Twitter cookies", json!({"domain": ".x.com"}))
                .errors(&["EXTENSION_NOT_CONNECTED"]),

            MethodInfo::new("notifications.create", "[Extension] Show desktop notification")
                .schema(
                    SchemaBuilder::object()
                        .property(
                            "title",
                            SchemaBuilder::string().description("Notification title"),
                        )
                        .property(
                            "message",
                            SchemaBuilder::string().description("Notification message"),
                        )
                        .required(&["title", "message"])
                        .build(),
                )
                .returns(
                    SchemaBuilder::object()
                        .property("notificationId", SchemaBuilder::string())
                        .build(),
                )
                .example("Show notification", json!({"title": "Task Complete", "message": "Scraping finished"}))
                .errors(&["EXTENSION_NOT_CONNECTED"]),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_session_id_with_session_id() {
        let mut params = HashMap::new();
        params.insert("session_id".to_string(), json!("test-session-123"));
        params.insert("url".to_string(), json!("https://example.com"));

        let session_id = BrowserService::get_session_id(&params);
        assert_eq!(session_id, Some("test-session-123".to_string()));
    }

    #[test]
    fn test_get_session_id_with_session_alias() {
        let mut params = HashMap::new();
        params.insert("session".to_string(), json!("my-session"));

        let session_id = BrowserService::get_session_id(&params);
        assert_eq!(session_id, Some("my-session".to_string()));
    }

    #[test]
    fn test_get_session_id_missing() {
        let mut params = HashMap::new();
        params.insert("url".to_string(), json!("https://example.com"));

        let session_id = BrowserService::get_session_id(&params);
        assert_eq!(session_id, None);
    }

    #[test]
    fn test_get_session_id_prefers_session_id_over_session() {
        let mut params = HashMap::new();
        params.insert("session_id".to_string(), json!("preferred"));
        params.insert("session".to_string(), json!("fallback"));

        let session_id = BrowserService::get_session_id(&params);
        assert_eq!(session_id, Some("preferred".to_string()));
    }

    #[test]
    fn test_get_session_id_ignores_non_string() {
        let mut params = HashMap::new();
        params.insert("session_id".to_string(), json!(123));

        let session_id = BrowserService::get_session_id(&params);
        assert_eq!(session_id, None);
    }
}

//! BrowserService implementing FgpService trait.
//!
//! Supports session-based browser automation for parallel requests.
//! Each session has isolated context (cookies, localStorage, cache).

use anyhow::{Context, Result};
use chrono::Utc;
use fgp_daemon::service::MethodInfo;
use fgp_daemon::FgpService;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::RwLock;

use crate::browser::BrowserClient;
use crate::models::*;

/// Browser automation service.
pub struct BrowserService {
    runtime: Runtime,
    client: Arc<RwLock<Option<Arc<BrowserClient>>>>,
    user_data_dir: PathBuf,
    auth_dir: PathBuf,
    headless: bool,
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
    ) -> Result<Arc<BrowserClient>> {
        if let Some(existing) = client.read().await.as_ref() {
            return Ok(Arc::clone(existing));
        }

        let mut client_lock = client.write().await;
        if client_lock.is_none() {
            let new_client = BrowserClient::new(user_data_dir.to_path_buf(), headless).await?;
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

        let result = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
            browser_client.navigate(url, session_id.as_deref()).await
        })?;

        Ok(serde_json::to_value(result)?)
    }

    fn handle_snapshot(&self, params: HashMap<String, Value>) -> Result<Value> {
        let session_id = Self::get_session_id(&params);
        let client = self.client.clone();
        let user_data_dir = self.user_data_dir.clone();
        let headless = self.headless;

        let result = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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

        let result = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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
        let selector = selector.to_string();

        let result = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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
        let selector = selector.to_string();
        let value = value.to_string();

        let result = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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
        let key = key.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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

        let state = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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

        let id = self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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
        let selector = selector.to_string();
        let value = value.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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
        let selector = selector.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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
        let selector = selector.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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
        let selector = selector.map(|s| s.to_string());

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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
        let key = key.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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
        let selector = selector.to_string();
        let path = path.to_string();

        self.runtime.block_on(async {
            let browser_client =
                Self::get_or_init_client(&client, &user_data_dir, headless).await?;
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
}

impl FgpService for BrowserService {
    fn name(&self) -> &str {
        "browser"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn dispatch(&self, method: &str, params: HashMap<String, Value>) -> Result<Value> {
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
        vec![
            // Navigation and state
            MethodInfo {
                name: "browser.open".to_string(),
                description: "Navigate to a URL".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.snapshot".to_string(),
                description: "Get ARIA accessibility tree with @eN refs".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.screenshot".to_string(),
                description: "Capture screenshot (base64 or file)".to_string(),
                params: vec![],
            },
            // Interaction
            MethodInfo {
                name: "browser.click".to_string(),
                description: "Click element by @eN ref or CSS selector".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.fill".to_string(),
                description: "Fill input field with value".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.press".to_string(),
                description: "Press a keyboard key".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.select".to_string(),
                description: "Select an option from a dropdown".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.check".to_string(),
                description: "Set checkbox/radio state".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.hover".to_string(),
                description: "Hover over an element".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.scroll".to_string(),
                description: "Scroll to element or by amount".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.press_combo".to_string(),
                description: "Press key with modifiers (Ctrl, Shift, Alt, Meta)".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.upload".to_string(),
                description: "Upload a file to a file input element".to_string(),
                params: vec![],
            },
            // Auth state
            MethodInfo {
                name: "browser.state.save".to_string(),
                description: "Save auth state (cookies + localStorage)".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.state.load".to_string(),
                description: "Load saved auth state".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.state.list".to_string(),
                description: "List saved auth states".to_string(),
                params: vec![],
            },
            // Session management
            MethodInfo {
                name: "browser.session.new".to_string(),
                description: "Create a new isolated session with its own browser context"
                    .to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.session.list".to_string(),
                description: "List all active sessions".to_string(),
                params: vec![],
            },
            MethodInfo {
                name: "browser.session.close".to_string(),
                description: "Close and dispose a session".to_string(),
                params: vec![],
            },
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

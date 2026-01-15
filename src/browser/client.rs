//! Chrome CDP client wrapper - optimized for maximum performance with session support.
//!
//! Supports multiple concurrent sessions for parallel browser automation.
//! Each session has isolated context (cookies, localStorage, cache).

use anyhow::{Context, Result};
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::browser::BrowserContextId;
use chromiumoxide::cdp::browser_protocol::input::{DispatchKeyEventParams, DispatchKeyEventType};
use chromiumoxide::cdp::browser_protocol::network::{
    CookieParam, SetCookiesParams, TimeSinceEpoch,
};
use chromiumoxide::cdp::browser_protocol::target::CreateBrowserContextParams;
use chromiumoxide::page::Page;
use futures::StreamExt;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::aria::extract_aria_tree;
use crate::models::{
    AriaSnapshot, ClickResult, FillResult, LocalStorageState, NavigationResult, ScreenshotResult,
    SerializableCookie,
};

/// A browser session with isolated context.
pub struct BrowserSession {
    pub id: String,
    pub context_id: Option<BrowserContextId>, // None = default context
    pub page: Page,
}

/// Chrome browser client with multi-session support for parallel requests.
pub struct BrowserClient {
    browser: Browser,
    sessions: Arc<RwLock<HashMap<String, BrowserSession>>>,
    default_session_id: String,
    #[allow(dead_code)]
    user_data_dir: PathBuf,
}

impl BrowserClient {
    /// Create a new browser client with a default session.
    pub async fn new(user_data_dir: PathBuf, headless: bool) -> Result<Self> {
        // Ensure user data directory exists
        tokio::fs::create_dir_all(&user_data_dir).await?;

        // Find Chrome executable - check common paths
        let chrome_path = Self::find_chrome_executable()?;

        let mut builder = BrowserConfig::builder()
            .chrome_executable(chrome_path)
            .user_data_dir(&user_data_dir)
            .viewport(None)
            .no_sandbox()
            // Performance flags (matching agent-browser/Playwright)
            .arg("--headless=old")
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg("--disable-back-forward-cache")
            .arg("--disable-background-timer-throttling")
            .arg("--disable-breakpad")
            .arg("--disable-component-extensions-with-background-pages")
            .arg("--disable-default-apps")
            .arg("--disable-extensions")
            .arg("--disable-hang-monitor")
            .arg("--disable-ipc-flooding-protection")
            .arg("--disable-popup-blocking")
            .arg("--disable-prompt-on-repost")
            .arg("--disable-renderer-backgrounding")
            .arg("--disable-sync")
            .arg("--disable-translate")
            .arg("--metrics-recording-only")
            .arg("--mute-audio")
            .arg("--no-first-run")
            .arg("--password-store=basic")
            .arg("--disable-features=MediaRouter,OptimizationHints,Translate,ThirdPartyStoragePartitioning");

        if !headless {
            builder = builder.with_head();
        }

        let config = builder
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build browser config: {}", e))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .context("Failed to launch browser")?;

        // Spawn handler task - just drain events, no logging overhead
        tokio::spawn(async move { while handler.next().await.is_some() {} });

        // Create default session with pre-warmed page
        let default_page = browser
            .new_page("about:blank")
            .await
            .context("Failed to create initial page")?;

        let default_session_id = "default".to_string();
        let default_session = BrowserSession {
            id: default_session_id.clone(),
            context_id: None, // Uses browser's default context
            page: default_page,
        };

        let mut sessions = HashMap::new();
        sessions.insert(default_session_id.clone(), default_session);

        Ok(Self {
            browser,
            sessions: Arc::new(RwLock::new(sessions)),
            default_session_id,
            user_data_dir,
        })
    }

    /// Create a new isolated session with its own browser context.
    pub async fn create_session(&self, session_id: &str) -> Result<String> {
        let mut sessions = self.sessions.write().await;

        if sessions.contains_key(session_id) {
            return Ok(session_id.to_string());
        }

        // Create isolated browser context
        let context_id = self
            .browser
            .create_browser_context(CreateBrowserContextParams::default())
            .await
            .context("Failed to create browser context")?;

        // Create page in the new context
        let page = self
            .browser
            .new_page(
                chromiumoxide::cdp::browser_protocol::target::CreateTargetParams::builder()
                    .url("about:blank")
                    .browser_context_id(context_id.clone())
                    .build()
                    .map_err(|e| anyhow::anyhow!("Failed to build target params: {:?}", e))?,
            )
            .await
            .context("Failed to create page in context")?;

        let session = BrowserSession {
            id: session_id.to_string(),
            context_id: Some(context_id),
            page,
        };

        sessions.insert(session_id.to_string(), session);
        tracing::info!("Created new session: {}", session_id);

        Ok(session_id.to_string())
    }

    /// Close and dispose a session.
    pub async fn close_session(&self, session_id: &str) -> Result<()> {
        if session_id == self.default_session_id {
            anyhow::bail!("Cannot close default session");
        }

        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.remove(session_id) {
            if let Some(context_id) = session.context_id {
                self.browser
                    .dispose_browser_context(context_id)
                    .await
                    .context("Failed to dispose browser context")?;
            }
            tracing::info!("Closed session: {}", session_id);
        }

        Ok(())
    }

    /// List all active sessions.
    pub async fn list_sessions(&self) -> Vec<String> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .map(|session| session.id.clone())
            .collect()
    }

    /// Get page for a session (or default).
    async fn get_page(&self, session_id: Option<&str>) -> Result<Page> {
        let sessions = self.sessions.read().await;
        let sid = session_id.unwrap_or(&self.default_session_id);

        sessions
            .get(sid)
            .map(|s| s.page.clone())
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", sid))
    }

    /// Navigate to a URL.
    pub async fn navigate(&self, url: &str, session_id: Option<&str>) -> Result<NavigationResult> {
        let page = self.get_page(session_id).await?;

        page.goto(url).await.context("Navigation failed")?;
        page.wait_for_navigation().await.ok();

        let current_url = page.url().await?.unwrap_or_default();
        let title = page.get_title().await?.unwrap_or_default();

        Ok(NavigationResult {
            url: current_url.to_string(),
            title,
            status: None,
        })
    }

    /// Get ARIA accessibility tree snapshot.
    pub async fn snapshot(&self, session_id: Option<&str>) -> Result<AriaSnapshot> {
        let page = self.get_page(session_id).await?;

        let url = page.url().await?.unwrap_or_default().to_string();
        let title = page.get_title().await?.unwrap_or_default();

        let nodes = extract_aria_tree(&page).await?;
        let element_count = count_nodes(&nodes);

        Ok(AriaSnapshot {
            url,
            title,
            nodes,
            element_count,
        })
    }

    /// Export cookies for a session.
    pub async fn get_cookies(&self, session_id: Option<&str>) -> Result<Vec<SerializableCookie>> {
        let page = self.get_page(session_id).await?;
        let cookies = page.get_cookies().await?;

        Ok(cookies
            .into_iter()
            .map(|cookie| SerializableCookie {
                name: cookie.name,
                value: cookie.value,
                domain: cookie.domain,
                path: cookie.path,
                expires: if cookie.session {
                    None
                } else {
                    Some(cookie.expires)
                },
                secure: cookie.secure,
                http_only: cookie.http_only,
                same_site: cookie.same_site,
            })
            .collect())
    }

    /// Restore cookies for a session.
    pub async fn set_cookies(
        &self,
        cookies: &[SerializableCookie],
        session_id: Option<&str>,
    ) -> Result<()> {
        if cookies.is_empty() {
            return Ok(());
        }

        let page = self.get_page(session_id).await?;
        let params: Vec<CookieParam> = cookies
            .iter()
            .map(|cookie| {
                let mut param = CookieParam::new(cookie.name.clone(), cookie.value.clone());
                param.domain = Some(cookie.domain.clone());
                param.path = Some(cookie.path.clone());
                param.secure = Some(cookie.secure);
                param.http_only = Some(cookie.http_only);
                param.same_site = cookie.same_site.clone();
                param.expires = cookie.expires.map(TimeSinceEpoch::new);
                param
            })
            .collect();

        page.execute(SetCookiesParams::new(params)).await?;
        Ok(())
    }

    /// Capture localStorage for a session.
    pub async fn get_local_storage(&self, session_id: Option<&str>) -> Result<LocalStorageState> {
        let page = self.get_page(session_id).await?;
        let origin: String = page
            .evaluate("location.origin")
            .await
            .context("Failed to evaluate location.origin")?
            .into_value()
            .context("Failed to parse location.origin")?;

        let entries: Vec<(String, String)> = page
            .evaluate("(() => { try { return Object.entries(localStorage); } catch (_) { return []; } })()")
            .await
            .context("Failed to read localStorage entries")?
            .into_value()
            .context("Failed to parse localStorage entries")?;

        let items: HashMap<String, String> = entries.into_iter().collect();

        Ok(LocalStorageState { origin, items })
    }

    /// Restore localStorage for a session.
    pub async fn set_local_storage(
        &self,
        state: &LocalStorageState,
        session_id: Option<&str>,
    ) -> Result<()> {
        let page = self.get_page(session_id).await?;
        let payload = serde_json::to_string(&state.items)?;
        let script = format!(
            r#"(function() {{
                const items = {};
                try {{ localStorage.clear(); }} catch (_) {{}}
                for (const [k, v] of Object.entries(items)) {{
                    try {{ localStorage.setItem(k, v); }} catch (_) {{}}
                }}
            }})()"#,
            payload
        );

        page.evaluate(script).await?;
        Ok(())
    }

    /// Take a screenshot.
    pub async fn screenshot(
        &self,
        path: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<ScreenshotResult> {
        let page = self.get_page(session_id).await?;

        let screenshot_data = page
            .screenshot(
                chromiumoxide::page::ScreenshotParams::builder()
                    .full_page(true)
                    .build(),
            )
            .await?;

        let (width, height) = (1920, 1080);

        if let Some(file_path) = path {
            tokio::fs::write(file_path, &screenshot_data).await?;
            Ok(ScreenshotResult {
                data: None,
                path: Some(file_path.to_string()),
                width,
                height,
            })
        } else {
            let encoded = base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &screenshot_data,
            );
            Ok(ScreenshotResult {
                data: Some(encoded),
                path: None,
                width,
                height,
            })
        }
    }

    /// Click an element.
    pub async fn click(&self, selector: &str, session_id: Option<&str>) -> Result<ClickResult> {
        let page = self.get_page(session_id).await?;

        let css_selector = resolve_selector(selector);

        let element = page
            .find_element(&css_selector)
            .await
            .context("Element not found")?;

        element.click().await?;

        Ok(ClickResult {
            success: true,
            element: Some(selector.to_string()),
        })
    }

    /// Fill an input field.
    pub async fn fill(
        &self,
        selector: &str,
        value: &str,
        session_id: Option<&str>,
    ) -> Result<FillResult> {
        let page = self.get_page(session_id).await?;

        let css_selector = resolve_selector(selector);

        let element = page
            .find_element(&css_selector)
            .await
            .context("Element not found")?;

        element.click().await?;
        element.type_str(value).await?;

        Ok(FillResult {
            success: true,
            value: value.to_string(),
        })
    }

    /// Press a key.
    pub async fn press(&self, key: &str, session_id: Option<&str>) -> Result<()> {
        let page = self.get_page(session_id).await?;

        page.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyDown)
                .key(key)
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to build key event: {:?}", e))?,
        )
        .await?;

        page.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .key(key)
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to build key event: {:?}", e))?,
        )
        .await?;

        Ok(())
    }

    // =========================================================================
    // NEW METHODS FOR FEATURE PARITY
    // =========================================================================

    /// Select an option from a dropdown.
    pub async fn select(
        &self,
        selector: &str,
        value: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        let page = self.get_page(session_id).await?;
        let css_selector = resolve_selector(selector);

        // Use JSON encoding for safe string escaping
        let selector_json = serde_json::to_string(&css_selector)?;
        let value_json = serde_json::to_string(value)?;

        let script = format!(
            r#"(() => {{
                const sel = {};
                const val = {};
                const el = document.querySelector(sel);
                if (!el) throw new Error('Element not found: ' + sel);
                el.value = val;
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return true;
            }})()"#,
            selector_json, value_json
        );

        page.evaluate(script)
            .await
            .context("Failed to select option")?;

        Ok(())
    }

    /// Set checkbox/radio state.
    pub async fn check(
        &self,
        selector: &str,
        checked: bool,
        session_id: Option<&str>,
    ) -> Result<()> {
        let page = self.get_page(session_id).await?;
        let css_selector = resolve_selector(selector);

        let selector_json = serde_json::to_string(&css_selector)?;

        let script = format!(
            r#"(() => {{
                const sel = {};
                const el = document.querySelector(sel);
                if (!el) throw new Error('Element not found: ' + sel);
                if (el.checked !== {}) {{
                    el.click();
                }}
                return el.checked;
            }})()"#,
            selector_json, checked
        );

        page.evaluate(script)
            .await
            .context("Failed to set checkbox state")?;

        Ok(())
    }

    /// Hover over an element.
    pub async fn hover(&self, selector: &str, session_id: Option<&str>) -> Result<()> {
        let page = self.get_page(session_id).await?;
        let css_selector = resolve_selector(selector);

        let element = page
            .find_element(&css_selector)
            .await
            .context("Element not found")?;

        element.hover().await?;

        Ok(())
    }

    /// Scroll to element or by amount.
    pub async fn scroll(
        &self,
        selector: Option<&str>,
        x: i32,
        y: i32,
        session_id: Option<&str>,
    ) -> Result<()> {
        let page = self.get_page(session_id).await?;

        let script = if let Some(sel) = selector {
            let css_selector = resolve_selector(sel);
            let selector_json = serde_json::to_string(&css_selector)?;
            format!(
                r#"(() => {{
                    const sel = {};
                    const el = document.querySelector(sel);
                    if (el) el.scrollIntoView({{ behavior: 'instant', block: 'center' }});
                }})()"#,
                selector_json
            )
        } else {
            format!("window.scrollBy({}, {})", x, y)
        };

        page.evaluate(script).await?;

        Ok(())
    }

    /// Press a key with modifiers (Ctrl, Shift, Alt, Meta).
    pub async fn press_combo(
        &self,
        modifiers: &[&str],
        key: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        let page = self.get_page(session_id).await?;

        // Calculate modifier flags
        let modifier_flags: i64 = modifiers.iter().fold(0, |acc, m| {
            acc | match m.to_lowercase().as_str() {
                "ctrl" | "control" => 1,
                "shift" => 2,
                "alt" => 4,
                "meta" | "cmd" | "command" => 8,
                _ => 0,
            }
        });

        // Send keyDown with modifiers
        page.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyDown)
                .key(key)
                .modifiers(modifier_flags)
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to build key event: {:?}", e))?,
        )
        .await?;

        // Send keyUp
        page.execute(
            DispatchKeyEventParams::builder()
                .r#type(DispatchKeyEventType::KeyUp)
                .key(key)
                .modifiers(modifier_flags)
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to build key event: {:?}", e))?,
        )
        .await?;

        Ok(())
    }

    /// Upload a file to an input element.
    pub async fn upload(
        &self,
        selector: &str,
        file_path: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        let page = self.get_page(session_id).await?;
        let css_selector = resolve_selector(selector);

        // Resolve to absolute path
        let path = std::path::Path::new(file_path);
        let absolute_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        // Verify file exists
        if !absolute_path.exists() {
            anyhow::bail!("File not found: {}", absolute_path.display());
        }

        // Validate element exists and is a file input
        let selector_json = serde_json::to_string(&css_selector)?;

        // Get the DOM node ID for the element
        let script = format!(
            r#"(() => {{
                const sel = {};
                const el = document.querySelector(sel);
                if (!el) throw new Error('Element not found: ' + sel);
                if (el.tagName !== 'INPUT' || el.type !== 'file') {{
                    throw new Error('Element is not a file input: ' + sel);
                }}
                return true;
            }})()"#,
            selector_json
        );

        page.evaluate(script)
            .await
            .context("Element validation failed")?;

        // Use CDP DOM.setFileInputFiles via element
        // chromiumoxide Element doesn't expose this directly, so we use Page.evaluate with file input
        // Instead, we'll focus the element and use CDP Input domain

        // Get the element's node id and use DOM.setFileInputFiles
        use chromiumoxide::cdp::browser_protocol::dom::{
            GetDocumentParams, QuerySelectorParams, SetFileInputFilesParams,
        };

        // Get document root
        let doc = page.execute(GetDocumentParams::default()).await?;
        let root_node_id = doc.root.node_id;

        // Query for the element
        let query_result = page
            .execute(QuerySelectorParams::new(root_node_id, &css_selector))
            .await?;
        let node_id = query_result.node_id;

        // Set the file
        page.execute(
            SetFileInputFilesParams::builder()
                .files(vec![absolute_path.to_string_lossy().to_string()])
                .node_id(node_id)
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to build SetFileInputFilesParams: {:?}", e))?,
        )
        .await?;

        tracing::debug!("Uploaded file {} to {}", file_path, selector);

        Ok(())
    }

    /// Health check - verify browser is responsive.
    pub async fn health_check(&self) -> Result<bool> {
        let _version = self.browser.version().await?;
        Ok(true)
    }

    /// Close the browser.
    #[allow(dead_code)]
    pub async fn close(mut self) -> Result<()> {
        // Dispose all non-default sessions
        let sessions = self.sessions.read().await;
        for (id, session) in sessions.iter() {
            if id != &self.default_session_id {
                if let Some(ref context_id) = session.context_id {
                    let _ = self
                        .browser
                        .dispose_browser_context(context_id.clone())
                        .await;
                }
            }
        }
        drop(sessions);

        self.browser.close().await?;
        Ok(())
    }

    /// Find Chrome executable on the system.
    fn find_chrome_executable() -> Result<PathBuf> {
        let (subdir_name, alt_subdir) = if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                (
                    "chrome-headless-shell-mac-arm64",
                    "chrome-headless-shell-mac-x64",
                )
            } else {
                (
                    "chrome-headless-shell-mac-x64",
                    "chrome-headless-shell-mac-arm64",
                )
            }
        } else {
            ("chrome-headless-shell-linux", "chrome-headless-shell-linux")
        };

        if let Some(home) = dirs::home_dir() {
            let playwright_cache = home.join("Library/Caches/ms-playwright");
            if playwright_cache.exists() {
                if let Ok(entries) = std::fs::read_dir(&playwright_cache) {
                    let mut headless_dirs: Vec<_> = entries
                        .filter_map(|e| e.ok())
                        .filter(|e| {
                            e.file_name()
                                .to_string_lossy()
                                .starts_with("chromium_headless_shell")
                        })
                        .collect();

                    headless_dirs.sort_by_key(|b| std::cmp::Reverse(b.file_name()));

                    for dir in headless_dirs {
                        let binary = dir.path().join(subdir_name).join("chrome-headless-shell");

                        if binary.exists() {
                            tracing::info!("Using chrome-headless-shell at: {:?}", binary);
                            return Ok(binary);
                        }

                        let alt_binary = dir.path().join(alt_subdir).join("chrome-headless-shell");

                        if alt_binary.exists() {
                            tracing::info!("Using chrome-headless-shell at: {:?}", alt_binary);
                            return Ok(alt_binary);
                        }
                    }
                }
            }
        }

        let paths = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
        ];

        for path in &paths {
            let p = PathBuf::from(path);
            if p.exists() {
                tracing::info!("Found Chrome at: {}", path);
                return Ok(p);
            }
        }

        anyhow::bail!("Chrome/Chromium not found. Install Playwright (npx playwright install chromium) for best performance.")
    }
}

/// Resolve @eN selector to CSS selector.
fn resolve_selector(selector: &str) -> String {
    if selector.starts_with("@e") {
        format!("[data-fgp-ref='{}']", &selector[1..])
    } else {
        selector.to_string()
    }
}

/// Count total nodes in tree.
fn count_nodes(nodes: &[crate::models::AriaNode]) -> usize {
    nodes.iter().map(|n| 1 + count_nodes(&n.children)).sum()
}

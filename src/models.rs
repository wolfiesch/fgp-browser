//! Data models for browser automation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use chromiumoxide::cdp::browser_protocol::network::CookieSameSite;

/// ARIA tree node with @eN reference ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AriaNode {
    /// Element reference ID (e.g., "@e1", "@e2")
    pub ref_id: String,
    /// ARIA role (e.g., "button", "textbox", "link")
    pub role: String,
    /// Accessible name
    #[serde(default)]
    pub name: Option<String>,
    /// Node value (for inputs)
    #[serde(default)]
    pub value: Option<String>,
    /// Whether the element is focusable
    #[serde(default)]
    pub focusable: bool,
    /// Whether the element is focused
    #[serde(default)]
    pub focused: bool,
    /// Child nodes
    #[serde(default)]
    pub children: Vec<AriaNode>,
}

/// ARIA tree snapshot response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AriaSnapshot {
    /// Page URL
    pub url: String,
    /// Page title
    pub title: String,
    /// Root ARIA nodes
    pub nodes: Vec<AriaNode>,
    /// Total element count
    pub element_count: usize,
}

/// Screenshot response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResult {
    /// Base64-encoded PNG data (if no path specified)
    #[serde(default)]
    pub data: Option<String>,
    /// File path (if path was specified)
    #[serde(default)]
    pub path: Option<String>,
    /// Image dimensions
    pub width: u32,
    pub height: u32,
}

/// Navigation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigationResult {
    /// Final URL after navigation
    pub url: String,
    /// Page title
    pub title: String,
    /// HTTP status code
    #[serde(default)]
    pub status: Option<u16>,
}

/// Browser session info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Session ID
    pub id: String,
    /// Current URL
    #[serde(default)]
    pub url: Option<String>,
    /// Whether this is the active session
    pub active: bool,
}

/// Saved auth state info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedState {
    /// State name
    pub name: String,
    /// Domain(s) the state applies to
    pub domains: Vec<String>,
    /// When the state was saved
    pub saved_at: String,
}

/// Serializable cookie for auth state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    #[serde(default)]
    pub expires: Option<f64>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub same_site: Option<CookieSameSite>,
}

/// Local storage snapshot for a single origin.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalStorageState {
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub items: HashMap<String, String>,
}

/// Auth state snapshot with cookies and localStorage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthState {
    #[serde(default)]
    pub cookies: Vec<SerializableCookie>,
    #[serde(default)]
    pub local_storage: LocalStorageState,
    #[serde(default)]
    pub saved_at: String,
}

/// Click result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickResult {
    /// Whether click was successful
    pub success: bool,
    /// Element that was clicked (for debugging)
    #[serde(default)]
    pub element: Option<String>,
}

/// Fill result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillResult {
    /// Whether fill was successful
    pub success: bool,
    /// Value that was filled
    pub value: String,
}

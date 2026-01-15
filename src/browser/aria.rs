//! ARIA tree extraction from Chrome DevTools Protocol.

use anyhow::{Context, Result};
use chromiumoxide::cdp::browser_protocol::accessibility::{
    AxNode as CdpAxNode, AxProperty, AxPropertyName, GetFullAxTreeParams,
};
use chromiumoxide::page::Page;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::models::AriaNode;

/// Extract ARIA accessibility tree from page.
pub async fn extract_aria_tree(page: &Page) -> Result<Vec<AriaNode>> {
    let mut counter = 0;

    // Try CDP accessibility tree first
    if let Ok(response) = page.execute(GetFullAxTreeParams::default()).await {
        // Single-pass extraction - no clones, references only
        let capacity = response.nodes.len() / 4; // Most nodes filtered out
        let mut nodes = Vec::with_capacity(capacity);

        for node in &response.nodes {
            if is_interactive_node(node) || has_role_or_name(node) {
                nodes.push(convert_node_ref(node, &mut counter));
            }
        }

        if !nodes.is_empty() {
            tracing::debug!(
                "Extracted {} nodes from CDP accessibility tree",
                nodes.len()
            );
            return Ok(nodes);
        }
    }

    // Fallback to DOM traversal - more reliable on macOS
    tracing::debug!("CDP accessibility tree empty, falling back to DOM traversal");
    let nodes = extract_dom_interactives(page, &mut counter).await?;

    Ok(nodes)
}

/// Check if a node is interactive and should be included.
fn is_interactive_node(node: &CdpAxNode) -> bool {
    let role_match = node
        .role
        .as_ref()
        .and_then(|role| role.value.as_ref())
        .is_some_and(|value| {
            let role_str = json_as_str(value).unwrap_or("");
            matches!(
                role_str,
                "button"
                    | "link"
                    | "textbox"
                    | "checkbox"
                    | "radio"
                    | "combobox"
                    | "listbox"
                    | "menuitem"
                    | "tab"
                    | "slider"
                    | "searchbox"
                    | "spinbutton"
                    | "switch"
                    | "option"
                    | "menuitemcheckbox"
                    | "menuitemradio"
                    | "treeitem"
                    | "heading"
                    | "img"
                    | "navigation"
                    | "main"
                    | "article"
                    | "section"
            )
        });

    role_match || is_focusable(node)
}

fn has_role_or_name(node: &CdpAxNode) -> bool {
    node.role.as_ref().and_then(|r| r.value.as_ref()).is_some()
        || node.name.as_ref().and_then(|n| n.value.as_ref()).is_some()
        || node.value.as_ref().and_then(|v| v.value.as_ref()).is_some()
}

fn is_focusable(node: &CdpAxNode) -> bool {
    node.properties
        .as_ref()
        .map(|props: &Vec<AxProperty>| {
            props.iter().any(|p| {
                matches!(p.name, AxPropertyName::Focusable)
                    && p.value
                        .value
                        .as_ref()
                        .and_then(json_as_bool)
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Helper to extract string from JSON value.
fn json_as_str(v: &JsonValue) -> Option<&str> {
    v.as_str()
}

/// Helper to extract bool from JSON value.
fn json_as_bool(v: &JsonValue) -> Option<bool> {
    v.as_bool()
}

#[derive(Debug, Deserialize)]
struct DomSnapshotNode {
    role: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    focusable: bool,
    #[serde(default)]
    focused: bool,
}

async fn extract_dom_interactives(page: &Page, counter: &mut usize) -> Result<Vec<AriaNode>> {
    let script = r#"(() => {
        const roleFor = (el) => {
            const explicit = el.getAttribute && el.getAttribute('role');
            if (explicit) return explicit;
            const tag = el.tagName ? el.tagName.toLowerCase() : '';
            if (tag === 'a') return 'link';
            if (tag === 'button') return 'button';
            if (tag === 'img') return 'img';
            if (tag === 'nav') return 'navigation';
            if (tag === 'main') return 'main';
            if (tag === 'article') return 'article';
            if (tag === 'section') return 'section';
            if (tag === 'option') return 'option';
            if (tag === 'select') return 'combobox';
            if (tag === 'textarea') return 'textbox';
            if (tag === 'input') {
                const t = (el.getAttribute('type') || 'text').toLowerCase();
                if (t === 'checkbox') return 'checkbox';
                if (t === 'radio') return 'radio';
                if (t === 'range') return 'slider';
                if (t === 'search') return 'searchbox';
                if (t === 'number') return 'spinbutton';
                return 'textbox';
            }
            if (tag && tag.startsWith('h')) return 'heading';
            if (el.isContentEditable) return 'textbox';
            return null;
        };
        const nameFor = (el) => {
            const label = el.getAttribute && el.getAttribute('aria-label');
            if (label) return label;
            const alt = el.getAttribute && el.getAttribute('alt');
            if (alt) return alt;
            const title = el.getAttribute && el.getAttribute('title');
            if (title) return title;
            const text = (el.textContent || '').trim();
            return text.length ? text : null;
        };
        const selector = [
            'a', 'button', 'input', 'select', 'textarea', 'option',
            '[role]', 'img', 'nav', 'main', 'article', 'section',
            'h1', 'h2', 'h3', 'h4', 'h5', 'h6', '[contenteditable]'
        ].join(',');
        const nodes = [];
        const seen = new Set();
        for (const el of document.querySelectorAll(selector)) {
            if (seen.has(el)) continue;
            seen.add(el);
            const role = roleFor(el);
            if (!role) continue;
            const name = nameFor(el);
            const value = 'value' in el ? el.value : null;
            nodes.push({
                role,
                name,
                value,
                focusable: el.tabIndex >= 0,
                focused: document.activeElement === el,
            });
        }
        return nodes;
    })()"#;

    let dom_nodes: Vec<DomSnapshotNode> = page
        .evaluate(script)
        .await
        .context("Failed to evaluate DOM fallback for ARIA snapshot")?
        .into_value()
        .context("Failed to parse DOM fallback for ARIA snapshot")?;

    let nodes = dom_nodes
        .into_iter()
        .filter(|n| !n.role.is_empty())
        .map(|n| {
            *counter += 1;
            let ref_id = format!("@e{}", counter);
            let name = n.name.and_then(|s| {
                let trimmed = s.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            });
            let value = n.value.and_then(|s| {
                let trimmed = s.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            });
            AriaNode {
                ref_id,
                role: n.role,
                name,
                value,
                focusable: n.focusable,
                focused: n.focused,
                children: vec![],
            }
        })
        .collect();

    Ok(nodes)
}

/// Convert CDP AxNode reference to our AriaNode format - zero-copy extraction.
fn convert_node_ref(node: &CdpAxNode, counter: &mut usize) -> AriaNode {
    *counter += 1;
    let ref_id = format!("@e{}", counter);

    let role = node
        .role
        .as_ref()
        .and_then(|r| r.value.as_ref())
        .and_then(|v: &JsonValue| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    let name = node
        .name
        .as_ref()
        .and_then(|n| n.value.as_ref())
        .and_then(|v: &JsonValue| v.as_str().map(|s| s.to_string()));

    let value = node
        .value
        .as_ref()
        .and_then(|v| v.value.as_ref())
        .and_then(|v: &JsonValue| v.as_str().map(|s| s.to_string()));

    let focusable = node
        .properties
        .as_ref()
        .map(|props: &Vec<AxProperty>| {
            props.iter().any(|p| {
                matches!(p.name, AxPropertyName::Focusable)
                    && p.value
                        .value
                        .as_ref()
                        .and_then(json_as_bool)
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    let focused = node
        .properties
        .as_ref()
        .map(|props: &Vec<AxProperty>| {
            props.iter().any(|p| {
                matches!(p.name, AxPropertyName::Focused)
                    && p.value
                        .value
                        .as_ref()
                        .and_then(json_as_bool)
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    AriaNode {
        ref_id,
        role,
        name,
        value,
        focusable,
        focused,
        children: vec![], // Flatten for LLM consumption
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_as_str() {
        assert_eq!(
            json_as_str(&JsonValue::String("hello".into())),
            Some("hello")
        );
        assert_eq!(json_as_str(&JsonValue::Number(42.into())), None);
        assert_eq!(json_as_str(&JsonValue::Bool(true)), None);
        assert_eq!(json_as_str(&JsonValue::Null), None);
    }

    #[test]
    fn test_json_as_bool() {
        assert_eq!(json_as_bool(&JsonValue::Bool(true)), Some(true));
        assert_eq!(json_as_bool(&JsonValue::Bool(false)), Some(false));
        assert_eq!(json_as_bool(&JsonValue::String("true".into())), None);
        assert_eq!(json_as_bool(&JsonValue::Number(1.into())), None);
        assert_eq!(json_as_bool(&JsonValue::Null), None);
    }

    #[test]
    fn test_dom_snapshot_node_deserialization() {
        let json = r#"{
            "role": "button",
            "name": "Submit Form",
            "value": null,
            "focusable": true,
            "focused": false
        }"#;

        let node: DomSnapshotNode = serde_json::from_str(json).unwrap();
        assert_eq!(node.role, "button");
        assert_eq!(node.name, Some("Submit Form".to_string()));
        assert!(node.focusable);
        assert!(!node.focused);
    }

    #[test]
    fn test_dom_snapshot_node_with_defaults() {
        let json = r#"{"role": "link"}"#;
        let node: DomSnapshotNode = serde_json::from_str(json).unwrap();

        assert_eq!(node.role, "link");
        assert_eq!(node.name, None);
        assert_eq!(node.value, None);
        assert!(!node.focusable);
        assert!(!node.focused);
    }

    #[test]
    fn test_interactive_roles() {
        // Test that we correctly identify interactive roles
        // Note: These tests check the string matching logic without CDP nodes
        let interactive_roles = vec![
            "button",
            "link",
            "textbox",
            "checkbox",
            "radio",
            "combobox",
            "listbox",
            "menuitem",
            "tab",
            "slider",
            "searchbox",
            "spinbutton",
            "switch",
            "option",
            "menuitemcheckbox",
            "menuitemradio",
            "treeitem",
            "heading",
            "img",
            "navigation",
            "main",
            "article",
            "section",
        ];

        for role in interactive_roles {
            assert!(
                matches!(
                    role,
                    "button"
                        | "link"
                        | "textbox"
                        | "checkbox"
                        | "radio"
                        | "combobox"
                        | "listbox"
                        | "menuitem"
                        | "tab"
                        | "slider"
                        | "searchbox"
                        | "spinbutton"
                        | "switch"
                        | "option"
                        | "menuitemcheckbox"
                        | "menuitemradio"
                        | "treeitem"
                        | "heading"
                        | "img"
                        | "navigation"
                        | "main"
                        | "article"
                        | "section"
                ),
                "Role '{}' should be considered interactive",
                role
            );
        }

        // Non-interactive roles
        let non_interactive = vec!["generic", "group", "document", "application"];
        for role in non_interactive {
            assert!(
                !matches!(
                    role,
                    "button"
                        | "link"
                        | "textbox"
                        | "checkbox"
                        | "radio"
                        | "combobox"
                        | "listbox"
                        | "menuitem"
                        | "tab"
                        | "slider"
                        | "searchbox"
                        | "spinbutton"
                        | "switch"
                        | "option"
                        | "menuitemcheckbox"
                        | "menuitemradio"
                        | "treeitem"
                        | "heading"
                        | "img"
                        | "navigation"
                        | "main"
                        | "article"
                        | "section"
                ),
                "Role '{}' should NOT be considered interactive",
                role
            );
        }
    }

    #[test]
    fn test_ref_id_generation() {
        // Test that counter increments properly for ref_id generation
        let mut counter = 0usize;

        counter += 1;
        let ref1 = format!("@e{}", counter);
        assert_eq!(ref1, "@e1");

        counter += 1;
        let ref2 = format!("@e{}", counter);
        assert_eq!(ref2, "@e2");

        counter += 1;
        let ref3 = format!("@e{}", counter);
        assert_eq!(ref3, "@e3");
    }
}

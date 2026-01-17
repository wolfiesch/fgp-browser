/**
 * FGP Browser Bridge - Background Service Worker
 *
 * Connects to FGP daemon via WebSocket and exposes Chrome APIs
 * that aren't available through CDP (DevTools Protocol).
 *
 * Architecture:
 *   FGP Daemon <--WebSocket--> Extension <--Chrome APIs--> Browser
 */

const FGP_WS_URL = 'ws://localhost:9223';  // FGP extension bridge port
const RECONNECT_DELAY = 3000;
const FGP_TAB_GROUP_NAME = 'FGP';
const FGP_TAB_GROUP_COLOR = 'blue';
const FGP_EXTENSION_VERSION = '0.1.1';  // Bump on protocol changes

let ws = null;
let fgpTabGroupId = null;
let connectionStatus = 'disconnected';

// ============================================================================
// WebSocket Connection to FGP Daemon
// ============================================================================

function connect() {
  if (ws && ws.readyState === WebSocket.OPEN) return;

  console.log('[FGP] Connecting to daemon at', FGP_WS_URL);
  connectionStatus = 'connecting';

  try {
    ws = new WebSocket(FGP_WS_URL);

    ws.onopen = () => {
      console.log('[FGP] Connected to daemon');
      connectionStatus = 'connected';
      updateBadge('connected');

      // Send hello message
      send({ type: 'hello', version: FGP_EXTENSION_VERSION, capabilities: getCapabilities() });
    };

    ws.onmessage = async (event) => {
      try {
        const request = JSON.parse(event.data);
        const response = await handleRequest(request);
        send({ id: request.id, ...response });
      } catch (err) {
        console.error('[FGP] Error handling message:', err);
        send({ id: request?.id, error: err.message });
      }
    };

    ws.onclose = () => {
      console.log('[FGP] Disconnected from daemon');
      connectionStatus = 'disconnected';
      updateBadge('disconnected');
      ws = null;

      // Auto-reconnect
      setTimeout(connect, RECONNECT_DELAY);
    };

    ws.onerror = (err) => {
      console.error('[FGP] WebSocket error:', err);
      connectionStatus = 'error';
      updateBadge('error');
    };

  } catch (err) {
    console.error('[FGP] Failed to connect:', err);
    connectionStatus = 'error';
    setTimeout(connect, RECONNECT_DELAY);
  }
}

function send(data) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(data));
  }
}

function getCapabilities() {
  return [
    'tabs.create', 'tabs.update', 'tabs.remove', 'tabs.query',
    'tabs.group', 'tabs.ungroup',
    'tabGroups.update', 'tabGroups.query',
    'scripting.executeScript',
    'cookies.get', 'cookies.getAll', 'cookies.set',
    'storage.local', 'storage.sync',
    'notifications.create'
  ];
}

// ============================================================================
// Request Handler - Routes FGP commands to Chrome APIs
// ============================================================================

async function handleRequest(request) {
  const { method, params = {} } = request;

  console.log('[FGP] Handling:', method, params);

  switch (method) {
    // === Tab Management ===
    case 'tabs.create':
      return handleTabCreate(params);
    case 'tabs.update':
      return handleTabUpdate(params);
    case 'tabs.remove':
      return handleTabRemove(params);
    case 'tabs.query':
      return handleTabQuery(params);
    case 'tabs.get':
      return handleTabGet(params);
    case 'tabs.navigate':
      return handleTabNavigate(params);

    // === Tab Groups (CDP can't do this!) ===
    case 'tabs.group':
      return handleTabGroup(params);
    case 'tabs.ungroup':
      return handleTabUngroup(params);
    case 'tabGroups.update':
      return handleTabGroupUpdate(params);
    case 'tabGroups.query':
      return handleTabGroupQuery(params);
    case 'tabGroups.collapse':
      return handleTabGroupCollapse(params);

    // === Page Interaction ===
    case 'scripting.executeScript':
      return handleExecuteScript(params);
    case 'page.snapshot':
      return handlePageSnapshot(params);
    case 'page.click':
      return handlePageClick(params);
    case 'page.fill':
      return handlePageFill(params);

    // === Cookies (with user's real cookies!) ===
    case 'cookies.get':
      return handleCookiesGet(params);
    case 'cookies.getAll':
      return handleCookiesGetAll(params);
    case 'cookies.set':
      return handleCookiesSet(params);

    // === Storage ===
    case 'storage.get':
      return handleStorageGet(params);
    case 'storage.set':
      return handleStorageSet(params);

    // === Notifications ===
    case 'notifications.create':
      return handleNotificationCreate(params);

    // === Utility ===
    case 'health':
      return { ok: true, result: { status: 'healthy' } };
    case 'capabilities':
      return { ok: true, result: getCapabilities() };
    case 'version':
      return { ok: true, result: { version: FGP_EXTENSION_VERSION } };

    default:
      return { ok: false, error: `Unknown method: ${method}` };
  }
}

// ============================================================================
// Tab Management Handlers
// ============================================================================

async function handleTabCreate(params) {
  const { url, active = true, groupWithFgp = true } = params;

  const tab = await chrome.tabs.create({ url, active });

  // Auto-group FGP-created tabs
  if (groupWithFgp) {
    await addTabToFgpGroup(tab.id);
  }

  return { ok: true, result: serializeTab(tab) };
}

async function handleTabUpdate(params) {
  const { tabId, url, active, pinned, muted } = params;
  const updates = {};
  if (url !== undefined) updates.url = url;
  if (active !== undefined) updates.active = active;
  if (pinned !== undefined) updates.pinned = pinned;
  if (muted !== undefined) updates.muted = muted;

  const tab = await chrome.tabs.update(tabId, updates);
  return { ok: true, result: serializeTab(tab) };
}

async function handleTabRemove(params) {
  const { tabId, tabIds } = params;
  const ids = tabIds || [tabId];
  await chrome.tabs.remove(ids);
  return { ok: true, result: { removed: ids } };
}

async function handleTabQuery(params) {
  const tabs = await chrome.tabs.query(params);
  return { ok: true, result: tabs.map(serializeTab) };
}

async function handleTabGet(params) {
  const { tabId } = params;
  const tab = await chrome.tabs.get(tabId);
  return { ok: true, result: serializeTab(tab) };
}

async function handleTabNavigate(params) {
  const { tabId, url } = params;
  const tab = await chrome.tabs.update(tabId, { url });
  return { ok: true, result: serializeTab(tab) };
}

// ============================================================================
// Tab Groups Handlers (THE KEY FEATURE CDP CAN'T DO!)
// ============================================================================

async function handleTabGroup(params) {
  const { tabIds, groupId, createProperties } = params;

  const options = { tabIds };
  if (groupId) options.groupId = groupId;
  if (createProperties) options.createProperties = createProperties;

  const resultGroupId = await chrome.tabs.group(options);
  return { ok: true, result: { groupId: resultGroupId } };
}

async function handleTabUngroup(params) {
  const { tabIds } = params;
  await chrome.tabs.ungroup(tabIds);
  return { ok: true };
}

async function handleTabGroupUpdate(params) {
  const { groupId, title, color, collapsed } = params;
  const updates = {};
  if (title !== undefined) updates.title = title;
  if (color !== undefined) updates.color = color;
  if (collapsed !== undefined) updates.collapsed = collapsed;

  const group = await chrome.tabGroups.update(groupId, updates);
  return { ok: true, result: group };
}

async function handleTabGroupQuery(params) {
  const groups = await chrome.tabGroups.query(params);
  return { ok: true, result: groups };
}

async function handleTabGroupCollapse(params) {
  const { groupId, collapsed = true } = params;
  await chrome.tabGroups.update(groupId, { collapsed });
  return { ok: true };
}

// Helper: Add tab to FGP group (creates group if needed)
async function addTabToFgpGroup(tabId) {
  try {
    // Find or create FGP group
    if (!fgpTabGroupId) {
      const groups = await chrome.tabGroups.query({ title: FGP_TAB_GROUP_NAME });
      if (groups.length > 0) {
        fgpTabGroupId = groups[0].id;
      }
    }

    if (fgpTabGroupId) {
      // Add to existing group
      await chrome.tabs.group({ tabIds: [tabId], groupId: fgpTabGroupId });
    } else {
      // Create new group
      fgpTabGroupId = await chrome.tabs.group({ tabIds: [tabId] });
      await chrome.tabGroups.update(fgpTabGroupId, {
        title: FGP_TAB_GROUP_NAME,
        color: FGP_TAB_GROUP_COLOR
      });
    }
  } catch (err) {
    console.warn('[FGP] Failed to group tab:', err);
  }
}

// ============================================================================
// Page Interaction Handlers
// ============================================================================

async function handleExecuteScript(params) {
  const { tabId, func, args = [], code } = params;

  const results = await chrome.scripting.executeScript({
    target: { tabId },
    func: code ? new Function(code) : func,
    args
  });

  return { ok: true, result: results.map(r => r.result) };
}

async function handlePageSnapshot(params) {
  const { tabId } = params;

  // Execute ARIA tree extraction in page context
  const results = await chrome.scripting.executeScript({
    target: { tabId },
    func: extractAriaTree
  });

  return { ok: true, result: results[0]?.result };
}

async function handlePageClick(params) {
  const { tabId, selector } = params;

  const results = await chrome.scripting.executeScript({
    target: { tabId },
    func: (sel) => {
      const el = document.querySelector(sel);
      if (!el) throw new Error(`Element not found: ${sel}`);
      el.click();
      return true;
    },
    args: [selector]
  });

  return { ok: true, result: { clicked: results[0]?.result } };
}

async function handlePageFill(params) {
  const { tabId, selector, value } = params;

  const results = await chrome.scripting.executeScript({
    target: { tabId },
    func: (sel, val) => {
      const el = document.querySelector(sel);
      if (!el) throw new Error(`Element not found: ${sel}`);
      el.value = val;
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
      return true;
    },
    args: [selector, value]
  });

  return { ok: true, result: { filled: results[0]?.result } };
}

// ============================================================================
// Cookie Handlers (Access real user cookies!)
// ============================================================================

async function handleCookiesGet(params) {
  const { url, name } = params;
  const cookie = await chrome.cookies.get({ url, name });
  return { ok: true, result: cookie };
}

async function handleCookiesGetAll(params) {
  const cookies = await chrome.cookies.getAll(params);
  return { ok: true, result: cookies };
}

async function handleCookiesSet(params) {
  const cookie = await chrome.cookies.set(params);
  return { ok: true, result: cookie };
}

// ============================================================================
// Storage Handlers
// ============================================================================

async function handleStorageGet(params) {
  const { keys, area = 'local' } = params;
  const storage = area === 'sync' ? chrome.storage.sync : chrome.storage.local;
  const data = await storage.get(keys);
  return { ok: true, result: data };
}

async function handleStorageSet(params) {
  const { data, area = 'local' } = params;
  const storage = area === 'sync' ? chrome.storage.sync : chrome.storage.local;
  await storage.set(data);
  return { ok: true };
}

// ============================================================================
// Notification Handler
// ============================================================================

async function handleNotificationCreate(params) {
  const { title, message, iconUrl } = params;
  const id = await chrome.notifications.create({
    type: 'basic',
    iconUrl: iconUrl || 'icons/icon128.png',
    title,
    message
  });
  return { ok: true, result: { notificationId: id } };
}

// ============================================================================
// Utility Functions
// ============================================================================

function serializeTab(tab) {
  return {
    id: tab.id,
    url: tab.url,
    title: tab.title,
    active: tab.active,
    pinned: tab.pinned,
    groupId: tab.groupId,
    windowId: tab.windowId,
    index: tab.index
  };
}

function updateBadge(status) {
  const colors = {
    connected: '#22c55e',    // green
    disconnected: '#6b7280', // gray
    connecting: '#f59e0b',   // yellow
    error: '#ef4444'         // red
  };

  chrome.action.setBadgeBackgroundColor({ color: colors[status] || colors.disconnected });
  chrome.action.setBadgeText({ text: status === 'connected' ? 'ON' : '' });
}

// ARIA tree extraction (runs in page context)
function extractAriaTree() {
  const nodes = [];
  const walker = document.createTreeWalker(
    document.body,
    NodeFilter.SHOW_ELEMENT,
    {
      acceptNode: (node) => {
        const role = node.getAttribute('role') || node.tagName.toLowerCase();
        const isInteractive = ['button', 'link', 'input', 'select', 'textarea', 'a'].includes(role) ||
                              node.hasAttribute('onclick') ||
                              node.tabIndex >= 0;
        return isInteractive ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_SKIP;
      }
    }
  );

  let node;
  let refId = 1;
  while (node = walker.nextNode()) {
    const role = node.getAttribute('role') || node.tagName.toLowerCase();
    const name = node.getAttribute('aria-label') ||
                 node.innerText?.slice(0, 100) ||
                 node.getAttribute('title') || '';

    nodes.push({
      ref_id: `@e${refId++}`,
      role,
      name: name.trim(),
      focusable: node.tabIndex >= 0,
      value: node.value || null
    });
  }

  return { nodes, element_count: nodes.length };
}

// ============================================================================
// Internal Message Handler (for popup communication)
// ============================================================================

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message.type === 'getStatus') {
    sendResponse({ status: connectionStatus });
    return true;
  }

  if (message.type === 'reconnect') {
    if (ws) {
      ws.close();
    }
    ws = null;
    connect();
    sendResponse({ ok: true });
    return true;
  }

  return false;
});

// ============================================================================
// Initialize
// ============================================================================

// Start connection on load
connect();

// Listen for extension icon click
chrome.action.onClicked.addListener(() => {
  if (connectionStatus !== 'connected') {
    connect();
  }
});

console.log('[FGP] Browser Bridge initialized');

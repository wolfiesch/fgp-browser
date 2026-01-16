# FGP Browser Bridge Extension (Prototype)

Chrome extension that bridges FGP daemon to Chrome Extension APIs, enabling features not possible through CDP (Chrome DevTools Protocol).

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    User's Chrome Browser                         │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │              FGP Browser Bridge Extension                 │   │
│  │                                                           │   │
│  │  ┌─────────────┐    ┌──────────────────────────────────┐ │   │
│  │  │  popup.html │    │     background.js (SW)           │ │   │
│  │  │  (Status UI)│    │                                  │ │   │
│  │  └─────────────┘    │  • WebSocket client              │ │   │
│  │                     │  • Chrome API handlers           │ │   │
│  │                     │  • Tab group management          │ │   │
│  │                     └──────────────┬───────────────────┘ │   │
│  └───────────────────────────────────│──────────────────────┘   │
│                                      │                           │
│         Chrome Extension APIs        │    ws://localhost:9223   │
│    (tabs, tabGroups, cookies, etc)   │                           │
└──────────────────────────────────────│───────────────────────────┘
                                       │
                              WebSocket Connection
                                       │
                    ┌──────────────────▼───────────────────┐
                    │         FGP Browser Daemon           │
                    │    (Rust, UNIX socket + WS bridge)   │
                    └──────────────────┬───────────────────┘
                                       │
                              UNIX Socket (FGP Protocol)
                                       │
                    ┌──────────────────▼───────────────────┐
                    │           Claude Code / Agent        │
                    └──────────────────────────────────────┘
```

## Feature Comparison: CDP vs Extension

| Feature | CDP (Current) | Extension (This) | Notes |
|---------|--------------|------------------|-------|
| **Tab Groups** | ❌ Not possible | ✅ Full support | `chrome.tabs.group()`, `chrome.tabGroups.*` |
| **User Sessions** | ⚠️ Requires profile copy | ✅ Native access | No `--user-data-dir` workaround needed |
| **Cookies** | ⚠️ Via Network domain | ✅ `chrome.cookies` API | Cleaner API, all cookies |
| **Notifications** | ❌ Not possible | ✅ `chrome.notifications` | Desktop notifications |
| **Storage Sync** | ❌ Not possible | ✅ `chrome.storage.sync` | Cross-device sync |
| **Navigate** | ✅ Full support | ✅ Full support | Both work well |
| **Screenshots** | ✅ Full support | ⚠️ Limited | CDP is better for screenshots |
| **DOM Access** | ✅ Full CDP | ✅ `scripting.executeScript` | Both work, different APIs |
| **Network Interception** | ✅ Full support | ⚠️ Limited | CDP has richer network APIs |
| **Performance Profiling** | ✅ Full support | ❌ Not available | CDP-only |
| **Browser Launch** | ✅ Spawns new | ❌ Uses existing | Extension runs in user's browser |
| **Headless Mode** | ✅ Supported | ❌ Not applicable | Extension requires headed Chrome |

## New Capabilities Unlocked

### 1. Tab Groups (THE BIG ONE)
```javascript
// Create FGP tab group with all agent-created tabs
await fgp.call('tabs.create', { url: 'https://x.com', groupWithFgp: true });

// Custom groups
const groupId = await fgp.call('tabs.group', {
  tabIds: [tab1, tab2, tab3],
  createProperties: { windowId }
});

await fgp.call('tabGroups.update', {
  groupId,
  title: 'Research',
  color: 'blue'
});
```

### 2. Native User Sessions
No more profile copying! Extension runs in user's actual browser:
```javascript
// Already logged in to Twitter, Gmail, etc.
await fgp.call('tabs.create', { url: 'https://x.com' });
// → Opens user's logged-in Twitter immediately
```

### 3. Cookie Access
```javascript
// Get all Twitter cookies
const cookies = await fgp.call('cookies.getAll', { domain: '.x.com' });

// Set a cookie
await fgp.call('cookies.set', {
  url: 'https://x.com',
  name: 'my_cookie',
  value: 'value123'
});
```

### 4. Desktop Notifications
```javascript
await fgp.call('notifications.create', {
  title: 'FGP Task Complete',
  message: 'Successfully scraped 100 pages'
});
```

### 5. Cross-Device Storage Sync
```javascript
// Store data that syncs across user's Chrome instances
await fgp.call('storage.set', {
  data: { lastTask: 'scrape-twitter' },
  area: 'sync'
});
```

## Installation (Development)

1. Open Chrome → `chrome://extensions`
2. Enable "Developer mode" (top right)
3. Click "Load unpacked"
4. Select `fgp/browser/extension/` directory

## How It Works

1. **Extension loads** → Background service worker starts
2. **Connects to FGP** → WebSocket to `ws://localhost:9223`
3. **Receives commands** → FGP daemon sends requests via WebSocket
4. **Executes Chrome APIs** → Extension calls `chrome.tabs`, `chrome.tabGroups`, etc.
5. **Returns results** → Response sent back via WebSocket

## Daemon-Side Changes Required

The FGP browser daemon needs a WebSocket server to communicate with the extension:

```rust
// In fgp-browser daemon
// Add WebSocket server on port 9223 that:
// 1. Accepts extension connection
// 2. Translates FGP protocol to WebSocket messages
// 3. Routes extension-only methods vs CDP methods
```

## Methods Available

### Tab Management
- `tabs.create` - Create new tab (auto-groups to FGP)
- `tabs.update` - Update tab properties
- `tabs.remove` - Close tab(s)
- `tabs.query` - Find tabs
- `tabs.navigate` - Navigate tab to URL

### Tab Groups (Extension-Only!)
- `tabs.group` - Add tabs to group
- `tabs.ungroup` - Remove from group
- `tabGroups.update` - Set title, color, collapsed
- `tabGroups.query` - Find groups
- `tabGroups.collapse` - Collapse/expand group

### Page Interaction
- `scripting.executeScript` - Run JS in page
- `page.snapshot` - Get ARIA tree
- `page.click` - Click element
- `page.fill` - Fill input field

### Cookies (Extension-Only!)
- `cookies.get` - Get specific cookie
- `cookies.getAll` - Get all matching cookies
- `cookies.set` - Set cookie

### Storage (Extension-Only!)
- `storage.get` - Get stored data
- `storage.set` - Store data (local or sync)

### Notifications (Extension-Only!)
- `notifications.create` - Show desktop notification

## Trade-offs

### Advantages of Extension
- ✅ Native user sessions (no profile copy)
- ✅ Tab groups
- ✅ Cleaner cookie API
- ✅ Desktop notifications
- ✅ Always uses real user browser

### Disadvantages of Extension
- ❌ Requires user to install extension
- ❌ No headless mode
- ❌ Limited network interception
- ❌ No performance profiling
- ❌ Can't spawn isolated browsers for parallel testing

## Hybrid Approach (Recommended)

Use **both** CDP and Extension based on use case:

| Use Case | Best Approach |
|----------|---------------|
| Interactive browsing with user sessions | Extension |
| Automated testing (isolated) | CDP |
| Tab organization | Extension |
| Screenshot/PDF generation | CDP |
| Performance profiling | CDP |
| Cookie manipulation | Extension |
| Parallel browser instances | CDP |

## Status

**Integrated** - Extension bridge is fully integrated with FGP daemon.

Completed:
1. ✅ WebSocket server added to FGP daemon (port 9223)
2. ✅ Extension methods routed via WebSocket bridge
3. ✅ CDP fallback for non-extension methods
4. ⏳ Tab grouping workflow testing (pending)

### Usage

Start daemon with extension bridge enabled:
```bash
browser-gateway start --extension-bridge --foreground
```

Install extension:
1. Open Chrome → `chrome://extensions`
2. Enable "Developer mode"
3. Click "Load unpacked" → Select `fgp/browser/extension/` directory
4. Extension popup shows connection status

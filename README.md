# FGP Browser Gateway

[![CI](https://github.com/fast-gateway-protocol/browser/actions/workflows/ci.yml/badge.svg)](https://github.com/fast-gateway-protocol/browser/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Crates.io](https://img.shields.io/crates/v/fgp-browser.svg)](https://crates.io/crates/fgp-browser)

Fast browser automation daemon using Chrome DevTools Protocol directly. **292x faster** than Playwright MCP, **2.8x faster** than Vercel's agent-browser.

## Why?

MCP stdio tools spawn a new process for every call (~2.3s overhead). FGP Browser keeps Chrome warm and ready:

| Operation | FGP Browser | Playwright MCP | agent-browser |
|-----------|-------------|----------------|---------------|
| Navigate  | **8ms**     | 2,328ms        | 22ms          |
| Snapshot  | **9ms**     | 2,484ms        | 31ms          |
| Screenshot| **30ms**    | 1,635ms        | 34ms          |

Multi-step workflows show even bigger gains:

| Workflow | Steps | FGP | MCP Estimate | Speedup |
|----------|-------|-----|--------------|---------|
| Login flow | 5 | 659ms | 11.5s | **17x** |
| Form submit | 7 | 304ms | 16.1s | **53x** |
| Pagination | 10 | 1,087ms | 23s | **21x** |

## Installation

```bash
# Clone and build
git clone https://github.com/fast-gateway-protocol/browser.git
cd fgp-browser
cargo build --release

# Add to PATH (optional)
cp target/release/browser-gateway ~/.local/bin/
```

**Requirements:**
- Rust 1.70+
- Chrome/Chromium installed

## Quick Start

```bash
# Start the daemon
browser-gateway start

# Navigate to a page
browser-gateway open "https://example.com"

# Get ARIA accessibility tree (for LLM consumption)
browser-gateway snapshot

# Fill a form
browser-gateway fill "input#email" "user@example.com"
browser-gateway fill "input#password" "secret"
browser-gateway click "button[type=submit]"

# Take a screenshot
browser-gateway screenshot /tmp/page.png

# Stop the daemon
browser-gateway stop
```

## CLI Commands

### Core Operations

```bash
browser-gateway open <url>              # Navigate to URL
browser-gateway snapshot                # Get ARIA tree with element refs (@e1, @e2...)
browser-gateway screenshot [path]       # Capture PNG (default: /tmp/screenshot.png)
browser-gateway click <selector>        # Click element (CSS selector or @ref)
browser-gateway fill <selector> <text>  # Fill input field
browser-gateway press <key>             # Press key (Enter, Tab, Escape, etc.)
```

### Form Interactions

```bash
browser-gateway select <selector> <value>    # Select dropdown option
browser-gateway check <selector>             # Check checkbox
browser-gateway check <selector> --uncheck   # Uncheck checkbox
browser-gateway hover <selector>             # Hover over element
browser-gateway scroll <selector>            # Scroll element into view
browser-gateway scroll --y 500               # Scroll down 500px
browser-gateway upload <selector> <path>     # Upload file
browser-gateway press-combo --modifiers Ctrl --key a  # Ctrl+A
```

### Session Management

Multiple isolated browser sessions for parallel workflows:

```bash
browser-gateway session new --id gmail       # Create session
browser-gateway session list                 # List sessions
browser-gateway --session gmail open "https://gmail.com"
browser-gateway --session gmail snapshot
browser-gateway session close --id gmail     # Close session
```

### Daemon Control

```bash
browser-gateway start                  # Start daemon (headless)
browser-gateway start --no-headless    # Start with visible browser
browser-gateway status                 # Check if running
browser-gateway health                 # Detailed health check
browser-gateway stop                   # Graceful shutdown
```

## FGP Protocol

The daemon listens on a UNIX socket at `~/.fgp/services/browser/daemon.sock`.

**Request format (NDJSON):**
```json
{"id": "uuid", "v": 1, "method": "browser.open", "params": {"url": "https://example.com"}}
```

**Response format:**
```json
{"id": "uuid", "ok": true, "result": {"title": "Example"}, "meta": {"server_ms": 8.2}}
```

### Available Methods

| Method | Params | Description |
|--------|--------|-------------|
| `browser.open` | `{url}` | Navigate to URL |
| `browser.snapshot` | `{}` | Get ARIA accessibility tree |
| `browser.screenshot` | `{path?}` | Capture PNG screenshot |
| `browser.click` | `{selector}` | Click element |
| `browser.fill` | `{selector, value}` | Fill input field |
| `browser.press` | `{key}` | Press keyboard key |
| `browser.select` | `{selector, value}` | Select dropdown option |
| `browser.check` | `{selector, checked?}` | Set checkbox state |
| `browser.hover` | `{selector}` | Hover over element |
| `browser.scroll` | `{selector?, x?, y?}` | Scroll page/element |
| `browser.press_combo` | `{key, modifiers[]}` | Key with modifiers |
| `browser.upload` | `{selector, path}` | Upload file |
| `session.new` | `{id}` | Create isolated session |
| `session.list` | `{}` | List active sessions |
| `session.close` | `{id}` | Close session |

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    browser-gateway                       │
├─────────────────────────────────────────────────────────┤
│  FGP Server (concurrent, thread-per-connection)         │
│  └── UNIX Socket: ~/.fgp/services/browser/daemon.sock   │
├─────────────────────────────────────────────────────────┤
│  BrowserService                                          │
│  └── BrowserClient (chromiumoxide)                      │
│      ├── Default Session (shared page)                  │
│      └── Named Sessions (isolated BrowserContext each)  │
├─────────────────────────────────────────────────────────┤
│  Chrome DevTools Protocol (CDP)                         │
│  └── Headless Chrome/Chromium                           │
└─────────────────────────────────────────────────────────┘
```

**Key design decisions:**
- **Direct CDP** (no Playwright abstraction) = minimal latency
- **Single-pass ARIA extraction** = fast accessibility tree
- **BrowserContext isolation** = parallel sessions without interference
- **Thread-per-connection** = concurrent request handling

## Integration with Claude Code

Add to your Claude Code skill or use directly:

```bash
# In your skill's run script
browser-gateway open "$URL"
SNAPSHOT=$(browser-gateway snapshot --json)
# Pass $SNAPSHOT to Claude for element selection
browser-gateway click "@e5"  # Click element ref from snapshot
```

## Performance Tips

1. **Reuse sessions** - Creating sessions has overhead; reuse for related operations
2. **Use element refs** - `@e5` from snapshot is faster than CSS selector lookup
3. **Batch operations** - Chain commands without waiting for Claude between each
4. **Headless mode** - Default; 10-20% faster than visible browser

## Development

```bash
# Build debug
cargo build

# Build release
cargo build --release

# Run tests
cargo test

# Run with debug logging
RUST_LOG=debug ./target/release/browser-gateway start
```

## Dependencies

- [chromiumoxide](https://github.com/nicoulaj/chromiumoxide) - Chrome DevTools Protocol
- [daemon](https://github.com/fast-gateway-protocol/daemon) - FGP server SDK
- [tokio](https://tokio.rs/) - Async runtime
- [clap](https://clap.rs/) - CLI parsing

## License

MIT

## Related

- [daemon](https://github.com/fast-gateway-protocol/daemon) - Core FGP SDK for building daemons
- [FGP Protocol Spec](../protocol/FGP-PROTOCOL.md) - Protocol documentation

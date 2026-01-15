# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2025-01-14

### Added
- Initial release
- Chrome DevTools Protocol integration
- Multi-session support with isolated browser contexts
- ARIA accessibility tree extraction (single-pass, ~9ms)
- Screenshot capture
- Element interaction: click, fill, press, select, check, hover, scroll
- Key combinations with modifiers (Ctrl+A, etc.)
- File upload support
- Session management (new, list, close)
- Health check endpoint
- CLI interface

### Performance
- Navigate: 8ms (292x faster than Playwright MCP)
- ARIA snapshot: 9ms (276x faster)
- Screenshot: 30ms (54x faster)

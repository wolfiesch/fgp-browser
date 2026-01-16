#!/bin/bash
# FGP Browser Connect Mode Helper
# Copies Chrome profile and launches with remote debugging for FGP access
#
# Usage:
#   ./connect-mode.sh start   # Copy profile, launch Chrome, start daemon
#   ./connect-mode.sh stop    # Stop daemon and Chrome
#   ./connect-mode.sh status  # Check status
#   ./connect-mode.sh refresh # Re-copy profile from original (updates sessions)

set -e

# Configuration
CHROME_APP="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
ORIGINAL_PROFILE="$HOME/Library/Application Support/Google/Chrome"
DEBUG_PROFILE="/tmp/fgp-chrome-profile"
DEBUG_PORT=9222
BROWSER_GATEWAY="$(dirname "$0")/../target/release/browser-gateway"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

check_chrome_running() {
    pgrep -x "Google Chrome" > /dev/null 2>&1
}

check_debug_port() {
    curl -s --connect-timeout 2 "http://localhost:$DEBUG_PORT/json/version" > /dev/null 2>&1
}

check_daemon_running() {
    "$BROWSER_GATEWAY" status 2>&1 | grep -q "Status: RUNNING"
}

cmd_start() {
    log_info "Starting FGP Browser Connect Mode..."

    # Check if already running
    if check_debug_port && check_daemon_running; then
        log_info "Already running! Use 'stop' first to restart."
        cmd_status
        return 0
    fi

    # Kill existing Chrome if running (user's regular Chrome)
    if check_chrome_running; then
        log_warn "Chrome is running. It will be closed to enable debug mode."
        read -p "Continue? [y/N] " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            log_info "Aborted."
            exit 1
        fi
        pkill -9 "Google Chrome" 2>/dev/null || true
        sleep 2
    fi

    # Copy profile if it doesn't exist or is stale (>24h old)
    if [ ! -d "$DEBUG_PROFILE" ] || [ "$(find "$DEBUG_PROFILE" -maxdepth 0 -mtime +1 2>/dev/null)" ]; then
        log_info "Copying Chrome profile (preserves your logins)..."
        rm -rf "$DEBUG_PROFILE" 2>/dev/null || true
        cp -r "$ORIGINAL_PROFILE" "$DEBUG_PROFILE"
        log_info "Profile copied to $DEBUG_PROFILE"
    else
        log_info "Using existing profile copy (less than 24h old)"
    fi

    # Launch Chrome with debugging
    log_info "Launching Chrome with remote debugging on port $DEBUG_PORT..."
    "$CHROME_APP" \
        --remote-debugging-port=$DEBUG_PORT \
        --user-data-dir="$DEBUG_PROFILE" \
        --no-first-run \
        > /dev/null 2>&1 &

    # Wait for Chrome to be ready
    for i in {1..10}; do
        if check_debug_port; then
            log_info "Chrome ready!"
            break
        fi
        sleep 1
    done

    if ! check_debug_port; then
        log_error "Chrome failed to start with debugging port"
        exit 1
    fi

    # Stop existing daemon if running
    if check_daemon_running; then
        log_info "Stopping existing daemon..."
        "$BROWSER_GATEWAY" stop 2>/dev/null || true
        sleep 1
    fi

    # Start FGP daemon in connect mode
    log_info "Starting FGP browser daemon in connect mode..."
    "$BROWSER_GATEWAY" start --connect "http://localhost:$DEBUG_PORT"

    sleep 2
    cmd_status
}

cmd_stop() {
    log_info "Stopping FGP Browser Connect Mode..."

    # Stop daemon
    if check_daemon_running; then
        log_info "Stopping FGP daemon..."
        "$BROWSER_GATEWAY" stop 2>/dev/null || true
    else
        log_info "Daemon not running"
    fi

    # Kill Chrome with debug profile
    if check_debug_port; then
        log_info "Stopping debug Chrome instance..."
        # Find Chrome processes using the debug profile
        pkill -f "user-data-dir=$DEBUG_PROFILE" 2>/dev/null || true
        pkill -f "remote-debugging-port=$DEBUG_PORT" 2>/dev/null || true
    else
        log_info "Debug Chrome not running"
    fi

    log_info "Cleanup complete. You can now start regular Chrome normally."
}

cmd_status() {
    echo "=== FGP Browser Connect Mode Status ==="
    echo

    # Chrome status
    if check_debug_port; then
        echo -e "Chrome Debug:  ${GREEN}RUNNING${NC} (port $DEBUG_PORT)"
        curl -s "http://localhost:$DEBUG_PORT/json/version" | grep -o '"Browser":"[^"]*"' | head -1
    else
        echo -e "Chrome Debug:  ${RED}NOT RUNNING${NC}"
    fi

    echo

    # Daemon status
    if check_daemon_running; then
        echo -e "FGP Daemon:    ${GREEN}RUNNING${NC}"
        "$BROWSER_GATEWAY" status 2>&1 | grep -E "Socket:|uptime"
    else
        echo -e "FGP Daemon:    ${RED}NOT RUNNING${NC}"
    fi

    echo

    # Profile status
    if [ -d "$DEBUG_PROFILE" ]; then
        PROFILE_AGE=$((($(date +%s) - $(stat -f %m "$DEBUG_PROFILE")) / 3600))
        echo -e "Profile Copy:  ${GREEN}EXISTS${NC} ($PROFILE_AGE hours old)"
        echo "               $DEBUG_PROFILE"
    else
        echo -e "Profile Copy:  ${YELLOW}NOT FOUND${NC}"
    fi
}

cmd_refresh() {
    log_info "Refreshing profile copy (to sync latest sessions)..."

    # Must stop first
    if check_debug_port || check_daemon_running; then
        log_warn "Stopping services first..."
        cmd_stop
        sleep 2
    fi

    # Remove old profile
    rm -rf "$DEBUG_PROFILE" 2>/dev/null || true

    # Re-copy
    log_info "Copying fresh profile..."
    cp -r "$ORIGINAL_PROFILE" "$DEBUG_PROFILE"

    log_info "Profile refreshed! Run 'start' to begin."
}

# Main
case "${1:-}" in
    start)
        cmd_start
        ;;
    stop)
        cmd_stop
        ;;
    status)
        cmd_status
        ;;
    refresh)
        cmd_refresh
        ;;
    *)
        echo "FGP Browser Connect Mode"
        echo
        echo "Usage: $0 {start|stop|status|refresh}"
        echo
        echo "Commands:"
        echo "  start    Copy Chrome profile, launch with debugging, start FGP daemon"
        echo "  stop     Stop FGP daemon and debug Chrome instance"
        echo "  status   Show current status"
        echo "  refresh  Re-copy Chrome profile to sync latest sessions"
        echo
        echo "This enables FGP to use your logged-in Chrome sessions (Twitter, etc.)"
        exit 1
        ;;
esac

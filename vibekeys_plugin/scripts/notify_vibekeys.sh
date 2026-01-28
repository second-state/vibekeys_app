#!/bin/bash
# Claude Code notification script for VibeKeys controller

VIBEKEYS_APP_URL="${VIBEKEYS_APP_URL:-http://127.0.0.1:57001}"
ACTION="$1"

case "$ACTION" in
    working|stop|pending)
        # Status update - send in background and exit 0
        curl -s -X POST "$VIBEKEYS_APP_URL/status" \
          -H 'Content-Type: application/json' \
          -d "{\"status\":\"$ACTION\"}" >/dev/null 2>&1 &
        exit 0
        ;;
    notify)
        # Notification event - test
        curl -s -X POST "$VIBEKEYS_APP_URL/send" \
          -H 'Content-Type: application/json' \
          -d "{\"message\":\"notify\"}" >/dev/null 2>&1 &
        exit 0
        ;;
    tool)
        # PreToolUse - send "tool use" message and return "ask" decision
        curl -s -X POST "$VIBEKEYS_APP_URL/send" \
          -H 'Content-Type: application/json' \
          -d "{\"message\":\"tool use\"}" >/dev/null 2>&1 &
        echo "{\"permissionDecision\":\"ask\"}"
        exit 0
        ;;
    post)
        # PostToolUse - send "post tool" message
        curl -s -X POST "$VIBEKEYS_APP_URL/send" \
          -H 'Content-Type: application/json' \
          -d "{\"message\":\"post tool\"}" >/dev/null 2>&1 &
        exit 0
        ;;
    ask)
        # PreToolUse - send pending and return "ask" decision
        curl -s -X POST "$VIBEKEYS_APP_URL/status" \
          -H 'Content-Type: application/json' \
          -d "{\"status\":\"pending\"}" >/dev/null 2>&1 &
        echo "{\"permissionDecision\":\"ask\"}"
        exit 0
        ;;
    send|msg)
        # Send custom message
        MESSAGE="$2"
        if [ -z "$MESSAGE" ]; then
            echo "Usage: $0 send <message>" >&2
            exit 1
        fi
        curl -s -X POST "$VIBEKEYS_APP_URL/send" \
          -H 'Content-Type: application/json' \
          -d "{\"message\":\"$MESSAGE\"}" >/dev/null 2>&1 &
        exit 0
        ;;
    *)
        exit 0
        ;;
esac

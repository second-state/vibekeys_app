#!/bin/bash
# Claude Code notification script for BLE controller

BLE_URL="${BLE_URL:-http://127.0.0.1:3000}"
ACTION="$1"

case "$ACTION" in
    working|stop|waiting)
        # Status update - send in background and exit 0
        curl -s -X POST "$BLE_URL/status" \
          -H 'Content-Type: application/json' \
          -d "{\"status\":\"$ACTION\"}" >/dev/null 2>&1 &
        exit 0
        ;;
    notify)
        # Notification event - test
        curl -s -X POST "$BLE_URL/send" \
          -H 'Content-Type: application/json' \
          -d "{\"message\":\"notify\"}" >/dev/null 2>&1 &
        exit 0
        ;;
    tool)
        # PreToolUse - send "tool use" message and return "ask" decision
        curl -s -X POST "$BLE_URL/send" \
          -H 'Content-Type: application/json' \
          -d "{\"message\":\"tool use\"}" >/dev/null 2>&1 &
        echo "{\"permissionDecision\":\"ask\"}"
        exit 0
        ;;
    post)
        # PostToolUse - send "post tool" message
        curl -s -X POST "$BLE_URL/send" \
          -H 'Content-Type: application/json' \
          -d "{\"message\":\"post tool\"}" >/dev/null 2>&1 &
        exit 0
        ;;
    ask)
        # PreToolUse - send waiting and return "ask" decision
        curl -s -X POST "$BLE_URL/status" \
          -H 'Content-Type: application/json' \
          -d "{\"status\":\"waiting\"}" >/dev/null 2>&1 &
        echo "{\"permissionDecision\":\"ask\"}"
        exit 0
        ;;
    *)
        exit 0
        ;;
esac

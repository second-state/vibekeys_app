---
title: "Codex support, one command away"
description: "VibeKeys now flips between Codex and Claude Code in a single command — /review on one key, one-tap approvals — with the confirmation right on the keypad screen."
date: 2026-07-01
tags: [vibekeys, codex, claude-code, release]
---

# Codex support, one command away

VibeKeys works with the AI coding tools you already use — Claude Code, Codex, Cursor, Gemini CLI. Today it gets one-command profiles to flip between Codex and Claude Code:

```bash
vibekeys profile codex     # set the keypad up for Codex
vibekeys profile claude    # switch back to Claude Code
```

## Why this matters

A physical key does exactly one thing — that's the point of a control surface. But "one thing" breaks the moment you use two tools. In Claude Code, `YOLO` toggles allow-all-edits with `Shift+Tab`. In Codex, the move you want is to approve — type `y`. Same key, different job.

So switching is one command, and it round-trips cleanly:

| Profile | `CUSTOM` | `YOLO` |
|---------|----------|--------|
| `codex` | `/review` + Enter — one-key code review | `y` — approve |
| `claude` | `/compact` + Enter | `Shift+Tab` — allow all edits |

Both profiles touch only those two keys. Your mic key, knob, and everything else stay exactly as they were — nothing moves under you.

## You'll know it worked

Run the command and you get one line — on your terminal *and* on the keypad's own screen:

```
✨ You're with Codex now
```

The confirmation only appears once every binding is applied. If the keypad didn't get it, you see the error instead — not a false "done."

## Try it

VibeKeys starts at **$29** (VibeKeys Max, with the OLED status display and wireless remote, is $99), ships worldwide today, and works with the AI coding tools you already use — one keypad for all of them. The `vibekeys` CLI is open source (GPL-3.0).

- **Get one:** [vibekeys.dev](https://vibekeys.dev)
- **The CLI + full command reference:** [github.com/second-state/vibekeys_app](https://github.com/second-state/vibekeys_app#keymap-profiles-switch-between-claude-code--codex)

One command sets it up for Codex. One command switches back.

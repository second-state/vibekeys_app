---
title: "VibeKeys MAX now speaks Codex"
description: "One command reconfigures your VibeKeys MAX for OpenAI Codex — /review on a single key, one-tap approvals — and switches back to Claude Code just as fast."
date: 2026-07-01
tags: [vibekeys, codex, claude-code, release]
---

# VibeKeys MAX now speaks Codex

Your VibeKeys MAX already knows Claude Code. Today it knows OpenAI Codex too — and switching between them takes one command.

```bash
vibekeys profile codex     # set the keyboard up for Codex
vibekeys profile claude    # switch back to Claude Code
```

That's the whole feature. The rest of this post is why it matters.

## The problem with one keyboard and two tools

A physical key does exactly one thing. That's the point of a hardware controller — you press `YOLO` and something happens, no menus, no hunting for a shortcut.

But "one thing" breaks the moment you use two AI coding tools. In Claude Code, `YOLO` toggles allow-all-edits with `Shift+Tab`. In Codex, the move you actually want is to approve — type `y`. Same key, different job. Reconfigure it by hand every time you switch tools and the hardware stops saving you time and starts costing it.

So we stopped making you do that.

## What a profile is

A profile is a named set of key bindings you apply in one command. We ship two:

| Profile | `CUSTOM` | `YOLO` |
|---------|----------|--------|
| `codex` | `/review` + Enter — one-key code review | `y` — approve |
| `claude` | `/compact` + Enter | `Shift+Tab` — allow all edits |

Press `CUSTOM` in a Codex session and you send `/review` — a full code review on one key. Press `YOLO` and you approve. Switch to `claude` and both keys go back to what Claude Code expects.

Both profiles touch only those two keys, so switching is a clean round-trip. Your `MIC`, `ESC`, `ACCEPT`, and the rest keep doing what they always did — nothing else moves under you.

## You'll know it worked

Run the command and you don't get a wall of raw output. You get one line, on your terminal and on the keyboard's own screen:

```
✨ You're with Codex now
```

The confirmation only prints once every binding is actually applied. If the keyboard didn't get it, you see the error instead — not a false "done."

## Try it

Update to the latest `vibekeys`, connect your VibeKeys MAX, and run:

```bash
vibekeys profile codex
```

Open Codex, press `CUSTOM`, and watch `/review` land. When you head back to Claude Code, one `vibekeys profile claude` puts everything back.

Full command reference is in the [README](https://github.com/second-state/vibekeys_app#keymap-profiles-switch-between-claude-code--codex).

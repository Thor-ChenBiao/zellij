# Agent Control Plane Design

This document records a design direction for building an agent-oriented workflow on top of Zellij.

The goal is not to integrate many provider-native desktop apps.
The goal is to standardize on Zellij as the runtime shell and add the missing collaboration primitives directly into Zellij.

Initial target providers:

- Claude Code
- Codex
- Gemini CLI / Gemini Code Assist agent mode

For the first implementation, all of them are treated as programs running inside terminal panes.

## Why Zellij First

This design intentionally chooses a Zellij-first approach.

Reasons:

- Zellij already owns pane input and output
- Zellij already manages PTYs and screen rendering
- Zellij already supports programmatic pane control
- Zellij source is available, so new runtime capabilities can be added directly
- provider differences become much smaller once all agents are normalized to panes

This is a deliberate tradeoff.
Instead of deeply integrating with every provider-native app, we define one terminal runtime and make that runtime smarter.

## Product Thesis

The core of the system is not a panel layout and not a session list.
The core is a message bridge.

Everything else is built on top of the bridge:

- live event streaming
- code visibility
- reviewer autonomy
- human intervention
- pair programming
- future scheduling and supervision

## First Three Workstreams

The first milestone should be split into three parts:

1. `bridge`
2. `reviewer`
3. `view`

These three parts should be implemented in this order.

## Part 1: Bridge

The bridge is the transport and routing layer.

It should be implemented at the pane IO boundary inside Zellij or directly adjacent to it.

### Responsibilities

- capture pane stdin writes
- capture pane stdout / PTY output
- capture rendered screen snapshots
- expose subscriptions for live events
- allow programmatic message injection into a pane
- keep a replayable local event log

### Design Principle

The bridge should be reliable, not intelligent.

It should move and persist messages.
It should not decide what they mean beyond minimal typing and routing.

### Endpoint Model

An endpoint is any live participant connected to the bridge.

Examples:

- a Claude Code pane
- a Codex pane
- a Gemini pane
- a reviewer process
- a human control panel
- a manager board

Suggested endpoint shape:

```json
{
  "endpoint_id": "driver-claude-1",
  "endpoint_type": "zellij_pane",
  "provider": "claude",
  "pane_id": "terminal_7",
  "session_name": "coding-main",
  "role": "driver",
  "repo_root": "/Users/chenbiao/zellij"
}
```

### Channel Model

The bridge should define a small number of typed channels.

Recommended starting channels:

- `pane.stdin.raw`
- `pane.stdout.raw`
- `pane.screen.render`
- `pane.event`
- `human.input`
- `review.comment`
- `pane.control`

Higher-level channels can be derived later:

- `command`
- `command_output`
- `file_change`
- `diff`
- `summary`
- `test_result`

### Message Examples

Human input injected into a driver pane:

```json
{
  "type": "human.input",
  "target_endpoint_id": "driver-claude-1",
  "text": "补一个回归测试，重点看 cursor hidden 分支",
  "send_enter": true
}
```

Rendered screen update from a pane:

```json
{
  "type": "pane.screen.render",
  "source_endpoint_id": "driver-claude-1",
  "pane_id": "terminal_7",
  "text": "⏺ Write(zellij-server/src/tab/mod.rs)\n⏺ Bash(cargo test -p zellij-server ...)",
  "timestamp": "2026-04-03T01:00:00+08:00"
}
```

Review comment produced by a reviewer:

```json
{
  "type": "review.comment",
  "source_endpoint_id": "reviewer-1",
  "target_endpoint_id": "driver-claude-1",
  "severity": "medium",
  "file": "zellij-server/src/tab/mod.rs",
  "category": "missing_test",
  "text": "隐藏光标分支有逻辑改动，但还没有 IME 回归测试。"
}
```

### Capability Model

Every endpoint should declare what it can emit and what it can receive.

Examples:

- a driver pane can emit output and receive text input
- a diff viewer can receive diff data but should not receive arbitrary chat input
- a reviewer can emit review comments and optionally receive control commands

Suggested initial capabilities:

- `can_emit_screen`
- `can_emit_stdout`
- `can_emit_events`
- `can_receive_text_input`
- `can_receive_review_comments`
- `can_receive_control`

### Transport

The first transport should stay local and simple.

Recommended MVP transport:

- local Unix domain socket or loopback TCP
- JSON payloads
- append-only local log for replay

This keeps the first version terminal-first and machine-local.

### Network-ready Design

The first implementation can stay local-only, but the bridge should leave a clean path for LAN collaboration.

Target future scenario:

- two or more people join the same local network room
- one person acts as driver and another as reviewer
- each side may have a local agent attached behind them
- roles can switch during the session

This is effectively pair programming over the bridge.

Because of this, transport must be abstracted early.

Recommended transport progression:

1. local Unix socket
2. loopback TCP
3. LAN WebSocket room transport
4. optional raw TCP transport for lower-overhead deployments

The bridge API should not assume local-only addressing.
Endpoints should be able to carry host or peer metadata from day one.

Preferred default for networked collaboration:

- WebSocket

Reason:

- simpler client integration
- easier browser and desktop support later
- easier room and presence semantics
- easier message inspection during development

Optional later optimization path:

- raw TCP

Reason:

- lower protocol overhead
- useful for specialized high-frequency local-network deployments
- can share the same message schema if framing is kept explicit

Suggested room concepts for later:

- `room_id`
- `peer_id`
- `role`
- `presence`
- `current_target`
- `shared_task_id`

This allows a future LAN mode without redesigning the message model.

### Immediate Zellij Hooks

Even before deeper source changes, Zellij already provides useful control primitives:

- write chars to a pane
- send keys to a pane
- dump current pane screen
- subscribe to pane render updates

These primitives are enough to validate the bridge concept before deeper integration into the PTY path.

## Part 2: Reviewer

The coding agent does not need extra autonomy.
The reviewer or scheduler does.

The reviewer is a sidecar process that watches one or more driver endpoints and decides whether to speak.

### Responsibilities

- watch the driver's recent activity
- inspect changed files and current diff
- notice likely issues
- produce review comments
- optionally send comments back to the driver

### Recommended Modes

The reviewer should support multiple operating modes:

- `observe`
- `suggest`
- `auto-review`
- `gatekeeper`

Recommended MVP default:

- `suggest`

In `suggest` mode, the reviewer does not directly interrupt the driver.
It produces comments that the human can approve or reject.

### Autonomy Requirements

Reviewer autonomy must be controlled.
Otherwise it will become noisy and unhelpful.

The reviewer should not respond to every event.

Suggested policy:

- only consider speaking after a short quiet period
- speak immediately for strong signals:
  - test failures
  - repeated command failures
  - large risky diffs
  - repeated edits in the same area without progress
- deduplicate repeated comments
- apply cooldown per file or issue category

### Reviewer State Machine

Suggested states:

- `idle`
- `observing`
- `thinking`
- `pending_human_approval`
- `sent`
- `cooldown`

### Inputs

The reviewer should subscribe to:

- rendered pane output
- recent pane event stream
- current repo diff
- recently touched files
- latest test outcomes

### Outputs

Reviewer output should be structured rather than free-form only.

Suggested fields:

- severity
- target file
- issue category
- concise feedback
- suggested next action

This structure makes it easier to render comments, route them, and deduplicate them.

## Part 3: View

The human needs a dedicated observation layer.

One panel is not enough because two different questions need to be answered:

- what is the agent doing right now?
- what code changed?

These should be treated as different views.

### View 1: Events

Purpose:

- show live activity
- show current commands
- show recent file actions
- show failures and warnings

This is the operational view.

Suggested command surface:

- `zellij view events`

### View 2: Code

Purpose:

- show the latest touched file
- show the current diff
- show changed files
- show the current file with change markers

This is the code visibility view.

Suggested command surface:

- `zellij view code`

### Optional Later View: Board

Purpose:

- supervise multiple drivers and reviewers at once
- show blocked states
- show pending reviews
- show overlap conflicts

Suggested later command surface:

- `zellij view board`

## Pair Programming Model

This design naturally supports a driver/reviewer/human workflow.

Roles:

- `driver`: the main coding agent
- `reviewer`: a sidecar agent that watches and critiques
- `human`: the deciding authority

Flow:

1. Human gives requirement to the driver.
2. Driver works inside a pane.
3. Bridge captures driver activity.
4. Reviewer watches the bridge and generates feedback.
5. Human approves, edits, or rejects feedback.
6. Approved feedback is sent back through the bridge into the driver pane.
7. Driver iterates.

This is the first meaningful form of pair programming on top of Zellij.

## Suggested Naming

Use short, literal names:

- `bridge`
- `reviewer`
- `view events`
- `view code`
- later `view board`

The names should reflect responsibilities directly.

## MVP Sequence

The first milestone should not try to solve everything.

Suggested implementation order:

1. Build bridge registration, subscribe, snapshot, and send primitives
2. Persist bridge events locally for replay
3. Build `view events`
4. Build `view code`
5. Build reviewer in `suggest` mode
6. Add reviewer-to-driver injection with human approval

## Non-goals For the First Version

The first version should not try to do the following:

- deep native integration with every provider's desktop app
- cross-machine orchestration
- fully automatic reviewer takeovers
- advanced scheduling across many tasks
- web UI

Those can be added later after the local Zellij-based flow proves useful.

## Future Extension: LAN Pair Programming

The bridge should eventually support a lightweight LAN mode.

Goal:

- one teammate can drive
- another teammate can review
- both can attach their own local agents behind their role
- the human collaboration loop works across machines, not just across panes

This should be treated as a transport extension, not a redesign of the product model.

The same primitives still apply:

- bridge
- reviewer
- view
- driver / reviewer / human roles

Only the transport changes from local-only to network-visible.

For that network-visible phase, the preferred first transport should be WebSocket.
Raw TCP can remain an optimization path rather than the default.

## Summary

The first useful version of this system should be:

- Zellij-first
- bridge-centered
- terminal-local
- human-in-the-loop
- reviewer-assisted

The bridge is the base layer.
The reviewer is the first intelligent sidecar.
The views are the first human-facing product surface.

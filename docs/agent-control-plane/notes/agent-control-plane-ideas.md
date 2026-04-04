# Agent Control Plane Notes

Updated: 2026-04-03

## Purpose

Build a local-first control plane for coding agents that can:

- monitor multiple agent sessions in parallel
- summarize progress, blockers, commands, and file changes
- support human-in-the-loop review and intervention
- support pair programming between two agents
- work across Claude Code, Codex, and Gemini CLI / Gemini Code Assist agent mode

This document captures product requirements and architecture ideas only.
It is not an implementation plan yet.

## Product Thesis

The system should manage tasks, evidence, and control flow rather than raw terminal sessions.

Stronger refinement after discussion:

The true core is not the UI and not the session manager.
The true core is a `message bridge`.

Everything else is built on top of it:

- event streams
- live file visibility
- review loops
- pair programming
- human intervention
- session supervision
- terminal / IDE / web views

Primary managed object:

- `task`

Supporting runtime objects:

- `agent_session`
- `worktree`
- `thread`
- `review`
- `handoff`
- `event_stream`

## Core User Needs

### 1. Multi-agent supervision

The user may run many coding agents at once.
They want one manager view that answers:

- Which agents are active?
- What is each agent trying to do?
- What files changed recently?
- What commands ran recently?
- Which agent is blocked?
- Which agent is ready for review?
- Which agents are touching overlapping files?

### 2. Continuous evidence stream

The user wants a constantly updating stream, not just periodic summaries.
The stream should include:

- command execution
- file reads
- file writes / edits
- search / grep / glob actions
- review requests
- errors / failures
- approvals / interruptions
- test runs

### 3. Pair programming between agents

The user wants two agents to collaborate on one programming task.

Example pattern:

- left agent discusses requirements with the human and implements
- right agent continuously watches progress and reviews
- when left reaches a checkpoint, right can submit review feedback
- review feedback can be injected back into the left agent thread
- the human can intervene at any time on either side

This is not just monitoring.
It requires a bidirectional message and review loop.

### 4. Human live visibility

The human must be able to continuously see:

- what the agent is reading
- what file the agent is currently editing
- what commands the agent is executing
- what changed in the file
- what review comments are pending

This is a first-class requirement, not a debugging extra.

## Supported Agent Targets

### Claude Code

Observed strengths:

- strong hook system
- easy local event emission
- session-oriented terminal workflow
- subagents available

Practical integration strategy:

- use hooks for `PreToolUse`, `PostToolUse`, `PostToolUseFailure`
- emit local JSONL events
- maintain a lightweight session state file

Useful for:

- live tool stream
- latest edited file
- local supervision dashboard

### Codex

Observed strengths:

- local persistent SQLite state already exists
- app-server protocol supports structured thread / turn / item events
- app-server can expose command execution and file change lifecycle
- supports hooks as well

Practical integration strategy:

- prefer `codex app-server` as the realtime event source
- use local SQLite only for indexing, replay, and historical summaries
- use hooks only as a lightweight fallback

Useful for:

- rich structured supervision
- thread-level orchestration
- future web UI / control plane

### Gemini CLI / Gemini Code Assist agent mode

Observed strengths:

- Gemini CLI is an open-source terminal agent
- Gemini Code Assist agent mode is powered by Gemini CLI
- supports MCP and agent-mode workflows

Practical integration strategy:

- build an adapter around Gemini CLI stdout / transcript / command wrappers
- if Gemini exposes extension or event hooks later, switch adapter to native events
- treat Gemini Code Assist agent mode as a Gemini-backed worker with reduced surface area

Useful for:

- third provider parity
- IDE pair-programming scenarios
- enterprise environments that standardize on Google tooling

## Architecture Direction

### 0. Message Bridge First

The bridge should not be tied to:

- a specific terminal multiplexer
- a specific pane
- a specific provider
- a specific UI surface

It should exist as an independent routing and observation layer.

The bridge should expose:

- registration
- discovery
- subscription
- write / inject
- history / replay
- capability inspection
- input / output stream observation

The right mental model is:

- not "a pane talks to another pane"
- but "a producer publishes typed messages to a bridge, and consumers subscribe or write back through registered channels"

### 0.1 Core bridge concepts

Recommended core objects:

- `endpoint`
  one live participant, eg Claude session, Codex thread, Gemini worker, human reviewer UI
- `channel`
  one logical stream, eg `events`, `diff`, `review`, `control`, `stdout`, `stdin`
- `message`
  one typed payload on a channel
- `subscription`
  one listener attached to one or more channels with optional filters
- `capability`
  declaration of what an endpoint can emit and what it can accept
- `route`
  a rule that forwards or transforms messages from one channel to another

### 0.2 Required endpoint capabilities

Every registered endpoint should declare a capability manifest.

At minimum:

- `can_emit_events`
- `can_emit_stdout`
- `can_emit_file_changes`
- `can_emit_commands`
- `can_accept_text_input`
- `can_accept_review_comments`
- `can_accept_control_commands`
- `can_pause`
- `can_resume`
- `can_stop`

This avoids hardcoding behavior per provider.

### 0.3 Required channel types

Minimum channels:

- `event`
- `command`
- `command_output`
- `file_change`
- `diff`
- `review`
- `summary`
- `control`
- `human_input`
- `agent_output`

Later channels:

- `approval`
- `test_result`
- `artifact`
- `conflict`
- `handoff`

### 0.4 Read / write distinction

This is critical.

Some channels are read-only from the outside:

- internal telemetry
- raw stdout
- raw file-change feed

Some channels are writable:

- human input
- review feedback
- control commands

Some are duplex:

- interactive chat thread
- pair-programming review loop

The bridge must explicitly track:

- which endpoints are readable
- which endpoints are writable
- which message types are accepted by each writable endpoint

### 0.5 Registration and discovery

The system should be able to answer in realtime:

- what endpoints currently exist
- which are online
- which channels each endpoint exposes
- which channels are writable
- which subscriptions are active
- which routes are currently forwarding messages

This enables dynamic supervision and ad hoc pairing.

### 0.6 Routing model

Examples:

- Claude driver `file_change` -> review dashboard
- Claude driver `event` -> manager summary
- reviewer `review` -> driver `human_input`
- human `control` -> driver `control`
- Codex `commandExecution` -> unified `command` channel
- Gemini output stream -> unified `agent_output` channel

### 0.7 Injection model

The bridge should support safe message injection into live endpoints.

Typical write operations:

- send freeform text into a session
- send structured review feedback
- send "stop / continue / summarize / test" control commands

Recommended rule:

- default write path goes through an approval gate when target is an active driver

## Message Bridge Design Goals

- provider-agnostic
- UI-agnostic
- transport-agnostic
- typed messages
- explicit capabilities
- replayable history
- human-in-the-loop by design
- easy to add many reviewers or observers

### 1. Unified Event Model

Each provider should normalize into a common event schema:

- `provider`: `claude | codex | gemini`
- `session_id`
- `thread_id`
- `task_id`
- `repo_root`
- `cwd`
- `event_type`
- `event_status`
- `tool_name`
- `summary`
- `payload_ref`
- `created_at`

Recommended event types:

- `user_message`
- `assistant_summary`
- `tool_start`
- `tool_ok`
- `tool_fail`
- `file_change`
- `command_output`
- `review_requested`
- `review_submitted`
- `blocked`
- `handoff`
- `completed`

### 2. Storage Layers

Layer A: append-only event log

- JSONL
- easiest for local prototyping
- easy to tail in terminal

Layer B: query store

- SQLite
- indexes by session, repo, task, file, status
- enables manager summaries and analytics

Layer C: transport

- local daemon or websocket
- needed only when moving beyond one machine / one terminal

## Pair Programming Design

### Roles

- `driver`: main coding agent
- `reviewer`: second agent watching, checking, critiquing, suggesting
- `human`: product / architecture / approval authority

### Shared objects

- same task id
- separate session ids
- ideally separate worktrees or at least separate interaction channels

### Event flow

1. Human explains requirement to driver.
2. Driver starts implementing.
3. Reviewer subscribes to driver's event stream and diff stream.
4. Reviewer sees:
   - latest file changes
   - command history
   - tests run
   - intermediate summaries
5. Reviewer emits structured feedback:
   - bug risk
   - missing test
   - design concern
   - simplification suggestion
6. Human chooses one:
   - send review back to driver
   - edit review before sending
   - ignore review
7. Driver receives review as a new message in its own thread.
8. Driver iterates.

### Key UX requirement

The reviewer should not only watch.
It should be able to push comments back into the driver thread.

That requires a message bridge:

- source: reviewer output
- transform: review-to-driver message formatter
- destination: driver session input channel

This should not be implemented as a blind keyboard macro if it can be avoided.
The better abstraction is:

- reviewer publishes `review.comment`
- bridge transforms it into the target provider's accepted input shape
- bridge injects it through a registered writable endpoint

### Safety requirement

The reviewer should default to read-only over the driver's work.
Otherwise both agents can thrash the same code.

Recommended operating modes:

- `watch-only`
- `review-and-comment`
- `review-and-take-over` only after human approval

## Control Plane UI Ideas

### Terminal MVP

Main panes:

- worker panes: running Claude / Codex / Gemini
- event stream pane: realtime tools and file changes
- manager pane: status board
- review pane: latest diff / file view

Bridge-first interpretation:

- panes are just viewers or controllers over bridge channels
- the bridge itself must survive if panes are closed or moved
- different terminal tools or IDE panes can attach to the same bridge

Manager board should show:

- active sessions
- task title
- provider
- repo
- files touched
- last command
- last file changed
- last heartbeat
- current state

### Later web UI

Views:

- kanban by task status
- timeline per agent
- diff viewer
- command transcript viewer
- review queue
- conflict detector

## State Machine

Per task:

- `planning`
- `coding`
- `waiting_review`
- `blocked`
- `needs_human`
- `ready_to_apply`
- `done`
- `failed`

Per session:

- `starting`
- `idle`
- `running`
- `waiting_input`
- `waiting_approval`
- `error`
- `offline`

## Conflict Detection

Useful automatic warnings:

- two agents touched the same file recently
- agent changed many files without tests
- agent is running commands repeatedly with no new file changes
- no heartbeat for N minutes
- repeated failures on same command
- reviewer raised issue but driver has not responded

## Suggested MVP Scope

Do first:

- message bridge spec
- unified event schema
- Claude adapter
- Codex adapter
- Gemini adapter
- terminal event stream
- manager summary view
- pair-programming message bridge

Do later:

- web UI
- remote multi-machine transport
- auto conflict resolution
- auto reviewer assignment

## Likely Integration Strategy

### Claude adapter

- native hooks
- local JSONL emitter

### Codex adapter

- app-server first
- SQLite as historical index
- hooks only as fallback

### Gemini adapter

- wrapper around CLI invocation and transcript capture initially
- migrate to native events if Google exposes stable hook/event APIs

## Open Questions

- What is the cleanest injection path back into a live Claude / Codex / Gemini session?
- Which providers support safe programmatic message injection today?
- How much of pair programming should be explicit human approval vs automatic loopback?
- Should reviewer agents share the same worktree or separate worktrees?
- Should manager be a read-only observer or an active supervisor agent?

## Immediate Product Direction

The strongest near-term concept is:

- a local terminal-first multi-agent control plane
- provider adapters for Claude / Codex / Gemini
- a provider-agnostic message bridge at the center
- pair programming via driver/reviewer roles
- manager view built on normalized event streams

This feels closer to a practical product than a toy script.

## Source Notes

These references informed the current design direction:

- Claude Code hooks and statusline docs for event emission and terminal integration
- Codex hooks docs: hooks exist but current `PreToolUse` / `PostToolUse` runtime is effectively Bash-only
- Codex advanced config docs: `hooks.json` lives at `~/.codex/hooks.json` or `<repo>/.codex/hooks.json` and requires `features.codex_hooks = true`
- Codex app-server docs: structured thread / turn / item events include `commandExecution` and `fileChange`, which is stronger than plain hooks
- Gemini Code Assist agent mode docs: Google explicitly frames agent mode as a pair programmer and supports commenting on, editing, and approving plans and tool use
- Gemini CLI GitHub repo: open-source terminal agent with MCP support and active roadmap

## Current Feasibility Assessment

Claude Code:

- very good for local event capture today
- less obvious as the long-term central control plane

Codex:

- strongest current foundation for a true control plane
- app-server plus local state makes orchestration more realistic

Gemini:

- strategically important for compatibility
- likely needs an adapter first, then deeper integration as native event surfaces mature

## Zellij-first Pivot

After discussion, the preferred product direction is:

- stop adapting many provider-native desktop apps
- standardize on Zellij as the runtime shell
- treat all agents as terminal endpoints running inside Zellij panes
- extend Zellij itself because source is available and behavior can be normalized

This changes the architecture substantially.

The first implementation should be Zellij-first, provider-second.

### Why this is attractive

- one terminal runtime
- one IO model
- one capture mechanism
- one injection mechanism
- provider differences get flattened at the pane boundary
- source is available, so we can add bridge primitives directly into the multiplexer

### First three concrete workstreams

1. IO bridge
2. autonomous reviewer
3. human-facing views

## Zellij-first Architecture

### 1. IO bridge

This is the foundational layer.

Zellij already sits between the keyboard and the pane PTY.
So the bridge should be implemented at the pane IO boundary.

Required capabilities:

- capture pane stdin writes
- capture pane stdout / PTY output
- expose current rendered screen snapshot
- allow programmatic input injection into a target pane
- allow subscription to live updates per pane

### 1.1 Recommended bridge channels

- `pane.stdin.raw`
- `pane.stdout.raw`
- `pane.screen.render`
- `pane.event`
- `pane.control`
- `review.comment`
- `human.input`

### 1.2 Recommended bridge transport

For MVP:

- local Unix domain socket or local TCP loopback
- JSON messages
- append-only local event log for replay

Potential command surface:

- `zellij bridge start`
- `zellij bridge register`
- `zellij bridge list`
- `zellij bridge subscribe`
- `zellij bridge send`

### 1.3 Bridge message examples

Input injection:

```json
{
  "type": "human.input",
  "target_pane_id": "terminal_7",
  "text": "补一个回归测试，重点看 cursor hidden 分支",
  "send_enter": true
}
```

Observed pane output:

```json
{
  "type": "pane.screen.render",
  "pane_id": "terminal_7",
  "session_name": "coding-main",
  "text": "⏺ Write(tab/mod.rs)\n⏺ Bash(cargo test -p zellij-server ...)",
  "timestamp": "2026-04-03T01:00:00+08:00"
}
```

### 1.4 Why bridge before semantics

The bridge should capture generic terminal facts first:

- bytes in
- bytes out
- rendered screen

Higher-level semantics such as:

- command execution
- file change
- test failure
- review request

should be derived on top.

That keeps the base layer provider-agnostic.

## Autonomous Reviewer

The coding agent itself does not need extra autonomy.
The reviewer or scheduler does.

### 2. Reviewer role

The reviewer subscribes to:

- driver pane screen updates
- recent event stream
- current git diff
- recently touched files

Then decides whether to:

- stay silent
- emit a review suggestion
- send a question
- ask for human approval before sending

### 2.1 Reviewer modes

- `observe`
- `suggest`
- `auto-review`
- `gatekeeper`

Recommended MVP default:

- `suggest`

This means:

- reviewer generates comments
- comments go to a pending queue
- human can approve sending them back to the driver

### 2.2 Reviewer autonomy policy

Reviewer should not speak on every event.
It needs debounce, thresholds, and memory.

Suggested rules:

- only consider speaking after quiet period, eg 10 to 20 seconds
- speak immediately on strong triggers:
  - test failure
  - dangerous command
  - wide diff expansion
  - same file edited repeatedly without progress
- deduplicate similar comments
- cooldown per file / topic

### 2.3 Reviewer state machine

- `idle`
- `observing`
- `thinking`
- `pending_human_approval`
- `sent`
- `cooldown`

### 2.4 Reviewer outputs

Reviewer should emit structured comments:

- severity
- target file
- issue category
- concise feedback
- suggested next action

Example:

```json
{
  "severity": "medium",
  "target_file": "zellij-server/src/tab/mod.rs",
  "category": "missing_test",
  "summary": "cursor hidden 分支仍缺一条直接回归测试",
  "next_action": "补一个最小 focused test"
}
```

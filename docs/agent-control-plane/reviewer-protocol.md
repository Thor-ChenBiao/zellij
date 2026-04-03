# Reviewer Autonomy and Driver Communication

This document defines how the reviewer should behave, how it learns its identity, and how it communicates with the driver agent through the bridge.

The goal is to make reviewer behavior predictable and composable rather than magical.

## Core Decisions

- reviewer is a first-class endpoint in the shared room
- bridge is the source of truth for identity, routing, and message typing
- reviewer does not talk to the driver directly by discovering pane IDs on its own
- reviewer talks through the bridge using room-scoped messages
- the driver should always be able to tell that a message came from a reviewer rather than a human

## Actor Model

There are three distinct actors:

- `driver`
  the coding agent actively writing code
- `reviewer`
  the sidecar agent that watches the driver and produces review feedback
- `human`
  the operator who can pause, approve, redirect, or override both

The bridge sits between them and does four things:

- capture activity
- build typed review context
- route reviewer output
- preserve human intervention points

## How Reviewer Learns Its Identity

Reviewer identity should not rely on implicit prompt wording alone.
It should be established by the bridge during startup.

Startup flow:

1. `zellij reviewer [id]` creates or joins a shared room
2. bridge registers a new endpoint with:
   - `room_id`
   - `endpoint_id`
   - `provider`
   - `role=reviewer`
   - `target_role=driver`
3. bridge generates a reviewer bootstrap packet
4. that bootstrap packet is injected as the first message into the reviewer pane
5. reviewer should treat all later review requests as belonging to that role until a new bootstrap packet arrives

The bridge should also persist the bootstrap payload in room metadata so the reviewer identity can be replayed after reconnect.

## Reviewer Bootstrap Contract

The bootstrap packet should tell the reviewer:

- who it is
- which room it belongs to
- who it is reviewing
- what counts as good feedback
- what output format it must use
- whether human approval is required before feedback is injected

Suggested bootstrap packet injected into the reviewer:

```text
[ACP_BOOTSTRAP_BEGIN]
room_id: 482731
endpoint_id: reviewer-7c31
role: reviewer
provider: claude
target_role: driver
mode: approval
human_override: enabled
message_schema: ACP_REVIEW_RESPONSE_V1

You are the reviewer for this shared coding room.
Your job is to review the driver's work for correctness, regressions, missing tests, risky assumptions, and incomplete follow-through.

Rules:
- do not start implementing code unless explicitly asked
- do not respond to every event
- wait for a review window
- prefer concise, actionable feedback
- produce output only in the required review response schema
[ACP_BOOTSTRAP_END]
```

This packet is the canonical answer to "how does reviewer know it is a reviewer?"

## How Driver Learns a Message Is From Reviewer

The driver should not receive raw, ambiguous text.
Every injected review must carry a stable bridge envelope.

Suggested driver-facing injection format:

```text
[ACP_MESSAGE_BEGIN]
room_id: 482731
message_type: review_feedback
source_role: reviewer
source_endpoint: reviewer-7c31
target_role: driver
sequence: 14
delivery_mode: approval
severity: medium
summary: Missing regression coverage for hidden-cursor IME behavior
[ACP_MESSAGE_END]

[Review from reviewer-7c31]
- Add a regression test for the hidden-cursor IME anchor path.
- Verify the host cursor move is not emitted redundantly.
- Re-run the relevant focused test after the patch.
```

This gives the driver a clear identity marker and a stable message boundary.

## Reviewer and Driver Should Not Speak Free-Form by Default

The bridge should standardize two message types:

- `review_request`
- `review_feedback`

Internally the bridge can store them as JSON.
When injected into panes, they should be rendered as readable text envelopes.

This keeps two layers:

- machine layer for routing and replay
- agent layer for prompt consumption

## Review Request Protocol

The bridge should not dump the full raw transcript every time.
It should build a bounded review bundle.

Suggested request fields:

- room metadata
- driver endpoint identity
- changed files
- last command(s)
- last command result
- last relevant output excerpt
- review reason
- current mode

Suggested reviewer request packet:

```text
[ACP_REVIEW_REQUEST_BEGIN]
room_id: 482731
request_id: rr-0014
driver_endpoint: claude-driver-9b1f
target_role: driver
mode: approval
reason: driver_turn_closed

changed_files:
- zellij-server/src/tab/mod.rs
- zellij-server/src/output/mod.rs

last_commands:
- cargo test hidden_cursor -- --nocapture

recent_output:
Driver updated IME cursor handling and reran focused tests.

task:
Review the latest driver changes.
Return one ACP_REVIEW_RESPONSE_V1 block only.
[ACP_REVIEW_REQUEST_END]
```

## Review Response Protocol

Reviewer output should be parseable and stable.
It should not be an open-ended essay.

Suggested response format:

```text
[ACP_REVIEW_RESPONSE_V1]
room_id: 482731
request_id: rr-0014
source_endpoint: reviewer-7c31
target_role: driver
severity: medium
confidence: high
should_send: true
summary: The change is directionally correct but still lacks regression coverage.

findings:
- No focused regression test covers the hidden-cursor branch.

actions:
- Add a regression test for the hidden-cursor IME path.
- Confirm host cursor updates are emitted only when the anchor changes.

notes:
- Current implementation shape looks small and localized.
[/ACP_REVIEW_RESPONSE_V1]
```

This gives the bridge something deterministic to parse and route.

## Reviewer Autonomy Model

Reviewer should be autonomous, but bounded.

It should not inject comments after every keystroke.

### Reviewer States

- `bootstrapping`
- `observing`
- `collecting_context`
- `reasoning`
- `ready_to_send`
- `waiting_human_approval`
- `cooldown`
- `paused`

### State Meaning

- `bootstrapping`
  reviewer has not yet received its role contract
- `observing`
  reviewer is subscribed to driver activity but is not preparing a review
- `collecting_context`
  bridge has decided a review window exists and is assembling evidence
- `reasoning`
  reviewer is preparing a structured response
- `ready_to_send`
  reviewer has produced a valid review packet
- `waiting_human_approval`
  review packet exists but has not yet been injected to driver
- `cooldown`
  reviewer recently sent feedback and should avoid spamming
- `paused`
  human intervention has halted automatic routing

### Review Triggers

Reviewer should wake up on bounded triggers only:

- driver turn appears complete
- command exits with failure
- test run finishes
- high-risk files changed
- human explicitly asks for review
- bridge idle timer expires after a burst of activity

### Suggested Turn-Close Heuristic

For MVP, use a simple heuristic:

- last driver output observed
- no new driver output for `N` seconds
- no active command still running

A good first value is `2-4` seconds of quiet time.

Later this can be improved using richer pane/PTY state.

## Anti-Spam Rules

Reviewer should follow these limits:

- at most one outstanding review packet per driver turn
- dedupe similar findings against the last sent review
- suppress low-value comments during cooldown
- allow urgent findings to bypass cooldown if severity is high

This is the main mechanism that makes autonomy useful instead of noisy.

## Human Intervention Model

Human must remain above the reviewer in the control hierarchy.

Default operating modes:

- `manual`
  reviewer drafts only, nothing is auto-injected
- `approval`
  reviewer drafts and bridge waits for human approval
- `auto`
  reviewer drafts and bridge injects automatically unless paused

Recommended default:

- `approval`

### Esc Semantics

`Esc` should not be interpreted as "kill reviewer".
It should be interpreted as "enter manual hold for this room".

Effects of manual hold:

- stop automatic reviewer-to-driver injection
- continue capturing events
- optionally continue letting reviewer draft packets
- require explicit human resume before automatic routing continues

This makes human takeover predictable.

## Routing Model

Reviewer should not target arbitrary panes by itself.
The bridge resolves targets.

Suggested routing rules:

- reviewer emits `target_role=driver`
- bridge resolves the active driver endpoint for that room
- bridge injects to that driver endpoint

If there are multiple drivers in a room later, the bridge can require:

- explicit `target_endpoint`
- or `active_driver_endpoint`

For the first version, `target_role=driver` is enough.

## Identity and Trust Boundary

The bridge is authoritative for message metadata.
The agents are not.

That means:

- bridge assigns endpoint IDs
- bridge stamps message source and destination
- bridge tracks sequence numbers
- bridge decides whether a message is approved for injection

Agents only consume the rendered envelope.

This prevents "pretend reviewer" ambiguity inside the room.

## MVP Implementation Order

Reviewer autonomy should be implemented in this order:

1. inject reviewer bootstrap packet on startup
2. define `ACP_REVIEW_REQUEST` and `ACP_REVIEW_RESPONSE` parsing
3. detect basic driver turn-close windows
4. deliver review requests to reviewer
5. buffer reviewer output in approval mode
6. inject approved reviewer feedback back to driver
7. add pause/resume semantics

## Practical Result

After this protocol exists, the interaction loop becomes:

1. driver writes code
2. bridge captures driver evidence
3. bridge decides a review window exists
4. reviewer receives a structured review request
5. reviewer emits a structured review response
6. bridge either buffers or injects it
7. driver receives a clearly identified reviewer message
8. human can interrupt at any point

That is the simplest version of pair programming with:

- one coding agent
- one reviewing agent
- one supervising human

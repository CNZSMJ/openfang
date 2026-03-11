# Async Multi-Agent Task Dispatch Design

**Version**: 1.0  
**Date**: 2026-03-12  
**Status**: Proposed

---

## 1. Background

The current multi-agent model in OpenFang supports:

- synchronous inter-agent calls through `agent_send`
- shared task queue primitives through `task_post/task_claim/task_complete`
- background execution through triggers, cron, and autonomous loops

However, it does **not** provide a full asynchronous delegation model where:

- a main agent can delegate work to child agents and continue talking to the user immediately
- delegated work can report progress over time
- users can ask for progress at any time
- progress and completion updates can be delivered back through all supported channels
- multiple task updates arriving in a short period can be aggregated into one assistant-style progress report

This design fills that gap.

---

## 2. Problem Statement

We want a main agent to behave like a real assistant:

- it can split work into multiple sub-tasks
- it can continue the user conversation while those tasks run in the background
- it can proactively report progress
- it can answer user questions about progress on demand
- it should not spam the user with many fragmented updates when multiple tasks progress at once

The existing synchronous model blocks the current turn. The existing task queue stores work, but does not provide execution orchestration, conversational routing, progress visibility, or aggregated reporting.

---

## 3. Goals

### Product Goals

- Enable true async task delegation from a main agent to one or more child agents.
- Let the main agent remain available for new user messages while child tasks execute.
- Make task progress visible to both the main agent and the user.
- Support both proactive progress reporting and user-initiated progress queries.
- Deliver updates consistently across WebSocket, WebChat, and non-WS channels.
- Aggregate multiple task updates into one assistant-quality progress report.

### Technical Goals

- Preserve session consistency and avoid concurrent writes to the same agent session.
- Reuse existing channel abstractions instead of building a WebSocket-only path.
- Support high fan-out scenarios such as 4+ concurrent child tasks.
- Keep raw task state updates separate from user-facing conversational summaries.

---

## 4. Non-Goals

- Replacing the existing synchronous `agent_send` path.
- Building a general distributed workflow engine beyond OpenFang's current kernel boundary.
- Guaranteeing exactly-once user notification across all third-party channels.
- Making every adapter render rich UI cards; only WebSocket/WebChat are expected to support richer visualization initially.

---

## 5. User Requirements

### Core Requirements

1. A main agent can dispatch work to another agent asynchronously and receive a `task_id` immediately.
2. The main agent is not blocked and can continue responding to the user.
3. Child agents can publish progress updates during execution.
4. The main agent can periodically check task progress and report it to the user.
5. Users can ask for the progress of ongoing tasks, and the main agent can retrieve it.
6. Progress and result delivery must work for all channels, not only WebSocket.
7. If multiple tasks update within a short time window, the main agent should produce one aggregated report instead of many fragmented messages.

### Experience Requirements

- The assistant should feel organized, not noisy.
- Progress reports should sound intentional and summarized.
- The user should never lose visibility into running work.
- Updates should return to the original conversation context whenever possible.

---

## 6. Design Principles

- **State and conversation are different concerns**: raw task state should be stored independently from user-facing messages.
- **Push and pull must both exist**: the system must support proactive updates and direct queries.
- **Session writes must stay serialized**: async updates cannot write into the same agent session concurrently.
- **Channel delivery should be abstracted**: notification routing should target a conversation, not just a WebSocket connection.
- **User-facing reporting should be aggregated**: multiple machine events should become one assistant message when appropriate.

---

## 7. High-Level Architecture

The design introduces five new subsystems:

1. **Async Task Dispatcher**
   - accepts async delegation requests
   - creates task records
   - runs child agent work in background `tokio` tasks

2. **Task State Store**
   - stores lifecycle, progress, metadata, routing info, and timestamps
   - supports reads by task, conversation, agent, and open status

3. **Main Agent Inbox / Turn Queue**
   - serializes user turns and async task callbacks into one ordered execution stream
   - prevents concurrent session mutation

4. **Conversation Progress Aggregator**
   - collects task updates across a short time window
   - emits one digest event for the main agent

5. **Notification Router**
   - routes updates back to the originating conversation across WS and non-WS channels
   - uses thread-aware sending when supported

---

## 8. Core Concepts

### 8.1 Async Task

An async task is a delegated unit of work executed by a child agent outside the main agent's current turn.

Each task has:

- execution state
- progress state
- ownership and lineage
- conversation routing context
- reporting policy

### 8.2 Reply Target

A `ReplyTarget` captures where background updates should return.

Suggested fields:

- `channel`
- `recipient`
- `thread_id`
- `platform_message_id`
- `output_format`
- `conversation_key`
- `agent_id`
- `session_id`

This allows the kernel to route progress updates to:

- WebSocket/WebChat session
- Slack thread
- Telegram chat
- Email reply chain
- other adapters through the common channel interface

### 8.3 Conversation Key

A `conversation_key` identifies the user-visible conversation across transport layers.

It is the join key for:

- active tasks in the same conversation
- aggregated reporting windows
- user-initiated progress lookups
- channel routing for async notifications

---

## 9. Task Lifecycle

Suggested task states:

- `queued`
- `running`
- `waiting_input`
- `completed`
- `failed`
- `cancelled`

Suggested progress fields:

- `progress_percent: Option<u8>`
- `progress_summary: String`
- `progress_detail: Option<String>`
- `last_progress_at: DateTime`
- `last_reported_at: Option<DateTime>`
- `next_report_at: Option<DateTime>`
- `report_seq: u64`

Suggested lineage fields:

- `task_id`
- `parent_task_id: Option<String>`
- `parent_agent_id`
- `worker_agent_id`
- `created_by_agent_id`

Suggested routing fields:

- `conversation_key`
- `reply_target_json`
- `user_visible_label`

---

## 10. Tooling Surface

### New Tools

#### `agent_dispatch_async`

Purpose:

- dispatch work to a child agent without blocking the current turn

Input:

- `agent_id`
- `message`
- `label`
- `report_every_secs` optional
- `report_policy` optional

Output:

- `task_id`
- accepted metadata

#### `task_progress_update`

Purpose:

- allow child agent or dispatcher to update progress

Input:

- `task_id`
- `summary`
- `percent` optional
- `detail` optional
- `state` optional

#### `task_get`

Purpose:

- retrieve one task's current state

Input:

- `task_id`

#### `task_list`

Purpose:

- retrieve tasks scoped to the current conversation, current main agent, or open tasks

Input:

- `scope`: `conversation | agent | open`
- `status` optional

#### `task_cancel`

Purpose:

- cancel a running task if supported

### Existing Tools to Keep

- `agent_send` remains synchronous for immediate RPC-style coordination.
- `task_post/task_claim/task_complete` can remain as lower-level primitives or be phased behind the richer async task API.

---

## 11. Main Agent Behavior Model

### 11.1 Dispatch Flow

1. User asks the main agent to perform work.
2. Main agent decides to delegate portions of that work.
3. Main agent calls `agent_dispatch_async` one or more times.
4. Kernel returns `task_id`s immediately.
5. Main agent replies to the user without waiting for child completion.

Example:

- "I’ve started four background tasks: research, code patching, analysis, and review. I’ll keep you updated as they progress."

### 11.2 Periodic Reporting Flow

The main agent should not block in a loop waiting for progress.

Instead:

1. Each task has `next_report_at`.
2. A background reporter checks due tasks.
3. When a report is due, the kernel emits a digest event into the main agent inbox.
4. The main agent runs one serialized turn, reads current task state through `task_list/task_get`, and produces a user-facing report.

### 11.3 User-Initiated Query Flow

When the user asks:

- "How is the task going?"
- "What’s the progress on the four subtasks?"

the main agent:

1. reads current open tasks for `conversation_key`
2. resolves the relevant task set
3. summarizes their latest state

This path bypasses reporting cooldowns.

---

## 12. Session Consistency and Ordering

This is the most important technical constraint.

Today, a turn loads session state, mutates it, and saves it back. If user messages and background task callbacks write to the same main-agent session concurrently, messages can be reordered or overwritten.

### Proposed Fix

Introduce a **Main Agent Inbox / Turn Queue**:

- user turns enqueue into the inbox
- task digest events enqueue into the same inbox
- only one turn executes at a time per main agent

This guarantees:

- no concurrent session mutation
- deterministic ordering
- safe interleaving of user messages and background reports

This queue should exist per main agent, not globally.

---

## 13. Reporting and Aggregation Design

### 13.1 Why Aggregation Is Required

If 4 child tasks update within 5 seconds, sending 4 separate progress reports will:

- spam the user
- make the main agent sound reactive and mechanical
- produce poor transcript quality

The assistant should instead produce one coherent summary covering all relevant tasks.

### 13.2 Two Update Channels

We intentionally split updates into two channels:

#### A. Raw Status Channel

Used for:

- internal state tracking
- task cards in WebChat/WS
- observability

This can receive every fine-grained task update.

#### B. User Report Channel

Used for:

- main-agent natural language responses
- Slack/Telegram/Email progress messages

This channel should be aggregated and rate-limited.

### 13.3 Aggregation Unit

Aggregation happens per `conversation_key`.

Reason:

- users think in terms of one conversation, not one task
- a single user update should cover all relevant active work

### 13.4 Aggregation Policy

Use a mixed debounce model:

- `debounce_window = 2s`
- `max_wait = 5s`
- `min_report_interval = 20s` per conversation

Behavior:

- if several task updates arrive close together, wait briefly and combine them
- if updates keep streaming, force a flush at `max_wait`
- if a report was just sent, suppress another conversational report until cooldown expires

### 13.5 Immediate Flush Conditions

These should bypass or shorten the normal aggregation window:

- any task reaches `completed`
- any task reaches `failed`
- any task reaches `waiting_input`
- all open tasks in the conversation finish

### 13.6 Aggregation Bucket Shape

Suggested in-memory structure:

- keyed by `conversation_key`
- stores latest update per `task_id`
- stores first event timestamp
- stores flush timer handle
- stores last conversational report timestamp

Only the latest task update per task should be kept inside the active window.

### 13.7 Digest Event Shape

When a window flushes, emit one digest event into the main agent inbox:

```text
[ASYNC TASK DIGEST]
conversation_key=conv_123
window_secs=5
tasks:
- task_a / researcher / running / 40% / gathered 12 of 30 sources
- task_b / coder / running / 65% / patch implemented, tests pending
- task_c / analyst / completed / analysis ready
- task_d / reviewer / running / 20% / review started
[/ASYNC TASK DIGEST]
```

The main agent then converts that digest into one user-facing summary.

---

## 14. Notification Routing Across Channels

### 14.1 Requirement

Async progress and results must be deliverable beyond WebSocket.

### 14.2 Proposed Router

Introduce a `NotificationRouter` that takes a `ReplyTarget` and decides how to deliver:

- `WebSocket/WebChat`: real-time connection event plus optional transcript message
- channels with thread support: `send_in_thread`
- channels without thread support: `send`
- email: use reply context where available

### 14.3 Delivery Rules

- Prefer returning updates to the original thread or conversation.
- Fall back to plain outbound message if threading is unavailable.
- Record best-effort delivery receipts.
- Preserve channel output formatting.

### 14.4 Why This Is Better Than WS-Only

A WS-only implementation would solve only the web dashboard case. The assistant behavior would still break on Slack, Telegram, Email, and other channels.

The correct abstraction is:

- task state is transport-agnostic
- report delivery is routed through the channel layer

---

## 15. WebSocket `task_update` Design

### 15.1 Purpose

`task_update` is a transport event, not necessarily a chat transcript message.

It is intended to power a richer UI in WebChat/WebSocket clients.

### 15.2 Example Payload

```json
{
  "type": "task_update",
  "task_id": "task_a",
  "conversation_key": "conv_123",
  "label": "Research latest APIs",
  "worker_agent_name": "researcher",
  "status": "running",
  "progress_percent": 40,
  "summary": "Gathered 12 of 30 sources",
  "report_seq": 3
}
```

### 15.3 Intended UI Effect

The UI should render a task card or task panel that updates in place:

- `queued -> running -> completed`
- progress text and percentage update live
- failures can surface inline

This avoids polluting the transcript with low-level machine updates.

### 15.4 Relationship to Assistant Messages

In WebChat there are two parallel outputs:

- task cards update live from `task_update`
- assistant transcript messages are generated only when the main agent emits an aggregated or important natural-language report

This keeps the UX clean while preserving visibility.

---

## 16. Behavior on Non-WS Channels

Non-WS channels usually cannot render dynamic cards.

Therefore:

- raw task updates remain internal
- user-visible progress should be sent as aggregated assistant messages

Example on Telegram/Slack:

- user receives one summarized progress update covering all relevant tasks
- not four separate low-level status lines

This keeps the assistant behavior consistent across transports.

---

## 17. Data Model Changes

Suggested task table additions:

- `state`
- `progress_percent`
- `progress_summary`
- `progress_detail`
- `last_progress_at`
- `last_reported_at`
- `next_report_at`
- `conversation_key`
- `reply_target_json`
- `parent_agent_id`
- `worker_agent_id`
- `created_by_agent_id`
- `user_visible_label`
- `report_policy_json`
- `error_message`

Suggested indexes:

- `(conversation_key, state)`
- `(next_report_at, state)`
- `(parent_agent_id, state)`
- `(worker_agent_id, state)`

Suggested aggregator state:

- in memory first
- optionally persisted later for crash recovery

---

## 18. Failure Handling

### Child Task Failure

- mark task as `failed`
- store error summary
- emit immediate or near-immediate digest
- main agent explains impact in user terms

### Main Agent Offline or Busy

- digest stays queued in main-agent inbox
- once the main agent queue drains, the digest is processed in order

### Delivery Failure

- task state remains correct even if notification fails
- delivery failure is logged separately
- user can still query progress later

### Process Restart

- persisted task state survives restart
- running tasks may become `failed` or `interrupted` based on restart policy
- aggregator windows can be rebuilt conservatively from task state

---

## 19. Security and Guardrails

- Only authorized agents should be able to read or mutate tasks they own or created.
- Child agents should update only tasks assigned to them.
- `reply_target` must be sanitized and treated as system-owned routing data.
- Progress payloads should be size-limited to prevent prompt injection or event flooding.
- Reporting cadence should be bounded to prevent spam across external channels.

---

## 20. Rollout Plan

### Phase 1: Minimal Async Delegation

- add `agent_dispatch_async`
- create persistent async task records
- execute child work in background
- expose `task_get` and `task_list`

### Phase 2: Progress Visibility

- add `task_progress_update`
- persist progress fields
- support user-initiated progress queries

### Phase 3: Safe Main-Agent Re-entry

- add main-agent inbox / turn queue
- route async digests through serialized turns

### Phase 4: Aggregated Reporting

- add conversation progress aggregator
- implement debounce/max-wait/cooldown flush logic
- emit digest events

### Phase 5: Cross-Channel Notifications

- add `ReplyTarget`
- add `NotificationRouter`
- add WS `task_update`
- route summaries through all supported channels

---

## 21. Open Questions

- Should the main agent always be the one to phrase updates, or can the kernel send system-formatted updates for low-priority channels?
- Should progress aggregation be configurable per conversation or per agent profile?
- Should task digests be visible in audit/comms screens as first-class events?
- Should the system support explicit user subscriptions such as "only notify me on completion"?
- How should interrupted tasks recover after kernel restart: retry, resume, or fail closed?

---

## 22. Recommended Direction

The recommended implementation is:

- keep raw task execution and progress at the kernel/task-store level
- keep user-facing progress summaries at the main-agent level
- serialize all main-agent re-entry through an inbox
- aggregate updates per conversation before waking the main agent
- treat WS as one delivery surface, not the architecture itself

This preserves correctness, supports all channels, and produces assistant-like behavior instead of a stream of machine events.


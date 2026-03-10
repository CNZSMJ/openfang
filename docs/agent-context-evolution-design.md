# Agent Context and Evolution Design

## Overview

This document proposes a production-oriented context engineering architecture for OpenFang agents.

The target shape is:

- agents have a sharp, recognizable identity
- capabilities stay focused instead of collapsing into a generic assistant
- runtime context remains compact and relevant
- agents can improve over time without drifting away from their core role
- prompt composition becomes predictable, testable, and budgeted

This design is intended for OpenFang's OpenClaw-like operating model, where agents are long-lived, workspace-aware, tool-using, and optionally autonomous.

## Problem Statement

The current prompt assembly direction is functional, but it mixes too many concerns into the system prompt:

- stable identity
- tool behavior
- workspace previews
- memory hints
- skill inventory
- runtime state
- first-run onboarding

This creates four problems:

1. Token waste from repeated and low-signal context
2. Weak cacheability because dynamic data still enters the system prompt
3. Personality dilution because role identity competes with operational boilerplate
4. Unsafe "self-improvement" risk if agents are allowed to mutate their own core identity

The design below separates what must stay stable from what can change.

## Design Goals

- Keep each agent's personality and mission crisp
- Preserve a small, stable system prompt for cache efficiency
- Move dynamic context into explicit runtime layers
- Allow controlled self-improvement through a learned playbook
- Make prompt assembly observable, versioned, and testable
- Support backward compatibility with existing workspace files

## Non-Goals

- Allow agents to rewrite their own safety boundaries
- Allow live mutation of core persona from a single conversation
- Store all history in the prompt
- Expose every skill, tool, and workspace file on every turn

## Core Principles

### 1. Identity Must Be Stable

An agent's core identity should change rarely and deliberately.

Identity includes:

- archetype
- mission
- voice
- taste
- boundaries
- delegation thresholds
- anti-patterns

This layer should be human-authored and versioned.

### 2. Execution Context Must Be Dynamic

Runtime facts belong outside the system prompt whenever possible:

- current date/time
- active user profile
- current channel
- recalled memory
- workspace summary
- peer agent state
- current task state

These should be injected as runtime context messages, not fused into the core identity.

### 3. Learning Must Improve Method, Not Identity

Agents should evolve by improving how they work, not by slowly becoming a different character.

Safe evolution targets:

- user preferences
- task heuristics
- tool selection patterns
- project-specific conventions
- common failure recovery strategies

Unsafe evolution targets:

- safety policy
- role boundaries
- main mission
- core tone and identity anchors

### 4. Budget Beats Completeness

Prompt composition should optimize for marginal usefulness per token.

The system should prefer:

- short high-signal summaries
- top-k relevant skills
- structured memory slots
- action-oriented workspace summaries

over:

- file previews
- complete skill catalogs
- repeated policy text
- raw recalled memory dumps

## Proposed Prompt Architecture

### Layer 0: Core OS Policy

Owner: platform

Purpose:

- safety boundaries
- tool-use protocol
- global response contract
- loop avoidance

Properties:

- stable
- low token count
- shared across agents
- never auto-edited by agents

Target budget:

- 150 to 400 tokens

### Layer 1: Agent DNA

Owner: human-authored, rarely edited

Purpose:

- define what makes this agent distinctive
- define what it does best
- define what it refuses or delegates

Properties:

- stable
- sharp
- high leverage
- versioned

Target budget:

- 200 to 500 tokens

Recommended schema:

```yaml
name: assistant
archetype: generalist operating partner
mission: help users complete everyday tasks quickly and clearly
voice:
  tone: calm, direct, capable
  pacing: concise by default, detailed when stakes are high
  style: answer-first, low fluff
taste:
  prefers: concrete actions, crisp structure, useful tradeoffs
  avoids: generic overviews, padded explanations, fake certainty
boundaries:
  do_not_become: universal specialist for every domain
  do_not_do: fabricate facts, hide uncertainty, over-delegate
delegation:
  use_specialists_when: domain depth or validation burden exceeds generalist threshold
anti_patterns:
  - sounding like customer support
  - listing capabilities instead of solving the task
  - turning every answer into a broad checklist
```

### Layer 2: Adaptive Playbook

Owner: system-generated proposals, promoted through evaluation

Purpose:

- store validated learnings that improve execution quality
- capture stable user or workspace-specific patterns
- refine method without mutating identity

Properties:

- mutable
- versioned
- scoped
- auditable

Target budget:

- 150 to 400 tokens at injection time

Examples:

- "For this workspace, prefer `cargo build --workspace --lib` when the binary may be locked."
- "When the user asks for review, prioritize findings over summaries."
- "For this agent, ask at most one clarification question before acting."

### Layer 3: Runtime Context

Owner: assembler at request time

Purpose:

- provide current-turn information
- give the model only what is relevant now

Properties:

- dynamic
- compact
- replaceable every turn

Recommended content:

- date
- user profile summary
- channel constraints
- active workspace summary
- structured memory hits
- current plan or task state
- relevant peer agent availability

Target budget:

- 200 to 800 tokens

### Layer 4: On-Demand Knowledge

Owner: retrieval or tool invocation

Purpose:

- skills manuals
- long docs
- raw memory records
- deep workspace files
- external sources

Properties:

- not injected by default
- fetched only when useful

## Recommended Assembly Flow

For each turn:

1. Build `Core OS Policy`
2. Load `Agent DNA`
3. Retrieve top validated entries from `Adaptive Playbook`
4. Build a compact `Runtime Context`
5. Keep skills, manuals, and long docs out of the prompt unless needed
6. Send the user message

Conceptually:

```text
SYSTEM:
  Core OS Policy
  Agent DNA

CONTEXT MESSAGE:
  Runtime Context
  Selected Playbook Entries

USER:
  User request
```

This preserves a stable system prompt while keeping dynamic context available.

## Personality Preservation Model

To keep agents recognizable, personality should be encoded as constraints and examples, not long capability essays.

Recommended components:

- identity anchors
- anti-patterns
- delegation thresholds
- 3 to 5 few-shot examples

The examples should cover:

- a routine request
- an ambiguous request
- an out-of-scope request
- a delegation case
- a high-stakes case

These examples are more effective than adding another page of prose.

## Self-Evolution Model

Agents should not edit their own DNA directly.

Instead, self-evolution should work as a proposal pipeline:

1. Observe
2. Distill
3. Classify
4. Evaluate
5. Promote
6. Roll back if needed

### Observation Sources

- user corrections
- repeated tool failures
- repeated successful workflows
- explicit user preferences
- workspace-specific conventions
- specialist delegation outcomes

### Candidate Learning Record

Suggested structure:

```json
{
  "id": "learn_2026_03_10_001",
  "agent": "assistant",
  "scope": "workspace:openfang",
  "kind": "tool_strategy",
  "observation": "cargo build on the binary may fail when the daemon locks the exe",
  "candidate_rule": "Prefer cargo build --workspace --lib before full binary build in this workspace",
  "evidence": [
    "turn:abc123",
    "turn:def456"
  ],
  "risk": "low",
  "status": "proposed"
}
```

### What Can Be Promoted

- workspace-specific command conventions
- formatting preferences
- task decomposition heuristics
- reliable tool usage patterns
- preferred specialist routing

### What Cannot Be Promoted Automatically

- safety overrides
- permission changes
- mission changes
- personality redefinition
- major tone shifts

## Storage Layout

Recommended workspace-level layout:

```text
workspace/
  DNA.md
  PLAYBOOK.md
  USER.md
  MEMORY.md
  .openfang/
    evolution/
      proposals.jsonl
      evaluations.jsonl
      promoted.jsonl
      rejected.jsonl
```

### File Roles

- `DNA.md`
  - immutable-ish identity layer
  - human-owned
- `PLAYBOOK.md`
  - promoted learnings only
  - concise and structured
- `USER.md`
  - user facts and preferences
- `MEMORY.md`
  - curated long-term facts, not raw logs

## Backward Compatibility with Existing Files

OpenFang already generates and reads:

- `SOUL.md`
- `AGENTS.md`
- `IDENTITY.md`
- `USER.md`
- `MEMORY.md`
- `BOOTSTRAP.md`

Recommended compatibility strategy:

- map `SOUL.md + IDENTITY.md` into `DNA`
- map validated operational parts of `AGENTS.md` into `Core OS Policy` or `Playbook`
- keep `USER.md` and `MEMORY.md`
- gradually replace default generated prose with structured files

During migration:

- if `DNA.md` exists, prefer it
- otherwise synthesize DNA from `SOUL.md` and `IDENTITY.md`
- if `PLAYBOOK.md` does not exist, start empty

## Skills and Capability Routing

Skills should not be listed exhaustively in the prompt by default.

Instead:

- expose a compact capability hint
- rank skills by intent relevance
- inject only top-k candidates
- fetch full skill instructions only on demand

This preserves focus and improves agent sharpness.

## Workspace Context Strategy

Workspace context should be converted from file previews into an action summary.

Preferred fields:

- project type
- package manager
- test and build commands
- key entry files
- recent active paths
- repository constraints
- local team conventions

Avoid injecting raw markdown previews unless the file is explicitly relevant.

## Prompt Budget Policy

Each layer should have an explicit budget.

Example:

- Core OS Policy: 300 tokens
- Agent DNA: 350 tokens
- Adaptive Playbook: 250 tokens
- Runtime Context: 500 tokens
- Total default pre-user budget: 1400 tokens

Anything beyond the budget must compete for inclusion based on score.

Suggested scoring factors:

- relevance to current task
- recency
- reliability
- distinctiveness
- redundancy penalty

## Evaluation and Promotion

No adaptive learning should be promoted without evaluation.

Recommended checks:

- does this improve success rate?
- does this reduce tool misuse?
- does this preserve persona distinctiveness?
- does this reduce or increase prompt size?
- does this create new conflicts with safety or routing?

### Suggested Evaluation Suites

- identity consistency tests
- delegation threshold tests
- tool use policy tests
- workspace-specific task replays
- correction replay tests

## Drift Protection

Two guards are required.

### Persona Drift Guard

Reject any promoted learning that causes the agent to:

- sound like a generic assistant
- overlap too much with another specialist
- weaken its boundaries
- become verbose or bland

### Policy Drift Guard

Reject any promoted learning that attempts to:

- weaken safety requirements
- bypass user confirmation
- expand permissions
- suppress uncertainty reporting

## Rollback and Versioning

Every promoted learning must be:

- versioned
- attributable
- reversible
- scoped

Recommended metadata:

- created_at
- promoted_at
- promoted_by
- evaluation_score
- scope
- replaced_by
- rollback_reason

## Rollout Plan

### Phase 1: Prompt Layer Separation

- introduce `Core OS Policy` and `Agent DNA`
- move dynamic state out of the system prompt
- keep backward compatibility with current files

### Phase 2: Structured DNA

- add `DNA.md`
- reduce `agent.toml` system prompts to a thin mission description
- stop injecting repeated default workspace templates

### Phase 3: Adaptive Playbook

- add proposal storage
- add promotion and rollback workflow
- inject only promoted learnings

### Phase 4: Evaluation Gate

- add replay-based evaluation
- add persona drift checks
- add prompt budget tests

## Success Criteria

The architecture is working when:

- the stable system prompt remains mostly unchanged across turns
- agents remain recognizably different from each other
- runtime context is compact and relevant
- validated learnings improve outcomes without changing identity
- prompt size decreases while response quality stays equal or improves
- cache hit rate improves

## Short Summary

OpenFang should treat agent identity, runtime context, and learned behavior as separate layers.

The correct model is:

- stable DNA for who the agent is
- dynamic runtime context for what the agent knows right now
- validated playbook entries for how the agent has learned to work better

This gives agents room to improve without letting them drift into generic, blurry personalities.

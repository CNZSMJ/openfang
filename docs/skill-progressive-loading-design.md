# Skill Progressive Loading Design

## Overview

This document proposes a progressive loading architecture for OpenFang skills.

The target shape is:

- the system prompt no longer contains a full catalog of bundled and installed skills
- agents discover relevant skills on demand instead of reading every skill on every turn
- detailed skill instructions are loaded only for the selected skill
- prompt size drops, cacheability improves, and skill choice becomes more reliable

This design is intended for OpenFang's current runtime, which already has:

- a skill registry
- `skill_get_instructions(skill_name)`
- per-agent skill visibility

The missing piece is a lightweight retrieval step between "skills exist" and "load one skill manual".

## Problem Statement

The current system prompt injects a full `## Skills` section every turn.

This creates four problems:

1. Token waste from listing dozens of unrelated skills
2. Lower prompt cache efficiency because a large dynamic catalog is always present
3. Worse model focus because irrelevant skills compete with the current task
4. Weak reliability if we later remove the full catalog without giving the model a way to discover relevant skills

The core issue is that the system currently supports:

- full skill inventory injection
- direct single-skill expansion via `skill_get_instructions`

but it does not support:

- skill discovery from natural language intent

## Goals

- Remove the full skill catalog from the default system prompt
- Let the model discover relevant skills from natural language task intent
- Load detailed instructions only for the chosen skill
- Keep the implementation simple and explainable in v1
- Reuse the existing skill registry and `skill_get_instructions` flow

## Non-Goals

- Build a general-purpose semantic retrieval platform in v1
- Add vector search infrastructure for skills in v1
- Change the skill installation model
- Redesign skill manifests
- Automatically force skill usage on every task

## Desired Runtime Flow

The desired model workflow is:

1. Decide whether the task may benefit from a specialized skill
2. Call `skill_search("natural language intent", top_k=3)`
3. Inspect the top results
4. If one result is clearly strongest, choose it directly
5. Otherwise compare the short list and choose the best fit, or skip skills if nothing is relevant
6. Call `skill_get_instructions(skill_name)`
7. Execute the task

This keeps skill discovery explicit and cheap while preserving access to detailed manuals only when needed.

## Prompt Strategy

### Current Behavior

The current prompt builder injects a full `## Skills` section containing every visible skill summary.

### Proposed Behavior

Replace the full skill catalog with a short protocol section:

```md
## Skills
- Skills are available on demand.
- Do not assume a skill is relevant just because it exists.
- When a request may benefit from specialized guidance, search for matching skills first.
- If a skill looks relevant, load detailed instructions only for that skill.
```

This keeps the stable prompt small while still telling the model how to discover and load skills.

## API and Tool Design

## New Tool: `skill_search`

Add a new built-in tool:

```json
{
  "query": "natural language task intent",
  "top_k": 3
}
```

Response shape:

```json
{
  "results": [
    {
      "name": "kubernetes",
      "description": "Kubernetes operations expert for kubectl, pods, deployments, and debugging",
      "tags": ["infra", "k8s"],
      "tools_count": 0,
      "has_prompt_context": true,
      "score": 0.91,
      "match_reason": [
        "alias:k8s->kubernetes",
        "description:deployment",
        "description:debug"
      ]
    }
  ]
}
```

### Why a New Tool Is Needed

OpenFang already has:

- a skill registry that can list installed skills
- `skill_get_instructions(skill_name)` for loading one skill manual

But it does not currently have a native skill discovery tool that accepts natural language queries.

`skill_search` fills exactly that gap.

## Retrieval Strategy

### Principle

Do not ask the model to generate exact keywords.

Instead:

- let the model provide natural language intent
- let the backend perform retrieval and ranking

This is more reliable than forcing the model to guess the exact vocabulary used by skill names or descriptions.

### V1 Retrieval Method

Version 1 should use a lightweight hybrid lexical scorer, not embeddings.

Candidate signals:

- exact name match
- name prefix match
- alias match
- description token hits
- tag hits
- provided tool hits
- small bonus for `has_prompt_context`

### Query Normalization

`skill_search` should normalize input before scoring:

- lowercase
- tokenize
- drop common stopwords
- expand aliases and domain synonyms

Examples:

- `k8s -> kubernetes`
- `rag -> retrieval, vector, embeddings`
- `email -> email-writer`
- `review -> code-reviewer`
- `ts -> typescript`

This keeps the interface natural-language-first while avoiding a hard dependency on model-generated keyword quality.

## Ranking

Version 1 ranking should stay simple and explainable.

An example scoring formula:

```text
score =
  10 * exact_name_match +
   6 * prefix_name_match +
   4 * alias_match +
   3 * description_hits +
   2 * tag_hits +
   1 * provided_tool_hits +
 0.5 * has_prompt_context
```

The exact constants can change. The important property is interpretability.

The response should include a compact `match_reason` list so the model can understand why a result was surfaced.

## Selection Rules for the Model

The runtime protocol should encourage these decisions:

- if the top result is clearly stronger than the rest, choose it directly
- if the top results are close, compare the top 2-3 briefly
- if all results are weak, skip skill loading and proceed normally
- do not load multiple skill manuals unless the task genuinely requires it

This prevents pointless prompt expansion after retrieval.

## Implementation Plan

### Phase 1: Minimal Working Flow

1. Replace the full skill catalog in the system prompt with a short skills protocol
2. Add `skill_search` as a built-in tool
3. Implement lexical search in the skill registry
4. Keep `skill_get_instructions` unchanged

### Phase 2: Better Matching

1. Add alias and synonym tables
2. Improve field weighting
3. Add telemetry for search queries and selected skills

### Phase 3: Optional Semantic Rerank

If the skill count grows enough to justify it:

1. Use lexical retrieval to get top 10-15 candidates
2. Add embedding-based reranking over that shortlist
3. Keep lexical retrieval as the stable fallback

This avoids committing to a vector-only design too early.

## Code Touch Points

### Prompt Layer

- [prompt_builder.rs](/Users/huangjiahao/workspace/openfang-0.1.0/feature-cyber-soul/crates/openfang-runtime/src/prompt_builder.rs)

Changes:

- remove full skill enumeration from the default system prompt
- replace it with a short skills usage protocol

### Tool Layer

- [tool_runner.rs](/Users/huangjiahao/workspace/openfang-0.1.0/feature-cyber-soul/crates/openfang-runtime/src/tool_runner.rs)

Changes:

- register `skill_search`
- validate input
- call registry search
- return compact structured results

### Registry Layer

- [registry.rs](/Users/huangjiahao/workspace/openfang-0.1.0/feature-cyber-soul/crates/openfang-skills/src/registry.rs)

Changes:

- add `search(query, top_k)` on `SkillRegistry`
- define result structs and scoring helpers

### Optional API Layer

- [routes.rs](/Users/huangjiahao/workspace/openfang-0.1.0/feature-cyber-soul/crates/openfang-api/src/routes.rs)

Optional future change:

- add `POST /api/skills/search` for dashboard and TUI reuse

## Why Not Use Vector Search in V1

OpenFang already has embedding-backed retrieval for memory, but not for skills.

Using vector search for skills in v1 would add:

- embedding generation lifecycle for bundled and installed skills
- index refresh logic
- provider dependency and fallback complexity
- less interpretable ranking

The skill corpus is still small enough that lexical retrieval with alias expansion should be sufficient for a first production version.

The correct upgrade path is:

- lexical retrieval first
- semantic rerank later if needed

not:

- vector search first

## Risks

### Risk: Weak Alias Coverage

The first version may miss good skills if the alias table is too small.

Mitigation:

- start with a focused alias table for the highest-traffic domains
- log weak-result queries for iteration

### Risk: Over-Retrieval

Broad lexical matching may return too many mediocre results.

Mitigation:

- keep `top_k` small
- rank by multiple fields
- surface `match_reason`
- let the model skip skills when results are weak

### Risk: Model Does Not Call `skill_search`

Even with the tool available, the model may continue to ignore it.

Mitigation:

- keep a short but explicit skills protocol in the system prompt
- use a highly literal tool name: `skill_search`
- add tests that assert the tool is available and described

## Acceptance Criteria

- The system prompt no longer lists all skills by default
- The model can discover relevant skills through `skill_search`
- The model loads detailed instructions only for selected skills
- Prompt token count drops materially on normal turns
- Skill selection quality is better than direct name guessing
- The design remains fully functional without embeddings

## Summary

The recommended design is:

- keep a short skills protocol in the system prompt
- add `skill_search` for natural language discovery
- keep `skill_get_instructions` for single-skill expansion
- use lexical retrieval first
- add semantic reranking only if scale later justifies it

This is the smallest change that materially improves prompt quality without introducing unnecessary retrieval infrastructure.

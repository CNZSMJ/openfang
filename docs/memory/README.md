# Memory Docs Contract

`docs/memory/` is the only home for ongoing memory-program management docs.

## Stable entrypoint

The user can resume work with a single instruction:

- `阅读 docs/memory/execution_state.md，继续工作`

When that instruction is given, the assistant must:

1. Read `docs/memory/execution_state.md` first.
2. Read every design document listed in its `## Design docs` section.
3. Check the current branch and worktree state.
4. Continue only within the scope of the `## Current phase`.
5. Update `docs/memory/execution_state.md` before ending substantial work.

## Structure freeze

To keep cross-computer continuity stable, the assistant must not rename, move, delete, or change the section structure of the following files without explicit user approval:

- `docs/memory/README.md`
- `docs/memory/execution_state.md`
- `docs/memory/agent_memory_enhancement_plan.md`

Allowed changes without approval:

- Update progress fields inside `docs/memory/execution_state.md`
- Append new design docs under `docs/memory/` only if explicitly needed for the active phase
- Update references in `docs/memory/execution_state.md`

Disallowed changes without approval:

- Replacing the entrypoint file with a different file
- Renaming headings in `docs/memory/execution_state.md`
- Moving memory management docs back to `docs/`
- Creating a second competing execution-state file

## File roles

- `docs/memory/execution_state.md`: operational state, branch/worktree continuity, next actions
- `docs/memory/agent_memory_enhancement_plan.md`: program design and staged architecture


---
name: find-skills
description: Helps users discover and install agent skills when they ask questions like "how do I do X", "find a skill for X", "is there a skill that can...", or express interest in extending capabilities. This skill should be used when the user is looking for functionality that might exist as an installable skill.
---

# Find Skills

This skill helps you discover and install skills from the open agent skills ecosystem.

## When to Use This Skill

Use this skill when the user:

- Asks "how do I do X" where X might be a common task with an existing skill
- Says "find a skill for X" or "is there a skill for X"
- Asks "can you do X" where X is a specialized capability
- Expresses interest in extending agent capabilities
- Wants to search for tools, templates, or workflows
- Mentions they wish they had help with a specific domain (design, testing, deployment, etc.)

## What is Skill Management?

OpenFang Skills are modular packages that extend agent capabilities. They are natively managed and stored either globally (`~/.openfang/skills/`) or within an agent's workspace.

**Native tools for skill management:**

- `skill_install(source, scope)` - Install from a slug ('agent-reach'), GitHub repo ('owner/repo'), URL, or local path.
- `skill_create(name, description, prompt, scope)` - Convert any documentation or instructions into a reusable skill.
- `skill_get_instructions(skill_name)` - Get detailed rules for an installed skill.

## How to Help Users Install Skills from Any Source

### 1. From a GitHub Repository or URL
If a user provides a link to a repo or a specific tool description:
```json
{
  "source": "owner/repo",
  "scope": "global"
}
```

### 2. From Local Development
If you've developed a tool locally and want to register it:
```json
{
  "source": "/path/to/my-skill",
  "scope": "global"
}
```

### 3. Converting Instructions to a Skill ("Instruction-to-Skill")
This is a powerful pattern for when a user shares a tweet, a blog post, or a set of rules:
1. Use `web_fetch` to read the instructions.
2. Summarize the core rules and tool definitions.
3. Call `skill_create` to save it forever.

Example:
```json
{
  "name": "twitter-marketing-expert",
  "description": "Rules for high-engagement tweets extracted from X thread",
  "prompt": "Captured rules: 1. Use questions... 2. Max 3 hashtags...",
  "scope": "workspace" 
}
```
*Tip: Use `scope='workspace'` for skills that are specific to a current project to keep the global environment clean.*

## Common Skill Sources

| Source          | Example `source` string                  |
| --------------- | ---------------------------------------- |
| Official Market | `agent-reach`                            |
| Community Repo  | `vercel-labs/agent-skills@skill-name`    |
| External Tool   | `https://github.com/user/my-custom-tool` |
| Local Path      | `/Users/me/dev/new-skill`                |

## Scope: Global vs. Workspace

- **`global` (Default)**: Use this for general tools (e.g., "React Doc Expert"). The skill becomes available to all agents (Coder, Assistant, etc.).
- **`workspace`**: Use this for project-specific rules (e.g., "Project-X Coding Style"). The skill stays confined to your current workspace.

## How to Help Users Find and Install Skills

### Step 1: Understand What They Need

When a user asks for help with something, identify:

1. The domain (e.g., React, testing, design, deployment)
2. The specific task (e.g., writing tests, creating animations, reviewing PRs)
3. Whether this is a common enough task that a skill likely exists

### Step 2: Search for Skills

Use `web_search` to find relevant skills from the Open Agent Skills ecosystem (skills.sh).

Search queries like:
- `site:skills.sh react performance`
- `site:skills.sh pr review`
- `site:skills.sh changelog`

### Step 3: Present Options to the User

When you find relevant skills, present them to the user with:

1. The skill name and what it does
2. A recommendation to install it using your native tools
3. A link to learn more at skills.sh

Example response:

```
I found a skill that might help! The "vercel-react-best-practices" skill provides
React and Next.js performance optimization guidelines from Vercel Engineering.

Would you like me to install it for you natively?
```

### Step 4: Native Installation

If the user wants to proceed, use the `skill_install` tool:

```json
{
  "source": "vercel-labs/agent-skills@vercel-react-best-practices"
}
```

This ensures the skill is correctly placed in `~/.openfang/skills/` and immediately available to the entire agent fleet.

## Common Skill Categories

When searching, consider these common categories:

| Category        | Example Queries                          |
| --------------- | ---------------------------------------- |
| Web Development | react, nextjs, typescript, css, tailwind |
| Testing         | testing, jest, playwright, e2e           |
| DevOps          | deploy, docker, kubernetes, ci-cd        |
| Documentation   | docs, readme, changelog, api-docs        |
| Code Quality    | review, lint, refactor, best-practices   |
| Design          | ui, ux, design-system, accessibility     |
| Productivity    | workflow, automation, git                |

## Tips for Effective Searches

1. **Use specific keywords**: "react testing" is better than just "testing"
2. **Try alternative terms**: If "deploy" doesn't work, try "deployment" or "ci-cd"
3. **Check popular sources**: Many skills come from `vercel-labs/agent-skills` or `ComposioHQ/awesome-claude-skills`

## When No Skills Are Found

If no relevant skills exist:

1. Acknowledge that no existing skill was found
2. Offer to help with the task directly using your general capabilities
3. Suggest that you can create a custom skill for this task using `skill_create`

Example:

```
I searched for skills related to "xyz" but didn't find any exact matches.
However, I can create a custom prompt-only skill for you to handle this!
Should I set up a "my-xyz-skill" for you?
```

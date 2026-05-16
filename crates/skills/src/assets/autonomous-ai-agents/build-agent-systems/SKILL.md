---
name: build-agent-systems
description: Design, specify, or review agent systems and reusable agent Skills, especially multi-user, multi-channel, distributed, or Kubernetes/pod-based agent products. Use when asked to extract agentic patterns, define agent roles, author portable Skills, design tool registries, session/memory models, channel adapters, orchestration flows, or safety controls for building agents without copying an existing codebase.
origin:
  source: moltis
  url: https://github.com/moltis-org/moltis
---

# Build Agent Systems

Use this skill to turn product requirements into an implementable agent-system blueprint and reusable Skills.

## Workflow

1. Define the operating envelope:
   - users and tenant boundaries
   - channels and inbound modes
   - tasks the agent may perform
   - tool side effects and approval requirements
   - deployment constraints such as pods, queues, stores, secrets, and sandboxing

2. Choose the agent primitives:
   - agent presets for identity, model, tool policy, memory scope, iteration limits, and delegation rules
   - tools for deterministic actions with typed schemas
   - Skills for procedural knowledge and domain workflows
   - sessions for durable conversation state, channel bindings, and cross-session coordination
   - memory for facts that must survive compaction or restart

3. Design the runtime loop:
   - normalize inbound input into a session message
   - build a prompt from identity, user context, project context, skills, memory, runtime state, and tool schemas
   - call the model in streaming mode when the channel can surface partial output
   - validate, sanitize, and execute tool calls
   - append tool results, compact oversized results, and repeat until final answer or limit
   - route the final answer and errors back to the originating channel

4. Design the Skill:
   - keep `SKILL.md` concise and trigger-focused
   - move detailed patterns, templates, and checklists into `references/`
   - include scripts only for fragile or repeated deterministic work
   - validate that the Skill gives an agent enough context to produce concrete artifacts

5. Map to distributed deployment:
   - keep agent runner pods stateless except for in-flight stream state
   - store sessions, channel account config, memory, and job metadata in shared durable stores
   - use queues or leases for inbound events so only one worker handles a message
   - use idempotency keys for channel webhooks and retries
   - isolate tool execution in sandbox workers or separate execution pods

6. Verify the design:
   - require explicit access policy for every channel and tool class
   - define what the user sees on failures; avoid silent drops
   - test prompt assembly, tool validation, loop limits, channel routing, and crash recovery
   - document remaining risks and operational runbooks

## References

- Read `references/moltis-agent-patterns.md` when extracting architecture patterns from Moltis-like systems.
- Read `references/agent-system-blueprint.md` when producing a user-facing blueprint or implementation plan.
- Read `references/skill-authoring-template.md` when writing the actual Skill artifact.

## Output Shape

Prefer concrete artifacts over prose:

- agent roles and preset table
- tool registry and policy matrix
- channel adapter matrix
- session and memory model
- runtime loop sequence
- Kubernetes/pod deployment map
- Skill draft with trigger description and references
- verification checklist and security review

Do not copy project-specific source code into the new system. Extract contracts, boundaries, invariants, and operational patterns.

# System Prompt Profiles + Section Toggles Plan

Date: 2026-02-17
Owner: gateway + agents + config + web-ui
Status: planned

## Problem
Users want control over which prompt sections are included (runtime, memory, user details, time/day tail, etc.), and different models behave better with different prompt shapes/order.

Current prompt assembly is mostly fixed in code, with only limited dynamic behavior.

## Goals
- Let users configure system prompt composition without editing code.
- Support per-model/per-provider prompt behavior.
- Keep prompt caching efficient by separating stable vs dynamic sections.
- Preserve mandatory safety/tooling rules even when users customize.
- Provide clear UI preview of the final rendered prompt and token impact.

## Non-goals (phase 1)
- Full freeform prompt template language.
- Arbitrary user-defined section scripts.
- Per-message adaptive section switching.

## High-level Approach
Introduce profile-driven prompt composition:
- A `PromptProfile` defines enabled sections, order, and section-specific options.
- One profile is default, optional model/provider selectors override it.
- Prompt builder renders:
1. Stable prefix sections (cache-friendly).
2. Dynamic tail sections (time/date and other per-request volatile values).

## Core Design

### 1) Prompt section model
Define typed section identifiers (no stringly-typed logic in core code):
- `identity`
- `user_details`
- `project_context`
- `workspace_files`
- `memory_bootstrap`
- `available_tools`
- `tool_call_guidance`
- `runtime`
- `guidelines`
- `voice_reply_mode`
- `runtime_datetime_tail`

Split sections into policy classes:
- `required`: cannot be disabled (safety/tool protocol baseline).
- `optional`: user-toggleable.
- `dynamic_tail`: should be rendered last for cache locality.

### 2) Profile schema
Add config schema in `moltis-config` for prompt profiles:
- `prompt_profiles.default`: name of default profile.
- `prompt_profiles.profiles[]`:
  - `name`
  - `description`
  - `enabled_sections[]`
  - `section_order[]` (for stable prefix)
  - `dynamic_tail_sections[]` (ordered; usually includes datetime tail)
  - `section_options` (per-section flags)
- `prompt_profiles.overrides[]`:
  - `match`:
    - provider glob (ex: `openai`, `minimax`)
    - model glob (ex: `minimax/*`, `gpt-5*`)
  - `profile`: profile name

### 3) Section options (initial set)
- `runtime`:
  - include host fields
  - include sandbox fields
  - include network/sudo fields
- `user_details`:
  - include name only vs full user profile
- `memory_bootstrap`:
  - include MEMORY.md snapshot
  - force `memory_search` guidance
- `runtime_datetime_tail`:
  - mode: `datetime` | `date_only` | `disabled`
  - placement: `tail` only (phase 1, fixed)

### 4) Builder changes
Refactor `crates/agents/src/prompt.rs` to:
- Build a typed intermediate list of sections from resolved profile.
- Render stable sections in configured order.
- Render dynamic sections at end.
- Enforce required sections regardless of user toggles.
- Keep existing defaults backward-compatible if no profile configured.

### 5) Profile resolution
In `crates/gateway/src/chat.rs`, resolve active profile per request:
1. Start with default profile.
2. Apply first matching override for provider/model.
3. Apply session-level temporary override (optional, phase 2).

Resolved profile is passed into prompt builder for:
- streaming path
- tool path
- sync path
- raw prompt preview
- full context preview

### 6) UI
Add `Settings > System Prompt` page:
- Profile list (create/edit/clone/delete).
- Section toggles and ordering controls.
- Model override rules editor.
- Live preview:
  - rendered prompt
  - character count
  - estimated token count
  - marker for dynamic tail lines that change every request

Phase 1 UI constraints:
- Power-user controls, no drag-drop required (up/down reorder buttons are fine).
- Guardrails visible for required sections (disabled toggle with explanation).

## API and Storage

### Backend APIs
Add namespaced endpoints per feature convention:
- `GET /api/system-prompt/profiles`
- `POST /api/system-prompt/profiles`
- `PUT /api/system-prompt/profiles/:name`
- `DELETE /api/system-prompt/profiles/:name`
- `POST /api/system-prompt/preview`
- `POST /api/system-prompt/resolve-profile`

RPC namespace (if used in current settings stack):
- `system_prompt.*`

### Persistence
Store in existing config persistence layer (not ad-hoc files):
- Add to config schema and loader.
- Keep defaults in template comments.

## Migration Plan
1. Add schema with implicit default profile matching current behavior.
2. Keep old code path as fallback while new path is integrated.
3. Switch builder callers to profile-aware path.
4. Remove fallback path once validated.

## Implementation Phases

### Phase 1: Backend profile engine
- Add typed profile + section config structures.
- Add resolver (default + model/provider override).
- Add profile-driven prompt builder.
- Ensure datetime tail remains final dynamic section.
- Add unit tests for section ordering and enforced required sections.

### Phase 2: Gateway integration
- Resolve profile for all chat prompt generation paths.
- Thread profile into raw prompt/full context debug endpoints.
- Add integration tests for model-specific profile selection.

### Phase 3: Settings UI
- Build profile editor + section toggles.
- Build override rule editor.
- Build live prompt preview endpoint usage.
- Add Playwright tests for create/edit/select/preview flows.

### Phase 4: Presets and docs
- Ship starter presets:
  - `balanced-default`
  - `strict-cache-tail`
  - `minimal-context`
  - `tool-heavy`
- Document model-specific recommendations and examples.

## Testing Strategy

### Rust unit tests
- Profile parsing/validation.
- Required section enforcement.
- Stable + dynamic ordering.
- Model/provider override matching precedence.
- Datetime tail placement invariants.

### Integration tests
- Chat paths (`run_with_tools`, `run_streaming`, `send_sync`) use same resolved profile.
- Raw prompt preview equals runtime assembly for same inputs.

### Web UI E2E
- Toggle sections and verify preview.
- Add model override and confirm resolution.
- Ensure required sections cannot be disabled.

## Security + Safety Constraints
- Required safety/tool protocol sections cannot be removed.
- Validate profile inputs strictly (unknown sections rejected).
- Server-side enforce max section text length in custom freeform fields (if added later).
- Keep secrets redaction behavior unchanged.

## Observability
- Add metrics/tracing:
  - profile resolution hit counts by profile name
  - override match counts by provider/model
  - prompt length by profile
  - preview endpoint usage

## Open Decisions
- Whether overrides use first-match-wins or priority numbers.
- Whether user/session ad-hoc override should be allowed in phase 1.
- Whether section reordering should be fully free or constrained to guardrails.

## Rollout
- Feature flag: `system_prompt_profiles`.
- Internal default-on first, then general release once prompt regressions are clean.
- Add changelog entry and docs for migration/default behavior.

# Skill Authoring Template

Use this when writing a Skill for an agent system.

## Folder Shape

```text
skill-name/
  SKILL.md
  references/
    domain-patterns.md
    blueprint.md
```

Add `scripts/` only when deterministic helper code is repeatedly needed. Add `assets/` only for files used in outputs.

## SKILL.md Skeleton

```markdown
---
name: skill-name
description: What the skill does and exact situations that should trigger it. Mention domain, task types, and important contexts.
---

# Skill Title

Use this skill to ...

## Workflow

1. Clarify the operating envelope.
2. Select the right reference file.
3. Produce concrete artifacts.
4. Validate against the checklist.

## References

- Read `references/domain-patterns.md` when ...
- Read `references/blueprint.md` when ...

## Output Shape

- artifact 1
- artifact 2
- risks and validation
```

## Trigger Description Rules

The frontmatter `description` is the only part visible before the skill loads. Include:

- what the skill helps build or analyze
- trigger phrases or user intents
- domain contexts
- important constraints such as Kubernetes, multi-channel, security, or skill authoring

Do not put "when to use" only in the body.

## Body Rules

- keep the body procedural
- use imperative steps
- avoid generic agent advice
- link each reference file by name and say when to read it
- prefer output shapes and checklists over essays
- avoid copying implementation code from source projects

## Reference Rules

- one level deep from `SKILL.md`
- split by decision context, not by arbitrary topic
- include contracts, invariants, and examples
- omit source-code details unless they define a portable boundary

## Quality Checklist

- frontmatter has `name` and `description`
- skill name is lowercase, hyphenated, and under 64 characters
- description is specific enough to trigger correctly
- `SKILL.md` can be read in under a few minutes
- all referenced files exist
- output shape is explicit
- security and validation are included for side-effectful systems

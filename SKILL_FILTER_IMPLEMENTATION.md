# Skill Whitelist/Blacklist Implementation

**Status:** ✅ Complete (core packages)  
**Date:** 2026-04-24

## Overview

Implemented configurable filtering for bundled skills via whitelist/blacklist patterns with wildcard support.

## Configuration Schema

```toml
[skills]
# If true, only skills in `bundled_whitelist` are exposed
whitelist_mode = false

# List of category/skill patterns to include (supports wildcards)
# Only used when whitelist_mode = true
bundled_whitelist = ["github/*", "software-development/plan"]

# List of category/skill patterns to exclude (supports wildcards)
# Applied AFTER whitelist filtering
bundled_blacklist = ["gaming/*", "creative/ascii-art"]
```

## Pattern Syntax

- **Exact match:** `"github/github-pr-workflow"` - matches single skill
- **Wildcard category:** `"github/*"` - matches all skills in `github/` category
- **Multiple patterns:** `["github/*", "software-development/*"]` - union match

## Filter Logic

1. **Blacklist always applies first** - excluded skills removed regardless of whitelist mode
2. **Whitelist mode enabled** - only skills matching whitelist patterns are included
3. **Whitelist mode disabled** - all skills included except blacklisted ones
4. **Pattern matching** - exact match OR category wildcard match

## Modified Files

### Core Infrastructure
| File | Changes |
|------|---------|
| `crates/config/src/schema/runtime.rs` | Added 3 fields to `SkillsConfig` struct |
| `crates/config/src/template.rs` | Added commented config examples |
| `crates/config/src/schema.rs` | Removed duplicate `SkillsConfig` definition |
| `crates/config/src/validate/schema_map.rs` | Added all 9 skills fields to validation schema |

### Skills System
| File | Changes |
|------|---------|
| `crates/skills/src/bundled.rs` | Modified `discover()` to accept `Option<&SkillsConfig>`, added `matches_pattern()` helper, 10 new tests |
| `crates/skills/src/discover.rs` | Updated `CompositeSkillDiscoverer` to accept and pass `SkillsConfig` |

### Gateway Services
| File | Changes |
|------|---------|
| `crates/gateway/src/services/skills.rs` | Updated `list()` and `install_dep()` to load config and pass to discoverer (2 locations) |
| `crates/gateway/src/server/prepare_core/post_state.rs` | Updated `ReadSkillTool` initialization to pass config |

### Chat & Web
| File | Changes |
|------|---------|
| `crates/chat/src/prompt.rs` | Updated skill discovery to pass config |
| `crates/web/src/api.rs` | Updated `/api/skills` endpoint to pass config |

## Test Results

```
✅ moltis-skills: 166/166 tests passed
   - 10 new whitelist/blacklist tests
   - All existing tests passing

✅ moltis-config: 280/283 tests passed
   - 3 pre-existing failures (unrelated to skills changes)
   - loader::tests::core::share_dir_data_dir_fallback
   - loader::tests::core::workspace_markdown_* (2 tests)

✅ cargo check: moltis-config, moltis-skills, moltis-chat
   - All compile successfully with --features bundled-skills
```

## Usage Examples

### Enable only GitHub skills
```toml
[skills]
whitelist_mode = true
bundled_whitelist = ["github/*"]
```

### Exclude gaming and social media
```toml
[skills]
whitelist_mode = false
bundled_blacklist = ["gaming/*", "social-media/*"]
```

### Complex filtering
```toml
[skills]
whitelist_mode = true
bundled_whitelist = ["software-development/*", "github/*"]
bundled_blacklist = ["software-development/plan", "github/github-auth"]
# Result: all software-dev + github skills EXCEPT 'plan' and 'github-auth'
```

## Backward Compatibility

- Default values preserve current behavior (all bundled skills enabled)
- `whitelist_mode = false` (default) - no filtering applied
- Existing `disabled_bundled_categories` field still supported
- Migration path: users can move from category-level to skill-level control

## Integration Points

The filtering is applied at discovery time, affecting:
1. **Skill list RPC** (`skills.list`) - returns filtered list
2. **Prompt generation** - only discoverable skills appear in system prompt
3. **Skill activation** - non-discoverable skills cannot be activated
4. **Dependency installation** - only discoverable skills can have deps installed

## Next Steps (Optional)

1. **UI Settings** - Add whitelist/blacklist editors to web UI
2. **Migration Helper** - Convert `disabled_bundled_categories` to new blacklist format
3. **Documentation** - Update user guide with pattern syntax examples
4. **Integration Tests** - End-to-end test with running moltis instance

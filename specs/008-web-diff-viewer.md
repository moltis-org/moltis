# Spec: Web UI Code Diff Viewer

**Status:** Proposed  
**Created:** 2026-04-29  
**Author:** Council  
**Labels:** enhancement, web-ui, code-index  

---

## Summary

Add a diff viewer to the Moltis web UI that allows users to view file changes in indexed projects. This displays the delta between the current file state and the last indexed snapshot.

---

## Motivation

The `code-index` crate already computes `SyncDelta` (added/removed/modified files) internally via `crates/code-index/src/delta.rs`, but this data never reaches the frontend. A diff viewer would help users:

- Review changes since last index
- Understand what content is being searched
- Audit code changes across indexing cycles
- Visualize incremental indexing impact

---

## Current State

### Backend (Rust)
| Component | Status | Location |
|-----------|--------|----------|
| `SyncDelta` struct | ✅ Exists | `crates/code-index/src/delta.rs:29` |
| Delta computation | ✅ Implemented | `compute_delta()` |
| File content retrieval | ❌ Missing | No gateway method |
| Diff generation | ❌ Missing | Need `similar` crate |

### Frontend (TypeScript/Preact)
| Component | Status | Location |
|-----------|--------|----------|
| Projects page | ✅ Exists | `crates/web/ui/src/pages/ProjectsPage.tsx` |
| `code_index_enabled` toggle | ✅ Implemented | Checkbox in edit form |
| Index status display | ❌ Missing | — |
| Diff viewer component | ❌ Missing | — |

---

## Implementation Plan

### Phase 1: Backend API (Week 1)

**1.1 Add dependencies**
```toml
# Root Cargo.toml [workspace.dependencies]
similar = { version = "2.7", features = ["inline", "serde"] }
```

**1.2 Create new RPC methods** (`crates/gateway/src/methods/services/code_index.rs` — NEW FILE)

```rust
pub(super) fn register(reg: &mut MethodRegistry) {
    reg.register("code_index.status", /* → IndexStatus + SyncDelta */);
    reg.register("code_index.file_content", /* → String */);
    reg.register("code_index.diff", /* → DiffResult */);
}
```

**1.3 Extend types** (`crates/code-index/src/types.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    pub relative_path: PathBuf,
    pub language: Language,
    pub old_content: Option<String>,
    pub new_content: Option<String>,
    pub unified_diff: String,
    pub stats: DiffStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffStats {
    pub additions: usize,
    pub deletions: usize,
    pub files_changed: usize,
}
```

**1.4 Register methods** (`crates/gateway/src/methods/services/admin.rs`)

```rust
#[cfg(any(feature = "qmd", feature = "code-index-builtin"))]
{
    mod code_index;
    code_index::register(reg);
}
```

**1.5 Add REST endpoints** (`crates/web/src/lib.rs`)

```rust
GET /api/code-index/:project_id/status
GET /api/code-index/:project_id/files/:path
GET /api/code-index/:project_id/diff/:path
```

---

### Phase 2: Basic Frontend (Week 2)

**2.1 Create store** (`crates/web/ui/src/stores/code-index-store.ts` — NEW FILE)

```typescript
import { signal } from "@preact/signals";
import { sendRpc } from "../helpers";

interface CodeIndexState {
  project_id: string | null;
  status: IndexStatus | null;
  files: FileEntry[];
  loaded: boolean;
  error: string | null;
}

export const codeIndexState = signal<CodeIndexState>({...});

export async function loadIndexStatus(projectId: string): Promise<void> {
  const res = await sendRpc("code_index.status", { project_id: projectId });
  // ...
}
```

**2.2 Create page** (`crates/web/ui/src/pages/CodeIndexPage.tsx` — NEW FILE)

- Tabs: Overview | Files | Diffs
- Overview: Project stats, last indexed, total files/chunks
- Files: File tree with language badges, size, last indexed

**2.3 Add routes** (`crates/web/ui/src/routes.ts`)

```typescript
export const routes = {
  // existing...
  codeIndex: "/projects/:project_id/code-index",
};
```

**2.4 Integrate with ProjectsPage**

Add "View Index" button on project cards where `code_index_enabled === true`.

---

### Phase 3: Diff Viewer Component (Week 3-4)

**3.1 Create component** (`crates/web/ui/src/components/DiffViewer.tsx` — NEW FILE)

Features:
- Unified/side-by-side toggle
- Hunk collapse/expand
- Syntax highlighting (reuse existing highlighter)
- Line numbers
- Copy diff button
- Download .patch file

**3.2 Add CSS** (`crates/web/ui/src/input.css`)

```css
.diff-container { }
.diff-line-add { background: #e6ffec; }
.diff-line-remove { background: #ffebe9; }
.diff-hunk-header { cursor: pointer; }
.diff-split { display: grid; grid-template-columns: 1fr 1fr; }
```

**3.3 Wire up diff API**

Call `code_index.diff()` on file selection, render unified diff.

---

### Phase 4: Polish (Week 5)

- [ ] Loading states, skeletons
- [ ] Error handling (file not found, binary file, too large)
- [ ] Keyboard shortcuts (j/k navigation, space toggle hunk)
- [ ] File size limit warning (>1MB)
- [ ] Binary file detection & skip
- [ ] Empty state illustrations

---

## File Impact Summary

| Area | New Files | Modified Files |
|------|-----------|----------------|
| Backend | 2 | 4 |
| Frontend | 3 | 4 |
| Dependencies | — | 2 (crate + npm) |
| **Total** | **5** | **8** |

---

## Technical Notes

### File Content Strategy
- **Read from disk** on-demand (not stored in DB)
- Use `FilteredFile.path` from `SyncDelta`
- Protect against path traversal (reuse `projects.complete_path` sanitization)

### Binary Detection
- Reuse existing logic from `crates/code-index/src/chunker.rs`
- Skip diff generation for binary files
- Show "Binary file, X bytes" placeholder

### Performance
- Limit diff generation to files <1MB
- Background job for large diffs
- Cache diff results in memory (TTL 5min)

### Security
- Require authentication (same as projects.* methods)
- Validate `project_id` ownership
- Sanitize `relative_path` parameter

---

## Existing Patterns to Reuse

### Backend
- Method registration: `memory.config.get` (`admin.rs:872`)
- File I/O: `read_file` tool (`crates/tools/src/fs/read/tool.rs`)
- Error handling: `Result<Value, ErrorShape>`

### Frontend
- Page structure: `SkillsPage.tsx` (tabs, filters, list)
- RPC calls: `sendRpc()` helper (`helpers.ts`)
- State signals: `project-store.ts`
- Components: `Badge`, `TabBar`, `ListItem`

---

## Dependencies

| Name | Version | Purpose |
|------|---------|---------|
| `similar` (Rust) | 2.7 | Unified diff generation |
| `diff` (npm) | optional | Client-side diff (if not server-side) |

---

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| File access permissions | Medium | Test in sandbox mode, document requirements |
| Large file performance | Medium | Enforce 1MB limit, show warning |
| Binary file corruption | Low | Detect early, skip gracefully |
| Path traversal attack | High | Sanitize all paths, validate project ownership |

---

## Future Enhancements

- [ ] Git integration (show `git diff` instead of index delta)
- [ ] Historical diffs (compare arbitrary snapshots)
- [ ] Multi-file diff view (like `git diff` output)
- [ ] Comment threads on diff lines
- [ ] Real-time updates on file watcher events
- [ ] Multi-cursor viewing for pair programming

---

## Related

- PR #921: Code index auto-trigger
- `crates/code-index/src/delta.rs`: SyncDelta implementation
- `crates/code-index/src/watcher/`: File watcher lifecycle
- `projects.*` RPC methods: Project management

---

## Acceptance Criteria

- [ ] User can view index status for enabled projects
- [ ] User can see list of modified files since last index
- [ ] User can view unified diff for any modified file
- [ ] User can toggle side-by-side mode
- [ ] User can collapse/expand hunks
- [ ] Binary files are detected and skipped
- [ ] Files >1MB show warning before diff generation
- [ ] Keyboard navigation works (j/k/space)
- [ ] Diff can be copied or downloaded as .patch

---

**Next Step:** Create GitHub issue upstream (moltis-org/moltis) or implement on `feat/diff-viewer` branch.

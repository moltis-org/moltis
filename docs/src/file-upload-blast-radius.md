# File Upload Feature Blast Radius Analysis

## Objective
Add secure file upload capability to the web UI, allowing users to transfer files of various types through the session interface. A file upload button (+) will be placed next to the voice recording activation, transferring files to a session-accessible tmp directory with periodic cleanup.

---

## Current State Summary

### Existing Upload Infrastructure
✅ **Backend already implements**: `POST /api/sessions/{session_key}/upload`
- Accepts raw binary data with Content-Type header
- Supports audio (with optional STT transcription), images, and generic files
- Max upload size: 25 MB (`MAX_UPLOAD_SIZE` in `upload_routes.rs`)
- Files stored via `session_store.save_media()`
- Files retrievable via `GET /api/sessions/{session_key}/media/{filename}`

✅ **Frontend already implements**:
- Voice recording uploads to session media (`voice-input.ts`, `voice-utils.ts`)
- Image drag-and-drop for chat (`media-drop.ts`) - images only, 20 MB limit
- Media rendering in chat messages (`session-render.ts`, `message-voice.ts`, `tool-helpers.ts`)
- Document display in tool cards (existing pattern in `tool-helpers.ts`)

---

## Files Requiring Changes

### 1. Backend (Rust)

#### Core Upload Handler
- **File**: `crates/httpd/src/upload_routes.rs`
- **Changes**: 
  - ✅ Already supports generic file upload
  - ⚠️ May need to expand file type validation/sanitization
  - ⚠️ May need to add file type categorization (document, code, data, etc.)
  - ⚠️ Consider adding file metadata tracking (original name, MIME type detection)
- **Blast Radius**: ⚠️ **HIGH** - Core upload logic affects all file handling

#### Session Store Implementation
- **File**: `crates/gateway/src/session_store.rs` (or equivalent)
- **Action**: Search for `impl.*save_media|fn save_media`
- **Changes**: Verify tmp directory handling, ensure session isolation
- **Blast Radius**: ⚠️ **HIGH** - Affects session data persistence

#### API Routes Registration
- **File**: `crates/web/src/lib.rs`
- **Lines**: 200-205 (already registered)
- **Changes**: None required - route already exists
- **Blast Radius**: ✅ None

#### Media Retrieval Handler
- **File**: `crates/web/src/api.rs`
- **Function**: `api_session_media_handler`
- **Changes**: 
  - Verify security checks (session ownership, path traversal prevention)
  - Consider adding Content-Type headers for proper browser handling
- **Blast Radius**: ⚠️ **MEDIUM** - Security-critical

#### Temp File Cleanup Service
- **File**: **NEW** - `crates/httpd/src/media_cleanup.rs` (or similar)
- **Purpose**: Periodic cleanup of session tmp files
- **Integration**: Hook into existing cron system or session lifecycle
- **Blast Radius**: ⚠️ **MEDIUM** - Data loss risk if misconfigured

#### Cleanup Integration Points
- **File**: `crates/httpd/src/server/runtime.rs` or `crates/httpd/src/lib.rs`
- **Changes**: Register cleanup task on server startup
- **Blast Radius**: ⚠️ **MEDIUM** - Affects server lifecycle

---

### 2. Frontend (TypeScript/TSX)

#### Chat Page HTML Template
- **File**: `crates/web/ui/src/pages/ChatPage.tsx`
- **Location**: Find `chatPageHTML` constant (static HTML string)
- **Changes**: 
  - Add `<input type="file" id="fileUploadInput" multiple />` (hidden)
  - Add `<button id="fileUploadBtn" class="chat-input-btn" title="Upload files">+</button>`
  - Position next to voice input button
- **Blast Radius**: ⚠️ **HIGH** - Main chat UI, affects all users

#### File Upload Handler Module
- **File**: **NEW** - `crates/web/ui/src/file-upload.ts`
- **Purpose**: Handle file selection, upload, progress, error handling
- **Functions**:
  - `uploadFile(sessionKey: string, file: File): Promise<UploadResponse>`
  - `triggerFileSelect(): void`
  - `handleFileUpload(files: File[]): Promise<void>`
  - `renderFilePreviews(container: HTMLElement, files: File[]): void`
  - `clearFilePreviews(): void`
- **Dependencies**: `helpers.ts` (sendRpc, renderMarkdown), `state.ts`, `i18n.ts`
- **Blast Radius**: ✅ **LOW** - New module

#### Chat Send Integration
- **File**: `crates/web/ui/src/pages/chat/chat-send.ts`
- **Function**: `sendChat()`
- **Changes**: 
  - Extend to handle file attachments alongside text
  - Include uploaded file URLs in message payload
- **Blast Radius**: ⚠️ **HIGH** - Core chat functionality

#### Message Rendering
- **File**: `crates/web/ui/src/sessions/session-render.ts`
- **Functions**: `renderUserMessage()`, `renderAssistantMessage()`
- **Changes**: 
  - Add support for rendering file attachments (documents, code files, etc.)
  - Create file card/preview UI similar to image thumbnails
- **Blast Radius**: ⚠️ **MEDIUM** - Affects message display

#### File Preview Component
- **File**: **NEW** - `crates/web/ui/src/file-preview.ts` (or integrate into `chat-ui.ts`)
- **Purpose**: Render file attachments in chat
- **UI Elements**:
  - File icon based on MIME type
  - Filename with size
  - Download/preview button
  - Remove button (before sending)
- **Blast Radius**: ✅ **LOW** - New component

#### Media Drop Zone Enhancement
- **File**: `crates/web/ui/src/media-drop.ts`
- **Changes**: 
  - Expand `ACCEPTED_TYPES` beyond images
  - Handle non-image files separately (document cards vs image thumbnails)
  - Update `handleFiles()` to route to new file upload handler
- **Blast Radius**: ⚠️ **MEDIUM** - Existing drag-drop behavior

#### Voice Input Module
- **File**: `crates/web/ui/src/voice-input.ts`
- **Location**: Near `initVoiceInput()` function
- **Changes**: 
  - Wire up file upload button click handler
  - Position button in DOM next to mic button
- **Blast Radius**: ⚠️ **MEDIUM** - Voice recording flow

#### State Management
- **File**: `crates/web/ui/src/state.ts`
- **Changes**: 
  - Add `$fileUploadBtn` signal/element reference
  - Add `$fileUploadInput` signal/element reference
  - Add `pendingFileUploads` array for preview management
- **Blast Radius**: ⚠️ **HIGH** - Global state affects all UI components

#### Localization Keys
- **Files**: 
  - `crates/web/ui/src/locales/en/chat.ts`
  - `crates/web/ui/src/locales/fr/chat.ts`
  - `crates/web/ui/src/locales/zh/chat.ts`
- **Keys to Add**:
  - `fileUploadTooltip`: "Upload files"
  - `fileUploadSelectFiles`: "Select files to upload"
  - `fileUploadMaxSize`: "Maximum file size: {size} MB"
  - `fileUploadFailed`: "File upload failed: {error}"
  - `fileTypeNotSupported`: "File type not supported: {type}"
- **Blast Radius**: ✅ **LOW** - Translation strings only

#### Styles
- **File**: `crates/web/src/assets/css/style.css`
- **Selectors to Add**:
  - `.chat-file-upload-btn` - Upload button styling
  - `.file-preview-container` - File preview strip container
  - `.file-preview-item` - Individual file preview card
  - `.file-preview-icon` - File type icon
  - `.file-preview-info` - Filename and metadata
  - `.file-preview-remove` - Remove file button
- **Blast Radius**: ⚠️ **MEDIUM** - Global CSS

---

### 3. Database/Schema

#### Session Media Tracking
- **Current**: Sessions stored in `sessions` table, media in filesystem
- **Check**: `crates/gateway/src/session_store.rs` for storage pattern
- **Changes**: 
  - May need to track file metadata (path, size, type, session_key, created_at)
  - Consider adding to `files` table or creating `session_media` table
- **Blast Radius**: ⚠️ **HIGH** - Schema changes require migrations

#### Cleanup Metadata
- **File**: `crates/gateway/src/session_store.rs` or separate cleanup service
- **Purpose**: Track file TTL, last access for cleanup decisions
- **Blast Radius**: ⚠️ **MEDIUM**

---

### 4. Configuration

#### Config Schema Updates
- **File**: `crates/config/src/schema.rs`
- **Section**: `MoltisConfig` → `tools` or `session` config
- **Fields to Add**:
  - `max_upload_size_mb: u64` (default: 25)
  - `session_media_ttl_hours: u64` (default: 24 or 168 for 1 week)
  - `cleanup_interval_hours: u64` (default: 1)
  - `allowed_file_types: Vec<String>` (optional whitelist)
- **Blast Radius**: ⚠️ **HIGH** - Config schema affects all instances

#### Config Validation
- **File**: `crates/config/src/validate.rs`
- **Function**: `build_schema_map()`
- **Changes**: Add new fields to schema map
- **Blast Radius**: ⚠️ **MEDIUM** - Validation rules

#### Frontend Config (Gon)
- **File**: `crates/web/src/gon.rs`
- **Purpose**: Pass server config to frontend
- **Changes**: Expose upload limits, allowed types to frontend
- **Blast Radius**: ⚠️ **LOW** - Frontend config only

---

### 5. Security

#### File Type Validation
- **File**: `crates/httpd/src/upload_routes.rs`
- **Changes**: 
  - Implement MIME type verification (magic bytes, not just extension)
  - Block dangerous types: `.exe`, `.bat`, `.sh`, `.js`, `.html` (unless explicitly allowed)
  - Sanitize filenames (prevent path traversal, unicode normalization)
- **Blast Radius**: ⚠️ **CRITICAL** - Security boundary

#### Size Limits
- **File**: `crates/httpd/src/upload_routes.rs`
- **Current**: `MAX_UPLOAD_SIZE = 25 MB`
- **Action**: Verify this is appropriate for document types
- **Blast Radius**: ⚠️ **HIGH** - DoS prevention

#### Rate Limiting
- **File**: May need new middleware or use existing rate limiter
- **Location**: `crates/httpd/src/request_throttle.rs` or middleware stack
- **Changes**: Limit uploads per session per minute
- **Blast Radius**: ⚠️ **MEDIUM** - Rate limiting affects all API calls

#### Session Isolation
- **File**: `crates/gateway/src/session_store.rs`
- **Verify**: Session A cannot access Session B's media
- **Blast Radius**: ⚠️ **CRITICAL** - Security boundary

#### Auth Middleware
- **File**: `crates/httpd/src/auth_middleware.rs`
- **Current**: Upload endpoint already requires auth (line 255, 278)
- **Verify**: No bypasses introduced
- **Blast Radius**: ⚠️ **CRITICAL** - Security boundary

---

### 6. Testing

#### Backend Tests
- **Files**:
  - `crates/httpd/tests/auth_middleware.rs` - Already has upload endpoint tests (line 996-1026)
  - `crates/web/tests/*.rs` (if exists)
  - **NEW**: `crates/httpd/tests/upload_routes.rs`
- **Tests to Add**:
  - Upload various file types (PDF, DOCX, TXT, ZIP, code files)
  - Upload oversized files (verify rejection)
  - Upload with malicious filenames (`../../evil.txt`)
  - Upload with missing Content-Type
  - Concurrent uploads to same session
  - Media retrieval authorization
  - Cleanup service functionality
- **Blast Radius**: ⚠️ **HIGH** - Test coverage critical

#### Frontend Tests
- **File**: `crates/web/ui/e2e/specs/*.spec.js`
- **Tests to Add**:
  - `file-upload.spec.js` - E2E file upload flow
  - Test drag-and-drop for non-image files
  - Test file preview rendering
  - Test upload progress indicators
  - Test upload error handling
- **Blast Radius**: ⚠️ **MEDIUM**

---

### 7. Documentation

#### API Documentation
- **File**: `docs/src/api.md` or similar
- **Add**: Document `/api/sessions/{session_key}/upload` endpoint
- **Blast Radius**: ✅ **LOW**

#### User Guide
- **File**: `docs/src/user-guide.md` or UI help text
- **Add**: How to upload files, supported types, size limits
- **Blast Radius**: ✅ **LOW**

#### Changelog
- **File**: `CHANGELOG.md` or `cliff.toml`
- **Add**: Feature entry for file upload capability
- **Blast Radius**: ✅ **LOW**

---

## File Types to Support (Prioritized)

### Phase 1 (MVP)
- ✅ Images: PNG, JPEG, GIF, WebP (already supported)
- ✅ Audio: WebM, WAV, MP3, OGG, FLAC (already supported)
- 🆕 Documents: PDF, TXT, MD, DOCX, ODT
- 🆕 Data: CSV, JSON, XML, YAML

### Phase 2 (Extended)
- 🆕 Archives: ZIP, TAR, GZ (extract to session tmp?)
- 🆕 Code: All common source files (.rs, .py, .js, .ts, .java, .cpp, etc.)
- 🆕 Spreadsheets: XLSX, ODS

### Blocked Types (Security)
- ❌ Executables: EXE, BAT, SH, PS1, COM
- ❌ Web: HTML, HTM, JS (unless in ZIP), PHP
- ❌ Scripts: PY, RB, PL (context-dependent)
- ❌ Office Macros: DOCM, XLSM, PPTM
- ❌ System: DLL, SO, DYLIB

---

## Implementation Phases

### Phase 1: Core Upload (+) Button (Backend Ready)
**Effort**: 2-3 days
**Files**: `ChatPage.tsx`, `file-upload.ts`, `state.ts`, `style.css`
**Backend**: Minimal - already functional
**Testing**: Basic upload/download flow

### Phase 2: File Previews & Integration
**Effort**: 2-3 days
**Files**: `file-preview.ts`, `chat-send.ts`, `session-render.ts`, `media-drop.ts`
**Features**: File cards, multi-file support, progress indicators
**Testing**: E2E tests, error handling

### Phase 3: Security Hardening
**Effort**: 1-2 days
**Files**: `upload_routes.rs`, `auth_middleware.rs`, rate limiting
**Features**: MIME validation, filename sanitization, rate limits
**Testing**: Security tests, penetration testing

### Phase 4: Cleanup Service
**Effort**: 1-2 days
**Files**: **NEW** `media_cleanup.rs`, integration in `runtime.rs`
**Features**: Scheduled cleanup, TTL-based deletion
**Testing**: Verify cleanup runs, no premature deletion

### Phase 5: Extended File Types
**Effort**: 1-2 days
**Files**: Upload validation, preview components
**Features**: Support archives, code files with syntax highlighting
**Testing**: Type-specific tests

---

## Total Blast Radius Summary

### Critical (Security/Correctness)
1. `crates/httpd/src/upload_routes.rs` - File validation, sanitization
2. `crates/httpd/src/auth_middleware.rs` - Auth bypass prevention
3. `crates/gateway/src/session_store.rs` - Session isolation
4. `crates/config/src/schema.rs` - Config limits

### High Impact (Widely Used)
1. `crates/web/ui/src/pages/ChatPage.tsx` - Main chat UI
2. `crates/web/ui/src/state.ts` - Global UI state
3. `crates/web/ui/src/pages/chat/chat-send.ts` - Chat sending logic
4. `crates/web/src/api.rs` - Media retrieval handler

### Medium Impact (Localized)
1. `crates/web/ui/src/media-drop.ts` - Drag-drop enhancement
2. `crates/web/ui/src/voice-input.ts` - Button placement
3. `crates/web/src/assets/css/style.css` - Styling
4. `crates/web/ui/src/sessions/session-render.ts` - Message rendering

### Low Impact (New/Isolated)
1. `crates/web/ui/src/file-upload.ts` - New module
2. `crates/web/ui/src/file-preview.ts` - New component
3. Localization files
4. Documentation

---

## Dependencies Checklist

- ✅ Backend upload endpoint: EXISTS
- ✅ Session media storage: EXISTS
- ✅ Media retrieval endpoint: EXISTS
- ✅ Frontend state management: EXISTS
- ⚠️ File type detection library (Rust): `infer` crate or `magic`
- ⚠️ Filename sanitization: Custom or `sanitize-filename` crate
- ⚠️ Cleanup scheduler: Integrate with existing cron system
- ⚠️ Frontend file icons: FontAwesome or custom SVGs

---

## Risk Assessment

### High Risk
- **Path traversal via filename**: Must sanitize rigorously
- **Session media leakage**: Isolation must be verified
- **DoS via large uploads**: Rate limiting mandatory
- **XSS via uploaded HTML/SVG**: Block or sanitize

### Medium Risk
- **Storage exhaustion**: Cleanup service must be reliable
- **File type spoofing**: MIME detection via magic bytes
- **Concurrent upload conflicts**: Atomic writes required

### Low Risk
- **UI performance**: File previews manageable with lazy loading
- **Browser compatibility**: Modern browsers support file API
- **Translation gaps**: Default to English fallbacks

---

## Next Steps

1. **Read full `upload_routes.rs`** - Understand current implementation
2. **Locate `session_store.rs`** - Verify storage mechanism
3. **Search for cleanup mechanisms** - Existing cron or session cleanup
4. **Review `ChatPage.tsx`** - Find exact location for button insertion
5. **Audit security middleware** - Ensure no bypasses
6. **Create `file-upload.ts`** - Implement core upload function
7. **Update `ChatPage.tsx`** - Add button and input elements
8. **Wire event handlers** - Connect button to upload logic
9. **Add file preview component** - Show pending uploads
10. **Integration test** - Full flow from UI to backend to retrieval

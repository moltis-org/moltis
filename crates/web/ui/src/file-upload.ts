// ── File Upload Module ───────────────────────────────────────
// Handles file upload to session media storage.
// Supports multiple file types: documents, code files, data files, images, audio.
// Files are uploaded to session-accessible tmp directory with cleanup.

import { t } from "./i18n";
import * as S from "./state";

// ── Configuration ────────────────────────────────────────────
export const MAX_FILE_SIZE = 25 * 1024 * 1024; // 25 MB (matches backend MAX_UPLOAD_SIZE)

// Allowed MIME types by category
const ALLOWED_TYPES: Record<string, string[]> = {
	// Documents
	"application/pdf": [".pdf"],
	"text/plain": [".txt", ".md", ".log"],
	"text/markdown": [".md"],
	"application/vnd.openxmlformats-officedocument.wordprocessingml.document": [".docx"],
	"application/rtf": [".rtf"],

	// Data files
	"text/csv": [".csv"],
	"application/json": [".json"],
	"application/xml": [".xml"],
	"text/xml": [".xml"],
	"application/yaml": [".yaml", ".yml"],
	"text/yaml": [".yaml", ".yml"],

	// Code files (common languages) — note: shell/ruby code uploads allowed
	// only when MIME type is set by the browser (e.g. text/x-rust).
	// Raw .sh/.rb extensions are blocked in BLOCKED_EXTENSIONS as a safety net.
	"text/x-rust": [".rs"],
	"text/x-python": [".py"],
	"text/typescript": [".ts", ".tsx"],
	"text/x-java": [".java"],
	"text/x-c++": [".cpp", ".cc", ".cxx", ".h", ".hpp"],
	"text/x-c": [".c", ".h"],
	"text/x-go": [".go"],

	// Images (already supported by existing flow)
	"image/png": [".png"],
	"image/jpeg": [".jpg", ".jpeg"],
	"image/gif": [".gif"],
	"image/webp": [".webp"],

	// Audio (already supported by existing flow)
	"audio/webm": [".webm"],
	"audio/wav": [".wav"],
	"audio/mpeg": [".mp3"],
	"audio/ogg": [".ogg"],
	"audio/flac": [".flac"],
};

// Blocked file extensions (security).
// These are blocked by extension regardless of MIME type.
// Note: .sh/.rb/.pl are intentionally blocked here even though some code
// MIME types (text/x-shellscript, text/x-ruby) are in ALLOWED_TYPES.
// The extension block wins — users must not upload executable scripts.
const BLOCKED_EXTENSIONS = new Set([
	".exe",
	".bat",
	".cmd",
	".com",
	".scr",
	".pif", // Windows executables
	".sh",
	".bash",
	".zsh",
	".fish", // Shell scripts
	".ps1",
	".psm1",
	".psd1", // PowerShell
	".html",
	".htm",
	".xhtml", // Web pages (XSS risk)
	".php",
	".phtml", // PHP
	".asp",
	".aspx",
	".asa",
	".asax", // ASP.NET
	".jsp",
	".jspx", // JSP
	".pl",
	".pm", // Perl
	".rb", // Ruby
	".dll",
	".so",
	".dylib", // Shared libraries
	".docm",
	".xlsm",
	".pptm", // Office with macros
	".jar",
	".war", // Java archives
	".js",
	".mjs", // JavaScript (XSS risk — served from same origin)
]);

// ── Upload Response Types ────────────────────────────────────
export interface UploadResponse {
	ok: boolean;
	url?: string;
	filename?: string;
	contentType?: string;
	size?: number;
	transcription?: { text: string; language?: string };
	transcriptionError?: string;
	error?: string;
	code?: string;
}

export interface PendingFileUpload {
	file: File;
	dataUrl?: string;
	previewUrl?: string;
	uploading: boolean;
	progress: number;
	error?: string;
	url?: string;
	filename?: string;
}

// ── File Type Utilities ──────────────────────────────────────

function getFileCategory(mimeType: string): string {
	if (
		mimeType.startsWith("application/pdf") ||
		mimeType.startsWith("text/plain") ||
		mimeType.includes("wordprocessing")
	) {
		return "document";
	}
	if (mimeType.includes("csv") || mimeType.includes("json") || mimeType.includes("xml") || mimeType.includes("yaml")) {
		return "data";
	}
	if (mimeType.startsWith("image/")) {
		return "image";
	}
	if (mimeType.startsWith("audio/")) {
		return "audio";
	}
	return "unknown";
}

export function getFileIconClass(mimeType: string): string {
	const category = getFileCategory(mimeType);
	switch (category) {
		case "document":
			return "icon-document";
		case "data":
			return "icon-data";
		case "image":
			return "icon-image";
		case "audio":
			return "icon-audio";
		default:
			return "icon-file";
	}
}

export function isFileTypeAllowed(file: File): { allowed: boolean; reason?: string } {
	const ext = `.${file.name.split(".").pop()?.toLowerCase() ?? ""}`;

	// Check blocked extensions first — this wins over MIME type allowlist
	if (BLOCKED_EXTENSIONS.has(ext)) {
		return {
			allowed: false,
			reason: t("chat:fileTypeBlocked", { extension: ext }),
		};
	}

	// Check if MIME type is in allowed list
	if (ALLOWED_TYPES[file.type]) {
		return { allowed: true };
	}

	// Unknown MIME type — allow only known-safe archive types
	const safeArchiveTypes = new Set([
		"application/octet-stream",
		"application/zip",
		"application/x-tar",
		"application/gzip",
	]);

	if (safeArchiveTypes.has(file.type) && [".zip", ".tar", ".gz", ".tgz"].includes(ext)) {
		return { allowed: true };
	}

	// Reject unknown types
	return {
		allowed: false,
		reason: t("chat:fileTypeNotSupported", { type: file.type || ext }),
	};
}

function sanitizeFilename(filename: string): string {
	// Remove path components using a single split on both separators
	const basename = filename.split(/[/\\]/).pop() ?? "unnamed";
	// Replace dangerous characters: < > : " | ? *
	const dangerousChars = /[<>:"|?*]/g;
	let sanitized = basename.replace(dangerousChars, "_");
	// Strip non-printable characters (C0 controls 0x00-0x1F and DEL 0x7F)
	// biome-ignore lint/suspicious/noControlCharactersInRegex: intentional control-char sanitization for security
	sanitized = sanitized.replace(/[\x00-\x1f\x7f]/g, "");
	// Limit length from the head to preserve meaningful filename prefix
	if (sanitized.length > 200) {
		return sanitized.substring(0, 200);
	}
	return sanitized || "unnamed";
}

export function formatFileSize(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

// ── Upload Functions ─────────────────────────────────────────
export async function uploadFile(file: File, options?: { transcribe?: boolean }): Promise<UploadResponse> {
	// Validate file type
	const typeCheck = isFileTypeAllowed(file);
	if (!typeCheck.allowed) {
		return {
			ok: false,
			error: typeCheck.reason || t("chat:fileTypeNotSupported", { type: file.type }),
			code: "FILE_TYPE_BLOCKED",
		};
	}

	// Validate file size
	if (file.size > MAX_FILE_SIZE) {
		return {
			ok: false,
			error: t("chat:fileTooLarge", {
				size: (file.size / 1024 / 1024).toFixed(2),
				max: (MAX_FILE_SIZE / 1024 / 1024).toFixed(0),
			}),
			code: "FILE_TOO_LARGE",
		};
	}

	// Sanitize filename
	const sanitizedFilename = sanitizeFilename(file.name);

	try {
		// Build upload URL — use S.activeSessionKey (plain string) for consistency
		const uploadUrl = `/api/sessions/${encodeURIComponent(S.activeSessionKey)}/upload`;

		// Build query params
		const params = new URLSearchParams();
		if (options?.transcribe && file.type.startsWith("audio/")) {
			params.set("transcribe", "true");
		}

		// Perform upload
		const response = await fetch(`${uploadUrl}?${params.toString()}`, {
			method: "POST",
			headers: {
				"Content-Type": file.type || "application/octet-stream",
				"X-Filename": sanitizedFilename,
			},
			body: file,
		});

		const result: UploadResponse = await response.json();

		if (!response.ok) {
			return {
				ok: false,
				error: result.error || t("chat:fileUploadFailed", { error: "server error" }),
				code: result.code || "UPLOAD_FAILED",
			};
		}

		return result;
	} catch (error) {
		console.error("[file-upload] upload error:", error);
		return {
			ok: false,
			error: error instanceof Error ? error.message : t("chat:unknownError"),
			code: "UPLOAD_ERROR",
		};
	}
}

export async function uploadFiles(files: File[], options?: { transcribe?: boolean }): Promise<UploadResponse[]> {
	// Upload sequentially to avoid overwhelming the server
	const results: UploadResponse[] = [];
	for (const file of files) {
		results.push(await uploadFile(file, options));
	}
	return results;
}

// ── File Preview Helpers ─────────────────────────────────────
export function readFileAsDataUrl(file: File): Promise<string> {
	return new Promise((resolve, reject) => {
		// For large files, don't create data URL
		if (file.size > 5 * 1024 * 1024) {
			resolve("");
			return;
		}
		const reader = new FileReader();
		reader.onload = () => resolve(reader.result as string);
		reader.onerror = () => reject(reader.error);
		reader.readAsDataURL(file);
	});
}

// ── State Management ─────────────────────────────────────────
let pendingUploads: PendingFileUpload[] = [];

export function getPendingUploads(): PendingFileUpload[] {
	return pendingUploads;
}

export function hasPendingUploads(): boolean {
	return pendingUploads.length > 0;
}

export function addPendingUpload(entry: PendingFileUpload): void {
	pendingUploads.push(entry);
}

export function removePendingUpload(index: number): void {
	pendingUploads.splice(index, 1);
}

export function clearPendingUploads(): void {
	pendingUploads = [];
}

export function updatePendingUpload(index: number, updates: Partial<PendingFileUpload>): void {
	if (pendingUploads[index]) {
		pendingUploads[index] = { ...pendingUploads[index], ...updates };
	}
}

// ── UI Helpers ───────────────────────────────���───────────────
/** Validate and stage files into pendingUploads (does NOT upload). */
export function handleFileSelection(files: File[]): File[] {
	const validFiles: File[] = [];

	for (const file of files) {
		const typeCheck = isFileTypeAllowed(file);
		if (!typeCheck.allowed) {
			console.warn("[file-upload] skipping blocked file:", file.name, typeCheck.reason);
			continue;
		}

		if (file.size > MAX_FILE_SIZE) {
			console.warn("[file-upload] skipping oversized file:", file.name);
			continue;
		}

		validFiles.push(file);
	}

	// Stage into pending
	for (const file of validFiles) {
		addPendingUpload({
			file,
			uploading: false,
			progress: 0,
		});
	}

	return validFiles;
}

// ── Preview Strip Management ────────────────────────────────

let filePreviewStrip: HTMLElement | null = null;
let fileInputRef: HTMLInputElement | null = null;

/** Render current pending uploads as a preview strip. */
export function renderFilePreviewStrip(): void {
	if (!filePreviewStrip) return;

	filePreviewStrip.textContent = "";

	if (pendingUploads.length === 0) {
		filePreviewStrip.classList.add("hidden");
		return;
	}

	filePreviewStrip.classList.remove("hidden");

	for (let i = 0; i < pendingUploads.length; i++) {
		const entry = pendingUploads[i];
		const item = document.createElement("div");
		item.className = "file-preview-item";

		const icon = document.createElement("span");
		icon.className = "icon icon-md icon-file file-preview-icon";
		item.appendChild(icon);

		const info = document.createElement("div");
		info.className = "file-preview-info";

		const name = document.createElement("span");
		name.className = "file-preview-name";
		name.textContent = entry.file.name;
		name.title = entry.file.name;
		info.appendChild(name);

		const size = document.createElement("span");
		size.className = "file-preview-size";
		size.textContent = formatFileSize(entry.file.size);
		info.appendChild(size);

		item.appendChild(info);

		const removeBtn = document.createElement("button");
		removeBtn.type = "button";
		removeBtn.className = "file-preview-remove";
		removeBtn.textContent = "×";
		removeBtn.title = "Remove file";
		removeBtn.addEventListener("click", () => {
			removePendingUpload(i);
			renderFilePreviewStrip();
		});
		item.appendChild(removeBtn);

		filePreviewStrip.appendChild(item);
	}
}

// ── Init / Teardown ─────────────────────────────────────────

/**
 * Initialize file upload UI.
 * Creates a hidden file input, a preview strip, and binds the upload button.
 * @param btn The `+` button element (may be null — module becomes inert)
 * @param inputRow The chat input row — preview strip is inserted before it
 */
export function initFileUpload(btn: HTMLButtonElement | null, inputRow: HTMLElement | null): void {
	// Create hidden file input (not in HTML template — avoids stale refs)
	fileInputRef = document.createElement("input");
	fileInputRef.type = "file";
	fileInputRef.multiple = true;
	fileInputRef.style.display = "none";
	fileInputRef.accept = buildAcceptString();

	fileInputRef.addEventListener("change", () => {
		if (fileInputRef?.files && fileInputRef.files.length > 0) {
			handleFileSelection(Array.from(fileInputRef.files));
			renderFilePreviewStrip();
			// Reset so same file can be re-selected
			fileInputRef.value = "";
		}
	});

	if (btn) {
		btn.addEventListener("click", () => fileInputRef?.click());
	}

	// Append hidden file input to DOM — iOS Safari silently ignores .click()
	// on detached <input type="file"> elements.
	document.body.appendChild(fileInputRef);

	// Create preview strip above the input row (same pattern as media-drop)
	filePreviewStrip = document.createElement("div");
	filePreviewStrip.className = "file-preview-strip hidden";
	if (inputRow?.parentElement) {
		inputRow.parentElement.insertBefore(filePreviewStrip, inputRow);
	}
}

/**
 * Teardown file upload UI — clean up DOM elements.
 */
export function teardownFileUpload(): void {
	if (filePreviewStrip?.parentElement) {
		filePreviewStrip.parentElement.removeChild(filePreviewStrip);
	}
	filePreviewStrip = null;
	if (fileInputRef?.parentElement) {
		fileInputRef.parentElement.removeChild(fileInputRef);
	}
	fileInputRef = null;
	clearPendingUploads();
}

/**
 * Build the `accept` attribute string from ALLOWED_TYPES.
 * Returns a comma-separated list of file extensions.
 */
function buildAcceptString(): string {
	const extensions = new Set<string>();
	for (const exts of Object.values(ALLOWED_TYPES)) {
		for (const ext of exts) extensions.add(ext);
	}
	return Array.from(extensions).join(",");
}

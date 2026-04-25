// ── File Upload Module ───────────────────────────────────────
// Handles file upload to session media storage.
// Supports multiple file types: documents, code files, data files, images, audio.
// Files are uploaded to session-accessible tmp directory with cleanup.

import { t } from "./i18n";
import * as S from "./state";
import { activeSessionKey } from "./stores/session-store";

// ── Configuration ────────────────────────────────────────────
const MAX_FILE_SIZE = 25 * 1024 * 1024; // 25 MB (matches backend MAX_UPLOAD_SIZE)

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
	
	// Code files (common languages)
	"text/x-rust": [".rs"],
	"text/x-python": [".py"],
	"text/javascript": [".js"],
	"text/typescript": [".ts", ".tsx"],
	"text/x-java": [".java"],
	"text/x-c++": [".cpp", ".cc", ".cxx", ".h", ".hpp"],
	"text/x-c": [".c", ".h"],
	"text/x-go": [".go"],
	"text/x-ruby": [".rb"],
	"text/x-shellscript": [".sh", ".bash"],
	"application/x-shellscript": [".sh"],
	
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

// Blocked file types (security)
const BLOCKED_EXTENSIONS = [
	".exe", ".bat", ".cmd", ".com", ".scr", ".pif", // Windows executables
	".sh", ".bash", ".zsh", ".fish", // Shell scripts (context-dependent, allow some)
	".ps1", ".psm1", ".psd1", // PowerShell
	".html", ".htm", ".xhtml", // Web pages (XSS risk)
	".php", ".phtml", // PHP
	".asp", ".aspx", ".asa", ".asax", // ASP.NET
	".jsp", ".jspx", // JSP
	".pl", ".pm", // Perl
	".rb", // Ruby (context-dependent)
	".dll", ".so", ".dylib", // Shared libraries
	".docm", ".xlsm", ".pptm", // Office with macros
	".jar", ".war", // Java archives
];

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
function getSupportedCategories(): string[] {
	return [
		t("chat:fileTypeDocuments"),
		t("chat:fileTypeData"),
		t("chat:fileTypeCode"),
		t("chat:fileTypeImages"),
		t("chat:fileTypeAudio"),
	];
}

function getFileCategory(mimeType: string): string {
	if (mimeType.startsWith("application/pdf") || mimeType.startsWith("text/plain") || mimeType.includes("wordprocessing")) {
		return "document";
	}
	if (mimeType.includes("csv") || mimeType.includes("json") || mimeType.includes("xml") || mimeType.includes("yaml")) {
		return "data";
	}
	if (mimeType.startsWith("text/") || mimeType.includes("x-")) {
		return "code";
	}
	if (mimeType.startsWith("image/")) {
		return "image";
	}
	if (mimeType.startsWith("audio/")) {
		return "audio";
	}
	return "unknown";
}

function getFileIconClass(mimeType: string): string {
	const category = getFileCategory(mimeType);
	switch (category) {
		case "document": return "icon-document";
		case "data": return "icon-data";
		case "code": return "icon-code";
		case "image": return "icon-image";
		case "audio": return "icon-audio";
		default: return "icon-file";
	}
}

function isFileTypeAllowed(file: File): { allowed: boolean; reason?: string } {
	const ext = "." + file.name.split(".").pop()?.toLowerCase() || "";
	
	// Check blocked extensions first
	if (BLOCKED_EXTENSIONS.includes(ext)) {
		return { 
			allowed: false, 
			reason: t("chat:fileTypeBlocked", { extension: ext }) 
		};
	}
	
	// Check if MIME type is in allowed list
	const allowedExtensions = ALLOWED_TYPES[file.type];
	if (allowedExtensions) {
		return { allowed: true };
	}
	
	// Unknown MIME type - allow if extension looks safe
	const safeUnknownTypes = [
		"application/octet-stream",
		"application/zip",
		"application/x-tar",
		"application/gzip",
	];
	
	if (safeUnknownTypes.includes(file.type)) {
		// For archives, check extension
		if ([".zip", ".tar", ".gz", ".tgz"].includes(ext)) {
			return { allowed: true };
		}
	}
	
	// For unknown types, be permissive but warn
	return { allowed: true };
}

function sanitizeFilename(filename: string): string {
	// Remove path components
	let sanitized = filename.split("/").pop()?.split("\\").pop() || "unnamed";
	
	// Remove or replace dangerous characters
	sanitized = sanitized.replace(/[<>:"|?*]/g, "_");
	
	// Limit length
	if (sanitized.length > 200) {
		sanitized = sanitized.substring(0, 200);
	}
	
	return sanitized || "unnamed";
}

// ── Upload Functions ─────────────────────────────────────────
export async function uploadFile(file: File, options?: { transcribe?: boolean }): Promise<UploadResponse> {
	// Validate file type
	const typeCheck = isFileTypeAllowed(file);
	if (!typeCheck.allowed) {
		return {
			ok: false,
			error: typeCheck.reason || t("chat:fileTypeNotSupported"),
			code: "FILE_TYPE_BLOCKED",
		};
	}
	
	// Validate file size
	if (file.size > MAX_FILE_SIZE) {
		return {
			ok: false,
			error: t("chat:fileTooLarge", { 
				size: (file.size / 1024 / 1024).toFixed(2),
				max: (MAX_FILE_SIZE / 1024 / 1024).toFixed(0)
			}),
			code: "FILE_TOO_LARGE",
		};
	}
	
	// Sanitize filename
	const sanitizedFilename = sanitizeFilename(file.name);
	
	try {
		// Build upload URL
		const sessionKey = activeSessionKey || "main";
		const uploadUrl = `/api/sessions/${encodeURIComponent(sessionKey)}/upload`;
		
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
				error: result.error || t("chat:uploadFailed"),
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
	const results: UploadResponse[] = [];
	
	for (const file of files) {
		const result = await uploadFile(file, options);
		results.push(result);
	}
	
	return results;
}

// ── File Preview Helpers ─────────────────────────────────────
export function readFileAsDataUrl(file: File): Promise<string> {
	return new Promise((resolve, reject) => {
		const reader = new FileReader();
		reader.onload = () => resolve(reader.result as string);
		reader.onerror = () => reject(reader.error);
		
		// For large files, don't create data URL (use placeholder instead)
		if (file.size > 5 * 1024 * 1024) {
			resolve(""); // Return empty for large files
			return;
		}
		
		reader.readAsDataURL(file);
	});
}

export function formatFileSize(bytes: number): string {
	if (bytes < 1024) return bytes + " B";
	if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
	return (bytes / 1024 / 1024).toFixed(1) + " MB";
}

// ── State Management ─────────────────────────────────────────
let pendingUploads: PendingFileUpload[] = [];

export function getPendingUploads(): PendingFileUpload[] {
	return pendingUploads;
}

export function addPendingUpload(pending: PendingFileUpload): void {
	pendingUploads.push(pending);
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

// ── UI Helpers ───────────────────────────────────────────────
export function triggerFileSelect(accept?: string): void {
	const input = document.createElement("input");
	input.type = "file";
	input.multiple = true;
	input.accept = accept || "*/*";
	
	input.addEventListener("change", async (event: Event) => {
		const target = event.target as HTMLInputElement;
		if (target.files && target.files.length > 0) {
			const files = Array.from(target.files);
			await handleFileUpload(files);
		}
	});
	
	input.click();
}

async function handleFileUpload(files: File[]): Promise<void> {
	// Import chat-send dynamically to avoid circular deps
	const { attachFilesToMessage } = await import("./pages/chat/chat-send");
	
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
	
	if (validFiles.length > 0) {
		await attachFilesToMessage(validFiles);
	}
}

// ── Export for backwards compat ──────────────────────────────
export { getFileIconClass, formatFileSize, isFileTypeAllowed };

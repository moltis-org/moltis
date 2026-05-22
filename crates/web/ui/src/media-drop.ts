// ── Attachment drag-and-drop + paste module ─────────────────
// Handles drag-and-drop file upload, clipboard paste, and
// attachment preview strip above the chat input area.

import { documentIcon, formatDocSize } from "./helpers";
import { t } from "./i18n";
import * as S from "./state";

export interface UploadedDocumentFile {
	display_name: string;
	stored_filename: string;
	mime_type: string;
	size_bytes?: number;
	url?: string;
}

export interface PendingAttachment {
	file: File;
	name: string;
	mimeType: string;
	sizeBytes: number;
	dataUrl?: string;
}

let pendingAttachments: PendingAttachment[] = [];
let previewStrip: HTMLElement | null = null;
let chatMsgBoxRef: HTMLElement | null = null;

// Track bound handlers for teardown
let boundDragOver: ((e: DragEvent) => void) | null = null;
let boundDragEnter: ((e: DragEvent) => void) | null = null;
let boundDragLeave: ((e: DragEvent) => void) | null = null;
let boundDrop: ((e: DragEvent) => void) | null = null;
let boundPaste: ((e: ClipboardEvent) => void) | null = null;
let boundAttachClick: (() => void) | null = null;
let boundAttachChange: ((e: Event) => void) | null = null;
let dragEnterCount = 0;

const MAX_FILE_SIZE = 20 * 1024 * 1024; // 20 MB
const MAX_INLINE_IMAGE_SIZE = 5 * 1024 * 1024; // 5 MB

function isImageFile(file: File): boolean {
	return file.type.split("/", 1)[0] === "image" && file.size <= MAX_INLINE_IMAGE_SIZE;
}

function readFileAsDataUrl(file: File): Promise<string> {
	return new Promise((resolve, reject) => {
		const reader = new FileReader();
		reader.onload = (): void => {
			resolve(reader.result as string);
		};
		reader.onerror = (): void => {
			reject(reader.error);
		};
		reader.readAsDataURL(file);
	});
}

function addAttachment(file: File, dataUrl?: string): void {
	pendingAttachments.push({
		file,
		name: file.name,
		mimeType: file.type || "application/octet-stream",
		sizeBytes: file.size,
		dataUrl,
	});
	renderPreview();
}

function removeAttachment(index: number): void {
	pendingAttachments.splice(index, 1);
	renderPreview();
}

function renderPreview(): void {
	if (!previewStrip) return;
	previewStrip.textContent = "";

	if (pendingAttachments.length === 0) {
		previewStrip.classList.add("hidden");
		return;
	}

	previewStrip.classList.remove("hidden");

	for (let i = 0; i < pendingAttachments.length; i++) {
		const attachment = pendingAttachments[i];
		const item = document.createElement("div");
		item.className = "media-preview-item";

		if (attachment.dataUrl) {
			const img = document.createElement("img");
			img.className = "media-preview-thumb";
			img.src = attachment.dataUrl;
			img.alt = attachment.name;
			item.appendChild(img);
		} else {
			const icon = document.createElement("span");
			icon.className = "media-preview-file-icon";
			icon.textContent = documentIcon(attachment.mimeType, attachment.name);
			item.appendChild(icon);
		}

		const name = document.createElement("span");
		name.className = "media-preview-name";
		name.textContent = attachment.name;
		name.title = `${attachment.name} \u00b7 ${attachment.mimeType} \u00b7 ${formatDocSize(attachment.sizeBytes)}`;
		item.appendChild(name);

		const meta = document.createElement("span");
		meta.className = "media-preview-meta";
		meta.textContent = formatDocSize(attachment.sizeBytes);
		item.appendChild(meta);

		const removeBtn = document.createElement("button");
		removeBtn.className = "media-preview-remove";
		removeBtn.textContent = "\u2715";
		removeBtn.title = t("common:actions.remove");
		removeBtn.dataset.idx = String(i);
		removeBtn.addEventListener("click", (e: MouseEvent): void => {
			const idx = Number.parseInt((e.currentTarget as HTMLButtonElement).dataset.idx!, 10);
			removeAttachment(idx);
		});
		item.appendChild(removeBtn);

		previewStrip.appendChild(item);
	}
}

async function handleFiles(files: FileList | File[]): Promise<void> {
	for (const file of files) {
		if (file.size > MAX_FILE_SIZE) continue;
		try {
			const dataUrl = isImageFile(file) ? await readFileAsDataUrl(file) : undefined;
			addAttachment(file, dataUrl);
		} catch (err) {
			console.warn("[media-drop] Failed to read file:", err);
		}
	}
}

function onDragOver(e: DragEvent): void {
	e.preventDefault();
	if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
}

function onDragEnter(e: DragEvent): void {
	e.preventDefault();
	dragEnterCount++;
	if (chatMsgBoxRef) chatMsgBoxRef.classList.add("drag-over");
}

function onDragLeave(e: DragEvent): void {
	e.preventDefault();
	dragEnterCount--;
	if (dragEnterCount <= 0) {
		dragEnterCount = 0;
		if (chatMsgBoxRef) chatMsgBoxRef.classList.remove("drag-over");
	}
}

function onDrop(e: DragEvent): void {
	e.preventDefault();
	dragEnterCount = 0;
	if (chatMsgBoxRef) chatMsgBoxRef.classList.remove("drag-over");

	const files = e.dataTransfer?.files;
	if (files && files.length > 0) {
		handleFiles(files);
	}
}

function onPaste(e: ClipboardEvent): void {
	const items = e.clipboardData?.files;
	if (!items || items.length === 0) return;

	// Accept any non-empty clipboard file; handleFiles enforces size caps.
	const pastedFiles: File[] = [];
	for (const f of items) {
		if (f.size > 0) pastedFiles.push(f);
	}
	if (pastedFiles.length > 0) {
		e.preventDefault();
		handleFiles(pastedFiles);
	}
}

/**
 * Initialize drag-and-drop and paste handling.
 */
export function initMediaDrop(msgBox: HTMLElement, inputArea: HTMLElement): void {
	chatMsgBoxRef = msgBox;

	// Create preview strip above the input row (not inside it)
	previewStrip = document.createElement("div");
	previewStrip.className = "media-preview-strip hidden";
	if (inputArea?.parentElement) {
		inputArea.parentElement.insertBefore(previewStrip, inputArea);
	}

	// Bind drag-and-drop to messages area
	if (msgBox) {
		boundDragOver = onDragOver;
		boundDragEnter = onDragEnter;
		boundDragLeave = onDragLeave;
		boundDrop = onDrop;
		msgBox.addEventListener("dragover", boundDragOver);
		msgBox.addEventListener("dragenter", boundDragEnter);
		msgBox.addEventListener("dragleave", boundDragLeave);
		msgBox.addEventListener("drop", boundDrop);
	}

	// Bind paste to chat input
	if (S.chatInput) {
		boundPaste = onPaste;
		(S.chatInput as HTMLElement).addEventListener("paste", boundPaste as EventListener);
	}

	const attachBtn = document.getElementById("attachBtn") as HTMLButtonElement | null;
	const attachInput = document.getElementById("attachInput") as HTMLInputElement | null;
	if (attachBtn && attachInput) {
		boundAttachClick = (): void => attachInput.click();
		boundAttachChange = (e: Event): void => {
			const input = e.currentTarget as HTMLInputElement;
			if (input.files && input.files.length > 0) handleFiles(input.files);
			input.value = "";
		};
		attachBtn.addEventListener("click", boundAttachClick);
		attachInput.addEventListener("change", boundAttachChange);
	}
}

/** Remove all listeners and clean up. */
export function teardownMediaDrop(): void {
	if (chatMsgBoxRef) {
		if (boundDragOver) chatMsgBoxRef.removeEventListener("dragover", boundDragOver);
		if (boundDragEnter) chatMsgBoxRef.removeEventListener("dragenter", boundDragEnter);
		if (boundDragLeave) chatMsgBoxRef.removeEventListener("dragleave", boundDragLeave);
		if (boundDrop) chatMsgBoxRef.removeEventListener("drop", boundDrop);
	}
	if (S.chatInput && boundPaste) {
		(S.chatInput as HTMLElement).removeEventListener("paste", boundPaste as EventListener);
	}
	const attachBtn = document.getElementById("attachBtn");
	const attachInput = document.getElementById("attachInput");
	if (attachBtn && boundAttachClick) attachBtn.removeEventListener("click", boundAttachClick);
	if (attachInput && boundAttachChange) attachInput.removeEventListener("change", boundAttachChange);
	if (previewStrip?.parentElement) {
		previewStrip.parentElement.removeChild(previewStrip);
	}
	pendingAttachments = [];
	previewStrip = null;
	chatMsgBoxRef = null;
	boundDragOver = null;
	boundDragEnter = null;
	boundDragLeave = null;
	boundDrop = null;
	boundPaste = null;
	boundAttachClick = null;
	boundAttachChange = null;
	dragEnterCount = 0;
}

export function getPendingAttachments(): PendingAttachment[] {
	return pendingAttachments;
}

/** Clear pending attachments and hide preview strip. */
export function clearPendingAttachments(): void {
	pendingAttachments = [];
	renderPreview();
}

export function hasPendingAttachments(): boolean {
	return pendingAttachments.length > 0;
}

function safeHeaderFilename(name: string): string {
	const safe = name.replace(/[^A-Za-z0-9._-]/g, "").replace(/^\.+/, "");
	return safe || "upload";
}

export async function uploadDocumentAttachment(
	attachment: PendingAttachment,
	sessionKey: string,
): Promise<UploadedDocumentFile> {
	const resp = await fetch(`/api/sessions/${encodeURIComponent(sessionKey)}/upload`, {
		method: "POST",
		headers: {
			"Content-Type": attachment.mimeType,
			"X-Filename": safeHeaderFilename(attachment.name),
		},
		body: attachment.file,
	});
	const payload: unknown = await resp.json().catch(() => null);
	if (!isSuccessfulUploadPayload(resp, payload)) {
		throw new Error("File upload failed");
	}
	return {
		display_name: attachment.name,
		stored_filename: typeof payload.filename === "string" ? payload.filename : safeHeaderFilename(attachment.name),
		mime_type: typeof payload.contentType === "string" ? payload.contentType : attachment.mimeType,
		size_bytes: typeof payload.size === "number" ? payload.size : attachment.sizeBytes,
		url: typeof payload.url === "string" ? payload.url : undefined,
	};
}

interface UploadPayload {
	ok: true;
	filename?: unknown;
	contentType?: unknown;
	size?: unknown;
	url?: unknown;
}

function isSuccessfulUploadPayload(response: Response, payload: unknown): payload is UploadPayload {
	return response.ok && typeof payload === "object" && payload !== null && "ok" in payload && payload.ok === true;
}

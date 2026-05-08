// ── Media drag-and-drop + paste module ──────────────────────
// Handles drag-and-drop image upload, clipboard paste, and
// image preview strip above the chat input area.

import { t } from "./i18n";
import * as S from "./state";

interface PendingImage {
	dataUrl: string;
	file: File;
	name: string;
}

let pendingImages: PendingImage[] = [];
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

const ACCEPTED_TYPES: string[] = ["image/png", "image/jpeg", "image/gif", "image/webp"];
const MAX_FILE_SIZE = 20 * 1024 * 1024; // 20 MB

function isImageFile(file: File): boolean {
	return ACCEPTED_TYPES.indexOf(file.type) !== -1;
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

function addImage(dataUrl: string, file: File): void {
	pendingImages.push({ dataUrl: dataUrl, file: file, name: file.name });
	renderPreview();
}

function removeImage(index: number): void {
	pendingImages.splice(index, 1);
	renderPreview();
}

function renderPreview(): void {
	if (!previewStrip) return;
	previewStrip.textContent = "";

	if (pendingImages.length === 0) {
		previewStrip.classList.add("hidden");
		return;
	}

	previewStrip.classList.remove("hidden");

	for (let i = 0; i < pendingImages.length; i++) {
		const item = document.createElement("div");
		item.className = "media-preview-item";

		const img = document.createElement("img");
		img.className = "media-preview-thumb";
		img.src = pendingImages[i].dataUrl;
		img.alt = pendingImages[i].name;
		item.appendChild(img);

		const name = document.createElement("span");
		name.className = "media-preview-name";
		name.textContent = pendingImages[i].name;
		item.appendChild(name);

		const removeBtn = document.createElement("button");
		removeBtn.className = "media-preview-remove";
		removeBtn.textContent = "\u2715";
		removeBtn.title = t("common:actions.remove");
		removeBtn.dataset.idx = String(i);
		removeBtn.addEventListener("click", (e: MouseEvent): void => {
			const idx = Number.parseInt((e.currentTarget as HTMLButtonElement).dataset.idx!, 10);
			removeImage(idx);
		});
		item.appendChild(removeBtn);

		previewStrip.appendChild(item);
	}
}

async function handleFiles(files: FileList | File[]): Promise<void> {
	for (const file of files) {
		if (!isImageFile(file)) continue;
		if (file.size > MAX_FILE_SIZE) continue;
		try {
			const dataUrl = await readFileAsDataUrl(file);
			addImage(dataUrl, file);
		} catch (err) {
			console.warn("[media-drop] Failed to read file:", err);
		}
	}
}

function onDragOver(e: DragEvent): void {
	e.preventDefault();
	e.dataTransfer!.dropEffect = "copy";
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

	const imageFiles: File[] = [];
	for (const f of items) {
		if (isImageFile(f)) imageFiles.push(f);
	}
	if (imageFiles.length > 0) {
		e.preventDefault();
		handleFiles(imageFiles);
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
	pendingImages = [];
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

export function getPendingImages(): PendingImage[] {
	return pendingImages;
}

/** Clear pending images and hide preview strip. */
export function clearPendingImages(): void {
	pendingImages = [];
	renderPreview();
}

export function hasPendingImages(): boolean {
	return pendingImages.length > 0;
}

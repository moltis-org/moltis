// ── Media drag-and-drop + paste module ──────────────────────
// Handles drag-and-drop image upload, clipboard paste, and
// image preview strip above the chat input area.

import * as S from "./state.js";

var pendingImages = [];
var previewStrip = null;
var chatMsgBoxRef = null;

// Track bound handlers for teardown
var boundDragOver = null;
var boundDragEnter = null;
var boundDragLeave = null;
var boundDrop = null;
var boundPaste = null;

var ACCEPTED_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"];
var MAX_FILE_SIZE = 20 * 1024 * 1024; // 20 MB

function isImageFile(file) {
	return ACCEPTED_TYPES.indexOf(file.type) !== -1;
}

function readFileAsDataUrl(file) {
	return new Promise((resolve, reject) => {
		var reader = new FileReader();
		reader.onload = () => {
			resolve(reader.result);
		};
		reader.onerror = () => {
			reject(reader.error);
		};
		reader.readAsDataURL(file);
	});
}

function addImage(dataUrl, file) {
	pendingImages.push({ dataUrl: dataUrl, file: file, name: file.name });
	renderPreview();
}

function removeImage(index) {
	pendingImages.splice(index, 1);
	renderPreview();
}

function renderPreview() {
	if (!previewStrip) return;
	previewStrip.textContent = "";

	if (pendingImages.length === 0) {
		previewStrip.classList.add("hidden");
		return;
	}

	previewStrip.classList.remove("hidden");

	for (var i = 0; i < pendingImages.length; i++) {
		var item = document.createElement("div");
		item.className = "media-preview-item";

		var img = document.createElement("img");
		img.className = "media-preview-thumb";
		img.src = pendingImages[i].dataUrl;
		img.alt = pendingImages[i].name;
		item.appendChild(img);

		var name = document.createElement("span");
		name.className = "media-preview-name";
		name.textContent = pendingImages[i].name;
		item.appendChild(name);

		var removeBtn = document.createElement("button");
		removeBtn.className = "media-preview-remove";
		removeBtn.textContent = "\u2715";
		removeBtn.title = "Remove";
		removeBtn.dataset.idx = String(i);
		removeBtn.addEventListener("click", (e) => {
			var idx = Number.parseInt(e.currentTarget.dataset.idx, 10);
			removeImage(idx);
		});
		item.appendChild(removeBtn);

		previewStrip.appendChild(item);
	}
}

async function handleFiles(files) {
	for (var file of files) {
		if (!isImageFile(file)) continue;
		if (file.size > MAX_FILE_SIZE) continue;
		try {
			var dataUrl = await readFileAsDataUrl(file);
			addImage(dataUrl, file);
		} catch (err) {
			console.warn("[media-drop] Failed to read file:", err);
		}
	}
}

function onDragOver(e) {
	e.preventDefault();
	e.dataTransfer.dropEffect = "copy";
}

function onDragEnter(e) {
	e.preventDefault();
	if (chatMsgBoxRef) chatMsgBoxRef.classList.add("drag-over");
}

function onDragLeave(e) {
	// Only remove if leaving the container (not entering a child)
	if (chatMsgBoxRef && !chatMsgBoxRef.contains(e.relatedTarget)) {
		chatMsgBoxRef.classList.remove("drag-over");
	}
}

function onDrop(e) {
	e.preventDefault();
	if (chatMsgBoxRef) chatMsgBoxRef.classList.remove("drag-over");

	var files = e.dataTransfer.files;
	if (files.length > 0) {
		handleFiles(files);
	}
}

function onPaste(e) {
	var items = e.clipboardData?.files;
	if (!items || items.length === 0) return;

	var imageFiles = [];
	for (var f of items) {
		if (isImageFile(f)) imageFiles.push(f);
	}
	if (imageFiles.length > 0) {
		e.preventDefault();
		handleFiles(imageFiles);
	}
}

/**
 * Initialize drag-and-drop and paste handling.
 * @param {HTMLElement} msgBox - The chat messages container (drop target)
 * @param {HTMLElement} inputArea - The input area container (preview strip parent)
 */
export function initMediaDrop(msgBox, inputArea) {
	chatMsgBoxRef = msgBox;

	// Create preview strip
	previewStrip = document.createElement("div");
	previewStrip.className = "media-preview-strip hidden";
	if (inputArea) {
		inputArea.insertBefore(previewStrip, inputArea.firstChild);
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
		S.chatInput.addEventListener("paste", boundPaste);
	}
}

/** Remove all listeners and clean up. */
export function teardownMediaDrop() {
	if (chatMsgBoxRef) {
		if (boundDragOver) chatMsgBoxRef.removeEventListener("dragover", boundDragOver);
		if (boundDragEnter) chatMsgBoxRef.removeEventListener("dragenter", boundDragEnter);
		if (boundDragLeave) chatMsgBoxRef.removeEventListener("dragleave", boundDragLeave);
		if (boundDrop) chatMsgBoxRef.removeEventListener("drop", boundDrop);
	}
	if (S.chatInput && boundPaste) {
		S.chatInput.removeEventListener("paste", boundPaste);
	}
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
}

/** @returns {Array<{dataUrl: string, file: File, name: string}>} */
export function getPendingImages() {
	return pendingImages;
}

/** Clear pending images and hide preview strip. */
export function clearPendingImages() {
	pendingImages = [];
	renderPreview();
}

/** @returns {boolean} */
export function hasPendingImages() {
	return pendingImages.length > 0;
}

// ── Sandbox event handlers ───────────────────────────────────

import { chatAddMsg, smartScrollToBottom } from "../chat-ui";
import { currentPrefix } from "../router";
import * as S from "../state";
import type { LocalLlmDownloadPayload, SandboxPhasePayload } from "../types/ws-events";
import { clearChatEmptyState } from "./shared";

/** Subset of SandboxInfo relevant to the building flag. */
interface SandboxInfoState {
	image_building?: boolean;
	[key: string]: unknown;
}

function updateSandboxBuildingFlag(building: boolean): void {
	const info = S.sandboxInfo as SandboxInfoState | null;
	if (info) S.setSandboxInfo({ ...info, image_building: building });
}

let sandboxPrepareIndicatorEl: HTMLElement | null = null;
export function handleSandboxPrepare(payload: SandboxPhasePayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;

	if (payload.phase === "start") {
		if (sandboxPrepareIndicatorEl) {
			sandboxPrepareIndicatorEl.remove();
			sandboxPrepareIndicatorEl = null;
		}
		sandboxPrepareIndicatorEl = chatAddMsg(
			"system",
			"Preparing sandbox environment (first run may take a minute)\u2026",
		);
		return;
	}

	if (sandboxPrepareIndicatorEl) {
		sandboxPrepareIndicatorEl.remove();
		sandboxPrepareIndicatorEl = null;
	}

	if (payload.phase === "error") {
		chatAddMsg("error", `Sandbox setup failed: ${payload.error || "unknown"}`);
	}
}

export function handleSandboxImageBuild(payload: SandboxPhasePayload): void {
	const phase = payload.phase;
	// Update the sandboxInfo signal so all pages (chat, settings) reflect the build state.
	updateSandboxBuildingFlag(phase === "start");

	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	if (phase === "start") {
		chatAddMsg("system", "Building sandbox image (installing packages)\u2026");
	} else if (phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		const msg = payload.built ? `Sandbox image ready: ${payload.tag}` : `Sandbox image already cached: ${payload.tag}`;
		chatAddMsg("system", msg);
	} else if (phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Sandbox image build failed: ${payload.error || "unknown"}`);
	}
}

export function handleSandboxImageProvision(payload: SandboxPhasePayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	if (payload.phase === "start") {
		chatAddMsg("system", "Provisioning sandbox packages\u2026");
	} else if (payload.phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("system", "Sandbox packages provisioned");
	} else if (payload.phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Sandbox provisioning failed: ${payload.error || "unknown"}`);
	}
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Provisioning UI with multiple phases
export function handleSandboxHostProvision(payload: SandboxPhasePayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	if (payload.phase === "start") {
		const msg = `Installing ${payload.count || ""} package${payload.count === 1 ? "" : "s"} on host\u2026`;
		chatAddMsg("system", msg);
	} else if (payload.phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		const parts: string[] = [];
		if ((payload.installed || 0) > 0) parts.push(`${payload.installed} installed`);
		if ((payload.skipped || 0) > 0) parts.push(`${payload.skipped} already present`);
		chatAddMsg("system", `Host packages ready (${parts.join(", ") || "done"})`);
	} else if (payload.phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Host package install failed: ${payload.error || "unknown"}`);
	}
}

export function handleBrowserImagePull(payload: SandboxPhasePayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;
	const image = payload.image || "browser container";
	if (payload.phase === "start") {
		chatAddMsg("system", `Pulling browser container image (${image})\u2026 This may take a few minutes on first run.`);
	} else if (payload.phase === "done") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("system", `Browser container image ready: ${image}`);
	} else if (payload.phase === "error") {
		if (S.chatMsgBox?.lastChild) S.chatMsgBox.removeChild(S.chatMsgBox.lastChild);
		chatAddMsg("error", `Browser container image pull failed: ${payload.error || "unknown"}`);
	}
}

// Track download indicator element
let downloadIndicatorEl: HTMLElement | null = null;

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Download progress UI with multiple states
export function handleLocalLlmDownload(payload: LocalLlmDownloadPayload): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;

	const modelName = payload.displayName || payload.modelId || "model";

	if (payload.error) {
		// Download error
		if (downloadIndicatorEl) {
			downloadIndicatorEl.remove();
			downloadIndicatorEl = null;
		}
		chatAddMsg("error", `Failed to download ${modelName}: ${payload.error}`);
		return;
	}

	if (payload.complete) {
		// Download complete
		if (downloadIndicatorEl) {
			downloadIndicatorEl.remove();
			downloadIndicatorEl = null;
		}
		chatAddMsg("system", `${modelName} ready`);
		return;
	}

	// Download in progress - show/update progress indicator
	if (!downloadIndicatorEl) {
		downloadIndicatorEl = document.createElement("div");
		downloadIndicatorEl.className = "msg system download-indicator";

		const status = document.createElement("div");
		status.className = "download-status";
		status.textContent = `Downloading ${modelName}\u2026`;
		downloadIndicatorEl.appendChild(status);

		const progressContainer = document.createElement("div");
		progressContainer.className = "download-progress";
		const progressBar = document.createElement("div");
		progressBar.className = "download-progress-bar";
		progressContainer.appendChild(progressBar);
		downloadIndicatorEl.appendChild(progressContainer);

		const progressText = document.createElement("div");
		progressText.className = "download-progress-text";
		downloadIndicatorEl.appendChild(progressText);

		if (S.chatMsgBox) {
			clearChatEmptyState();
			S.chatMsgBox.appendChild(downloadIndicatorEl);
			smartScrollToBottom();
		}
	}

	// Update progress bar
	const barEl = downloadIndicatorEl.querySelector(".download-progress-bar") as HTMLElement | null;
	const textEl = downloadIndicatorEl.querySelector(".download-progress-text") as HTMLElement | null;
	const containerEl = downloadIndicatorEl.querySelector(".download-progress") as HTMLElement | null;

	if (barEl && containerEl) {
		if (payload.progress != null) {
			// Determinate progress - show actual percentage
			containerEl.classList.remove("indeterminate");
			barEl.style.width = `${payload.progress.toFixed(1)}%`;
		} else if (payload.total == null && payload.downloaded != null) {
			// Indeterminate progress - CSS handles the animation
			containerEl.classList.add("indeterminate");
			barEl.style.width = ""; // Let CSS control width
		}
	}

	if (payload.downloaded != null && textEl) {
		const downloadedMb = (payload.downloaded / (1024 * 1024)).toFixed(1);
		if (payload.total != null) {
			const totalMb = (payload.total / (1024 * 1024)).toFixed(1);
			textEl.textContent = `${downloadedMb} / ${totalMb} MB`;
		} else {
			textEl.textContent = `${downloadedMb} MB`;
		}
	}
}

// ── Sandbox event handlers ───────────────────────────────────

import { chatAddMsg, smartScrollToBottom } from "../chat-ui";
import { currentPrefix } from "../router";
import * as S from "../state";
import type { LocalLlmDownloadPayload, LocalLlmLifecyclePayload, SandboxPhasePayload } from "../types/ws-events";
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

let buildIndicatorEl: HTMLElement | null = null;
let buildTimerInterval: ReturnType<typeof setInterval> | null = null;
let buildStartTime = 0;

function clearBuildIndicator(): void {
	if (buildTimerInterval) {
		clearInterval(buildTimerInterval);
		buildTimerInterval = null;
	}
	if (buildIndicatorEl) {
		buildIndicatorEl.remove();
		buildIndicatorEl = null;
	}
}

function formatElapsed(ms: number): string {
	const secs = Math.floor(ms / 1000);
	const m = Math.floor(secs / 60);
	const s = secs % 60;
	return m > 0 ? `${m}m ${s}s` : `${s}s`;
}

export function handleSandboxImageBuild(payload: SandboxPhasePayload): void {
	const phase = payload.phase;
	// Update the sandboxInfo signal so all pages (chat, settings) reflect the build state.
	updateSandboxBuildingFlag(phase === "start");

	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;

	if (phase === "start") {
		clearBuildIndicator();
		buildStartTime = Date.now();

		buildIndicatorEl = document.createElement("div");
		buildIndicatorEl.className = "msg system download-indicator";

		const status = document.createElement("div");
		status.className = "download-status";
		const pkgCount = payload.package_count || 0;
		const pkgLabel = pkgCount > 0 ? ` (${pkgCount} packages)` : "";
		status.textContent = `Building sandbox image${pkgLabel}\u2026`;
		buildIndicatorEl.appendChild(status);

		const progressContainer = document.createElement("div");
		progressContainer.className = "download-progress indeterminate";
		const progressBar = document.createElement("div");
		progressBar.className = "download-progress-bar";
		progressContainer.appendChild(progressBar);
		buildIndicatorEl.appendChild(progressContainer);

		const progressText = document.createElement("div");
		progressText.className = "download-progress-text";
		progressText.textContent = "First run — usually takes 3\u20135 minutes";
		buildIndicatorEl.appendChild(progressText);

		if (S.chatMsgBox) {
			clearChatEmptyState();
			S.chatMsgBox.appendChild(buildIndicatorEl);
			smartScrollToBottom();
		}

		// Update elapsed time every second.
		buildTimerInterval = setInterval(() => {
			const textEl = buildIndicatorEl?.querySelector(".download-progress-text");
			if (textEl) {
				textEl.textContent = `Elapsed: ${formatElapsed(Date.now() - buildStartTime)}`;
			}
		}, 1000);
	} else if (phase === "done") {
		clearBuildIndicator();
		if (!payload.built) {
			// Image was already cached — no need to tell the user about it.
			return;
		}
		chatAddMsg("system", "Sandbox image ready");
	} else if (phase === "error") {
		clearBuildIndicator();
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

// ── Local LLM lifecycle handler ──────────────────────────────

let lifecycleIndicatorEl: HTMLElement | null = null;

export function handleLocalLlmLifecycle(raw: Record<string, unknown>): void {
	const isChatPage = currentPrefix === "/chats";
	if (!isChatPage) return;

	const payload = raw as unknown as LocalLlmLifecyclePayload;
	const modelName = payload.modelId || "model";

	if (payload.state === "loading") {
		if (lifecycleIndicatorEl) {
			lifecycleIndicatorEl.remove();
		}
		lifecycleIndicatorEl = chatAddMsg("system", `Loading model ${modelName} into memory\u2026`);
	} else if (payload.state === "loaded") {
		if (lifecycleIndicatorEl) {
			lifecycleIndicatorEl.remove();
			lifecycleIndicatorEl = null;
		}
		const sizeStr = payload.modelSizeBytes ? ` (${(payload.modelSizeBytes / (1024 * 1024 * 1024)).toFixed(1)} GB)` : "";
		chatAddMsg("system", `Model ${modelName} loaded${sizeStr}`);
	} else if (payload.state === "unloaded") {
		const msg =
			payload.reason === "idle" ? `Model ${modelName} unloaded after inactivity` : `Model ${modelName} unloaded`;
		chatAddMsg("system", msg);
	}
	// "unloading" state is transient — no need to show in chat
}

// ── Sandbox toggle + image selector ─────────────────────────

import { updateCommandInputUI, updateTokenBar } from "./chat-ui";
import { sendRpc } from "./helpers";
import { t } from "./i18n";
import * as S from "./state";

interface SandboxInfoRecord {
	backend?: string;
}

interface SessionPatchResult {
	result?: {
		sandbox_enabled?: boolean;
		sandbox_image?: string;
	};
}

const SANDBOX_DISABLED_HINT = (): string => t("chat:sandboxDisabledHint");

function sandboxRuntimeAvailable(): boolean {
	const info = S.sandboxInfo as SandboxInfoRecord | null;
	return (info?.backend || "none") !== "none";
}

/** Truncate long hash suffixes: "repo:abcdef...uvwxyz" */
function truncateHash(str: string): string {
	const idx = str.lastIndexOf(":");
	if (idx !== -1) {
		const suffix = str.slice(idx + 1);
		if (suffix.length > 12) {
			return `${str.slice(0, idx + 1) + suffix.slice(0, 6)}\u2026${suffix.slice(-6)}`;
		}
	}
	if (str.length > 24 && str.indexOf(":") === -1) {
		return `${str.slice(0, 6)}\u2026${str.slice(-6)}`;
	}
	return str;
}

/** Apply disabled/enabled styling to a button element. */
function applyButtonAvailability(
	btn: HTMLButtonElement,
	available: boolean,
	enabledTitle: string,
	disabledTitle: string,
): void {
	btn.disabled = !available;
	btn.style.opacity = available ? "" : "0.55";
	btn.style.cursor = available ? "pointer" : "not-allowed";
	btn.title = available ? enabledTitle : disabledTitle;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: UI state management with multiple controls
function applySandboxControlAvailability(): boolean {
	const available = sandboxRuntimeAvailable();
	const hint = available ? "" : SANDBOX_DISABLED_HINT();

	const toggleBtn = S.sandboxToggleBtn;
	if (toggleBtn) {
		applyButtonAvailability(toggleBtn, available, t("chat:sandboxToggleTooltip"), hint);
	}

	const imageBtn = S.sandboxImageBtn;
	if (imageBtn) {
		applyButtonAvailability(imageBtn, available, t("chat:sandboxImageTooltip"), hint);
	}

	const dropdown = S.sandboxImageDropdown;
	if (!available && dropdown) {
		dropdown.classList.add("hidden");
	}

	return available;
}

// ── Sandbox enabled/disabled toggle ─────────────────────────

export function updateSandboxUI(enabled: boolean): void {
	S.setSessionSandboxEnabled(!!enabled);
	const effectiveSandboxRoute = !!enabled && sandboxRuntimeAvailable();
	S.setSessionExecMode(effectiveSandboxRoute ? "sandbox" : "host");
	S.setSessionExecPromptSymbol(effectiveSandboxRoute || S.hostExecIsRoot ? "#" : "$");
	updateCommandInputUI();
	updateTokenBar();
	const label = S.sandboxLabel;
	const toggleBtn = S.sandboxToggleBtn;
	if (!(label && toggleBtn)) return;
	if (!applySandboxControlAvailability()) {
		label.textContent = t("chat:sandboxDisabled");
		toggleBtn.style.borderColor = "";
		toggleBtn.style.color = "var(--muted)";
		return;
	}
	if (S.sessionSandboxEnabled) {
		label.textContent = t("chat:sandboxed");
		toggleBtn.style.borderColor = "var(--accent, #f59e0b)";
		toggleBtn.style.color = "var(--accent, #f59e0b)";
	} else {
		label.textContent = t("chat:sandboxDirect");
		toggleBtn.style.borderColor = "";
		toggleBtn.style.color = "var(--muted)";
	}
}

export function bindSandboxToggleEvents(): void {
	const toggleBtn = S.sandboxToggleBtn;
	if (!toggleBtn) return;
	toggleBtn.addEventListener("click", () => {
		if (!sandboxRuntimeAvailable()) return;
		const newVal = !S.sessionSandboxEnabled;
		sendRpc<SessionPatchResult>("sessions.patch", {
			key: S.activeSessionKey,
			sandboxEnabled: newVal,
		}).then((res) => {
			if (res?.payload?.result) {
				updateSandboxUI(res.payload.result.sandbox_enabled as boolean);
			} else {
				updateSandboxUI(newVal);
			}
		});
	});
}

// ── Sandbox image selector ──────────────────────────────────

const DEFAULT_IMAGE = "ubuntu:25.10";
let sandboxImageBtnEl: HTMLButtonElement | null = null;
let sandboxImageBtnClickHandler: ((e: MouseEvent) => void) | null = null;
let sandboxImageDocClickHandler: (() => void) | null = null;
let sandboxImageRepositionHandler: (() => void) | null = null;

export function updateSandboxImageUI(image: string | null): void {
	S.setSessionSandboxImage(image || null);
	const imageLabel = S.sandboxImageLabel;
	if (!imageLabel) return;
	if (!applySandboxControlAvailability()) {
		imageLabel.textContent = t("chat:sandboxUnavailable");
		return;
	}
	imageLabel.textContent = truncateHash(image || DEFAULT_IMAGE);
}

export function bindSandboxImageEvents(): void {
	const imageBtn = S.sandboxImageBtn;
	if (!imageBtn) return;
	if (sandboxImageBtnEl && sandboxImageBtnClickHandler) {
		sandboxImageBtnEl.removeEventListener("click", sandboxImageBtnClickHandler);
	}
	if (sandboxImageDocClickHandler) {
		document.removeEventListener("click", sandboxImageDocClickHandler);
	}
	if (sandboxImageRepositionHandler) {
		window.removeEventListener("resize", sandboxImageRepositionHandler);
		document.removeEventListener("scroll", sandboxImageRepositionHandler, true);
	}

	sandboxImageBtnClickHandler = (e: MouseEvent): void => {
		if (!sandboxRuntimeAvailable()) return;
		e.stopPropagation();
		toggleImageDropdown();
	};
	sandboxImageDocClickHandler = (): void => {
		const dropdown = S.sandboxImageDropdown;
		if (dropdown) {
			dropdown.classList.add("hidden");
		}
	};
	sandboxImageRepositionHandler = (): void => positionImageDropdown();

	sandboxImageBtnEl = imageBtn;
	sandboxImageBtnEl.addEventListener("click", sandboxImageBtnClickHandler);
	document.addEventListener("click", sandboxImageDocClickHandler);

	window.addEventListener("resize", sandboxImageRepositionHandler);
	document.addEventListener("scroll", sandboxImageRepositionHandler, true);
}

function toggleImageDropdown(): void {
	const dropdown = S.sandboxImageDropdown;
	if (!(dropdown && S.sandboxImageBtn)) return;
	const isHidden = dropdown.classList.contains("hidden");
	if (isHidden) {
		populateImageDropdown();
		dropdown.classList.remove("hidden");
		requestAnimationFrame(positionImageDropdown);
	} else {
		dropdown.classList.add("hidden");
	}
}

function positionImageDropdown(): void {
	const dropdown = S.sandboxImageDropdown;
	const btn = S.sandboxImageBtn;
	if (!(dropdown && btn)) return;
	if (dropdown.classList.contains("hidden")) return;

	const btnRect = btn.getBoundingClientRect();
	const viewportWidth = window.innerWidth || document.documentElement.clientWidth || 0;
	const viewportHeight = window.innerHeight || document.documentElement.clientHeight || 0;

	dropdown.style.position = "fixed";
	dropdown.style.zIndex = "70";
	dropdown.style.marginTop = "0";
	dropdown.style.minWidth = `${Math.max(200, Math.round(btnRect.width))}px`;
	dropdown.style.maxWidth = `${Math.max(220, viewportWidth - 16)}px`;

	const preferredTop = btnRect.bottom + 4;
	dropdown.style.top = `${preferredTop}px`;
	dropdown.style.left = `${Math.max(8, Math.round(btnRect.left))}px`;

	// Measure after placement so we can clamp to viewport and optionally open upward.
	let dropdownRect = dropdown.getBoundingClientRect();
	const spaceBelow = viewportHeight - btnRect.bottom - 8;
	const spaceAbove = btnRect.top - 8;
	const shouldOpenUp = spaceBelow < 180 && spaceAbove > spaceBelow;
	const maxHeight = Math.max(120, shouldOpenUp ? spaceAbove : spaceBelow);
	dropdown.style.maxHeight = `${Math.floor(maxHeight)}px`;

	if (shouldOpenUp) {
		const desiredTop = btnRect.top - Math.min(dropdownRect.height, maxHeight) - 4;
		dropdown.style.top = `${Math.max(8, Math.round(desiredTop))}px`;
	}

	dropdownRect = dropdown.getBoundingClientRect();
	const clampedLeft = Math.max(
		8,
		Math.min(Math.round(btnRect.left), Math.round(viewportWidth - dropdownRect.width - 8)),
	);
	dropdown.style.left = `${clampedLeft}px`;
}

function populateImageDropdown(): void {
	const dropdown = S.sandboxImageDropdown;
	if (!dropdown) return;
	dropdown.textContent = "";

	// Fetch available backends and images in parallel.
	interface AvailableBackend {
		id: string;
		label: string;
		kind: string;
	}
	interface BackendsResponse {
		backends?: AvailableBackend[];
		default?: string;
	}
	interface CachedImage {
		tag: string;
		skill_name?: string;
		size?: string;
	}
	interface CachedImagesResponse {
		images?: CachedImage[];
	}

	Promise.all([
		fetch("/api/sandbox/available-backends")
			.then((r) => r.json())
			.catch(() => ({ backends: [] })),
		fetch("/api/images/cached")
			.then((r) => r.json())
			.catch(() => ({ images: [] })),
	]).then(([backendsData, imagesData]: [BackendsResponse, CachedImagesResponse]) => {
		const backends = backendsData.backends || [];
		const images = imagesData.images || [];

		// Backend section header.
		if (backends.length > 0) {
			const header = document.createElement("div");
			header.className = "px-3 py-1 text-[10px] font-medium text-[var(--muted)] uppercase tracking-wider";
			header.textContent = "Backend";
			dropdown.appendChild(header);

			for (const b of backends) {
				const isCurrent = S.sessionSandboxBackend === b.id;
				const opt = document.createElement("div");
				opt.className =
					"px-3 py-1.5 text-xs cursor-pointer hover:bg-[var(--surface2)] transition-colors flex items-center gap-2";
				if (isCurrent) {
					opt.style.color = "var(--accent, #f59e0b)";
					opt.style.fontWeight = "600";
				}
				const kindBadge = b.kind === "remote" ? " \u2601" : "";
				opt.textContent = `${b.label}${kindBadge}`;
				opt.addEventListener("click", (e: MouseEvent): void => {
					e.stopPropagation();
					selectBackend(b.id);
				});
				dropdown.appendChild(opt);
			}
		}

		// Image section (only relevant for container backends).
		const divider = document.createElement("div");
		divider.className = "border-t border-[var(--border)] my-1";
		dropdown.appendChild(divider);

		const imgHeader = document.createElement("div");
		imgHeader.className = "px-3 py-1 text-[10px] font-medium text-[var(--muted)] uppercase tracking-wider";
		imgHeader.textContent = "Image";
		dropdown.appendChild(imgHeader);

		addImageOption(dropdown, DEFAULT_IMAGE, !S.sessionSandboxImage);
		for (const img of images) {
			const isCurrent = S.sessionSandboxImage === img.tag;
			addImageOption(dropdown, img.tag, isCurrent, `${img.skill_name} (${img.size})`);
		}

		requestAnimationFrame(positionImageDropdown);
	});
}

function selectBackend(backendId: string): void {
	sendRpc<SessionPatchResult>("sessions.patch", {
		key: S.activeSessionKey,
		sandboxBackend: backendId,
	});
	S.setSessionSandboxBackend(backendId);
	const dropdown = S.sandboxImageDropdown;
	if (dropdown) {
		dropdown.classList.add("hidden");
	}
}

function addImageOption(dropdown: HTMLElement, tag: string, isActive: boolean, subtitle?: string): void {
	const opt = document.createElement("div");
	opt.className = "px-3 py-2 text-xs cursor-pointer hover:bg-[var(--surface2)] transition-colors";
	if (isActive) {
		opt.style.color = "var(--accent, #f59e0b)";
		opt.style.fontWeight = "600";
	}

	const label = document.createElement("div");
	label.textContent = truncateHash(tag);
	label.title = tag;
	opt.appendChild(label);

	if (subtitle) {
		const sub = document.createElement("div");
		sub.textContent = subtitle;
		sub.style.color = "var(--muted)";
		sub.style.fontSize = "0.65rem";
		opt.appendChild(sub);
	}

	opt.addEventListener("click", (e: MouseEvent): void => {
		e.stopPropagation();
		selectImage(tag === DEFAULT_IMAGE ? null : tag);
	});

	dropdown.appendChild(opt);
}

function selectImage(tag: string | null): void {
	const value = tag || "";
	sendRpc<SessionPatchResult>("sessions.patch", {
		key: S.activeSessionKey,
		sandboxImage: value,
	}).then((res) => {
		if (res?.payload?.result) {
			updateSandboxImageUI(res.payload.result.sandbox_image as string);
		} else {
			updateSandboxImageUI(tag);
		}
	});
	const dropdown = S.sandboxImageDropdown;
	if (dropdown) {
		dropdown.classList.add("hidden");
	}
}

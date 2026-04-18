// ── Sandbox toggle + image selector ─────────────────────────

import { updateCommandInputUI, updateTokenBar } from "./chat-ui";
import { sendRpc } from "./helpers";
import { t } from "./i18n";
import * as S from "./state";

const SANDBOX_DISABLED_HINT = (): string => t("chat:sandboxDisabledHint");

function sandboxRuntimeAvailable(): boolean {
	return ((S.sandboxInfo as Record<string, unknown> | null)?.backend || "none") !== "none";
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

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: UI state management with multiple controls
function applySandboxControlAvailability(): boolean {
	const available = sandboxRuntimeAvailable();
	const title = available ? null : SANDBOX_DISABLED_HINT();

	if (S.sandboxToggleBtn) {
		(S.sandboxToggleBtn as HTMLButtonElement).disabled = !available;
		(S.sandboxToggleBtn as HTMLElement).style.opacity = available ? "" : "0.55";
		(S.sandboxToggleBtn as HTMLElement).style.cursor = available ? "pointer" : "not-allowed";
		if (title) {
			(S.sandboxToggleBtn as HTMLElement).title = title;
		} else {
			(S.sandboxToggleBtn as HTMLElement).title = t("chat:sandboxToggleTooltip");
		}
	}

	if (S.sandboxImageBtn) {
		(S.sandboxImageBtn as HTMLButtonElement).disabled = !available;
		(S.sandboxImageBtn as HTMLElement).style.opacity = available ? "" : "0.55";
		(S.sandboxImageBtn as HTMLElement).style.cursor = available ? "pointer" : "not-allowed";
		if (title) {
			(S.sandboxImageBtn as HTMLElement).title = title;
		} else {
			(S.sandboxImageBtn as HTMLElement).title = t("chat:sandboxImageTooltip");
		}
	}

	if (!available && S.sandboxImageDropdown) {
		(S.sandboxImageDropdown as HTMLElement).classList.add("hidden");
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
	if (!(S.sandboxLabel && S.sandboxToggleBtn)) return;
	if (!applySandboxControlAvailability()) {
		(S.sandboxLabel as HTMLElement).textContent = t("chat:sandboxDisabled");
		(S.sandboxToggleBtn as HTMLElement).style.borderColor = "";
		(S.sandboxToggleBtn as HTMLElement).style.color = "var(--muted)";
		return;
	}
	if (S.sessionSandboxEnabled) {
		(S.sandboxLabel as HTMLElement).textContent = t("chat:sandboxed");
		(S.sandboxToggleBtn as HTMLElement).style.borderColor = "var(--accent, #f59e0b)";
		(S.sandboxToggleBtn as HTMLElement).style.color = "var(--accent, #f59e0b)";
	} else {
		(S.sandboxLabel as HTMLElement).textContent = t("chat:sandboxDirect");
		(S.sandboxToggleBtn as HTMLElement).style.borderColor = "";
		(S.sandboxToggleBtn as HTMLElement).style.color = "var(--muted)";
	}
}

export function bindSandboxToggleEvents(): void {
	if (!S.sandboxToggleBtn) return;
	(S.sandboxToggleBtn as HTMLElement).addEventListener("click", () => {
		if (!sandboxRuntimeAvailable()) return;
		const newVal = !S.sessionSandboxEnabled;
		sendRpc("sessions.patch", {
			key: S.activeSessionKey,
			sandboxEnabled: newVal,
		}).then((res) => {
			const payload = res as unknown as Record<string, unknown>;
			if (payload?.result) {
				updateSandboxUI((payload.result as Record<string, unknown>).sandbox_enabled as boolean);
			} else {
				updateSandboxUI(newVal);
			}
		});
	});
}

// ── Sandbox image selector ──────────────────────────────────

const DEFAULT_IMAGE = "ubuntu:25.10";
let sandboxImageBtnEl: HTMLElement | null = null;
let sandboxImageBtnClickHandler: ((e: MouseEvent) => void) | null = null;
let sandboxImageDocClickHandler: (() => void) | null = null;
let sandboxImageRepositionHandler: (() => void) | null = null;

export function updateSandboxImageUI(image: string | null): void {
	S.setSessionSandboxImage(image || null);
	if (!S.sandboxImageLabel) return;
	if (!applySandboxControlAvailability()) {
		(S.sandboxImageLabel as HTMLElement).textContent = t("chat:sandboxUnavailable");
		return;
	}
	(S.sandboxImageLabel as HTMLElement).textContent = truncateHash(image || DEFAULT_IMAGE);
}

export function bindSandboxImageEvents(): void {
	if (!S.sandboxImageBtn) return;
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
		if (S.sandboxImageDropdown) {
			(S.sandboxImageDropdown as HTMLElement).classList.add("hidden");
		}
	};
	sandboxImageRepositionHandler = (): void => positionImageDropdown();

	sandboxImageBtnEl = S.sandboxImageBtn as HTMLElement;
	sandboxImageBtnEl.addEventListener("click", sandboxImageBtnClickHandler);
	document.addEventListener("click", sandboxImageDocClickHandler);

	window.addEventListener("resize", sandboxImageRepositionHandler);
	document.addEventListener("scroll", sandboxImageRepositionHandler, true);
}

function toggleImageDropdown(): void {
	if (!(S.sandboxImageDropdown && S.sandboxImageBtn)) return;
	const isHidden = (S.sandboxImageDropdown as HTMLElement).classList.contains("hidden");
	if (isHidden) {
		populateImageDropdown();
		(S.sandboxImageDropdown as HTMLElement).classList.remove("hidden");
		requestAnimationFrame(positionImageDropdown);
	} else {
		(S.sandboxImageDropdown as HTMLElement).classList.add("hidden");
	}
}

function positionImageDropdown(): void {
	if (!(S.sandboxImageDropdown && S.sandboxImageBtn)) return;
	const dropdown = S.sandboxImageDropdown as HTMLElement;
	const btn = S.sandboxImageBtn as HTMLElement;
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
	const clampedLeft = Math.max(8, Math.min(Math.round(btnRect.left), Math.round(viewportWidth - dropdownRect.width - 8)));
	dropdown.style.left = `${clampedLeft}px`;
}

function populateImageDropdown(): void {
	if (!S.sandboxImageDropdown) return;
	const dropdown = S.sandboxImageDropdown as HTMLElement;
	dropdown.textContent = "";

	// Default option
	addImageOption(DEFAULT_IMAGE, !S.sessionSandboxImage);

	// Fetch cached images
	fetch("/api/images/cached")
		.then((r) => r.json())
		.then((data: Record<string, unknown>) => {
			const images = (data.images || []) as Array<Record<string, unknown>>;
			for (const img of images) {
				const isCurrent = S.sessionSandboxImage === img.tag;
				addImageOption(img.tag as string, isCurrent, `${img.skill_name} (${img.size})`);
			}
			requestAnimationFrame(positionImageDropdown);
		})
		.catch(() => {
			// Silently ignore fetch errors for image list
		});
}

function addImageOption(tag: string, isActive: boolean, subtitle?: string): void {
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

	(S.sandboxImageDropdown as HTMLElement).appendChild(opt);
}

function selectImage(tag: string | null): void {
	const value = tag || "";
	sendRpc("sessions.patch", {
		key: S.activeSessionKey,
		sandboxImage: value,
	}).then((res) => {
		const payload = res as unknown as Record<string, unknown>;
		if (payload?.result) {
			updateSandboxImageUI((payload.result as Record<string, unknown>).sandbox_image as string);
		} else {
			updateSandboxImageUI(tag);
		}
	});
	if (S.sandboxImageDropdown) {
		(S.sandboxImageDropdown as HTMLElement).classList.add("hidden");
	}
}

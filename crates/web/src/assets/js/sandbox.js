// ── Sandbox toggle + image selector ─────────────────────────

import { updateCommandInputUI, updateTokenBar } from "./chat-ui.js";
import { sendRpc } from "./helpers.js";
import { t } from "./i18n.js";
import * as S from "./state.js";

var SANDBOX_DISABLED_HINT = () => t("chat:sandboxDisabledHint");

function sandboxRuntimeAvailable() {
	return (S.sandboxInfo?.backend || "none") !== "none";
}

/** Truncate long hash suffixes: "repo:abcdef…uvwxyz" */
function truncateHash(str) {
	var idx = str.lastIndexOf(":");
	if (idx !== -1) {
		var suffix = str.slice(idx + 1);
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
function applySandboxControlAvailability() {
	var available = sandboxRuntimeAvailable();
	var title = available ? null : SANDBOX_DISABLED_HINT();

	if (S.sandboxToggleBtn) {
		S.sandboxToggleBtn.disabled = !available;
		S.sandboxToggleBtn.style.opacity = available ? "" : "0.55";
		S.sandboxToggleBtn.style.cursor = available ? "pointer" : "not-allowed";
		if (title) {
			S.sandboxToggleBtn.title = title;
		} else {
			S.sandboxToggleBtn.title = t("chat:sandboxToggleTooltip");
		}
	}

	if (S.sandboxImageBtn) {
		S.sandboxImageBtn.disabled = !available;
		S.sandboxImageBtn.style.opacity = available ? "" : "0.55";
		S.sandboxImageBtn.style.cursor = available ? "pointer" : "not-allowed";
		if (title) {
			S.sandboxImageBtn.title = title;
		} else {
			S.sandboxImageBtn.title = t("chat:sandboxImageTooltip");
		}
	}

	if (!available && S.sandboxImageDropdown) {
		S.sandboxImageDropdown.classList.add("hidden");
	}

	return available;
}

// ── Sandbox enabled/disabled toggle ─────────────────────────

export function updateSandboxUI(enabled) {
	S.setSessionSandboxEnabled(!!enabled);
	var effectiveSandboxRoute = !!enabled && sandboxRuntimeAvailable();
	S.setSessionExecMode(effectiveSandboxRoute ? "sandbox" : "host");
	S.setSessionExecPromptSymbol(effectiveSandboxRoute || S.hostExecIsRoot ? "#" : "$");
	updateCommandInputUI();
	updateTokenBar();
	if (!(S.sandboxLabel && S.sandboxToggleBtn)) return;
	if (!applySandboxControlAvailability()) {
		S.sandboxLabel.textContent = t("chat:sandboxDisabled");
		S.sandboxToggleBtn.style.borderColor = "";
		S.sandboxToggleBtn.style.color = "var(--muted)";
		return;
	}
	if (S.sessionSandboxEnabled) {
		S.sandboxLabel.textContent = t("chat:sandboxed");
		S.sandboxToggleBtn.style.borderColor = "var(--accent, #f59e0b)";
		S.sandboxToggleBtn.style.color = "var(--accent, #f59e0b)";
	} else {
		S.sandboxLabel.textContent = t("chat:sandboxDirect");
		S.sandboxToggleBtn.style.borderColor = "";
		S.sandboxToggleBtn.style.color = "var(--muted)";
	}
}

export function bindSandboxToggleEvents() {
	if (!S.sandboxToggleBtn) return;
	S.sandboxToggleBtn.addEventListener("click", () => {
		if (!sandboxRuntimeAvailable()) return;
		var newVal = !S.sessionSandboxEnabled;
		sendRpc("sessions.patch", {
			key: S.activeSessionKey,
			sandboxEnabled: newVal,
		}).then((res) => {
			if (res?.result) {
				updateSandboxUI(res.result.sandbox_enabled);
			} else {
				updateSandboxUI(newVal);
			}
		});
	});
}

// ── Sandbox image selector ──────────────────────────────────

var DEFAULT_IMAGE = "ubuntu:25.10";
var sandboxImageBtnEl = null;
var sandboxImageBtnClickHandler = null;
var sandboxImageDocClickHandler = null;
var sandboxImageRepositionHandler = null;

export function updateSandboxImageUI(image) {
	S.setSessionSandboxImage(image || null);
	if (!S.sandboxImageLabel) return;
	if (!applySandboxControlAvailability()) {
		S.sandboxImageLabel.textContent = t("chat:sandboxUnavailable");
		return;
	}
	S.sandboxImageLabel.textContent = truncateHash(image || DEFAULT_IMAGE);
}

export function bindSandboxImageEvents() {
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

	sandboxImageBtnClickHandler = (e) => {
		if (!sandboxRuntimeAvailable()) return;
		e.stopPropagation();
		toggleImageDropdown();
	};
	sandboxImageDocClickHandler = () => {
		if (S.sandboxImageDropdown) {
			S.sandboxImageDropdown.classList.add("hidden");
		}
	};
	sandboxImageRepositionHandler = () => positionImageDropdown();

	sandboxImageBtnEl = S.sandboxImageBtn;
	sandboxImageBtnEl.addEventListener("click", sandboxImageBtnClickHandler);
	document.addEventListener("click", sandboxImageDocClickHandler);

	window.addEventListener("resize", sandboxImageRepositionHandler);
	document.addEventListener("scroll", sandboxImageRepositionHandler, true);
}

function toggleImageDropdown() {
	if (!(S.sandboxImageDropdown && S.sandboxImageBtn)) return;
	var isHidden = S.sandboxImageDropdown.classList.contains("hidden");
	if (isHidden) {
		populateImageDropdown();
		S.sandboxImageDropdown.classList.remove("hidden");
		requestAnimationFrame(positionImageDropdown);
	} else {
		S.sandboxImageDropdown.classList.add("hidden");
	}
}

function positionImageDropdown() {
	if (!(S.sandboxImageDropdown && S.sandboxImageBtn)) return;
	if (S.sandboxImageDropdown.classList.contains("hidden")) return;

	var btnRect = S.sandboxImageBtn.getBoundingClientRect();
	var viewportWidth = window.innerWidth || document.documentElement.clientWidth || 0;
	var viewportHeight = window.innerHeight || document.documentElement.clientHeight || 0;

	S.sandboxImageDropdown.style.position = "fixed";
	S.sandboxImageDropdown.style.zIndex = "70";
	S.sandboxImageDropdown.style.marginTop = "0";
	S.sandboxImageDropdown.style.minWidth = `${Math.max(200, Math.round(btnRect.width))}px`;
	S.sandboxImageDropdown.style.maxWidth = `${Math.max(220, viewportWidth - 16)}px`;

	var preferredTop = btnRect.bottom + 4;
	S.sandboxImageDropdown.style.top = `${preferredTop}px`;
	S.sandboxImageDropdown.style.left = `${Math.max(8, Math.round(btnRect.left))}px`;

	// Measure after placement so we can clamp to viewport and optionally open upward.
	var dropdownRect = S.sandboxImageDropdown.getBoundingClientRect();
	var spaceBelow = viewportHeight - btnRect.bottom - 8;
	var spaceAbove = btnRect.top - 8;
	var shouldOpenUp = spaceBelow < 180 && spaceAbove > spaceBelow;
	var maxHeight = Math.max(120, shouldOpenUp ? spaceAbove : spaceBelow);
	S.sandboxImageDropdown.style.maxHeight = `${Math.floor(maxHeight)}px`;

	if (shouldOpenUp) {
		var desiredTop = btnRect.top - Math.min(dropdownRect.height, maxHeight) - 4;
		S.sandboxImageDropdown.style.top = `${Math.max(8, Math.round(desiredTop))}px`;
	}

	dropdownRect = S.sandboxImageDropdown.getBoundingClientRect();
	var clampedLeft = Math.max(8, Math.min(Math.round(btnRect.left), Math.round(viewportWidth - dropdownRect.width - 8)));
	S.sandboxImageDropdown.style.left = `${clampedLeft}px`;
}

function populateImageDropdown() {
	if (!S.sandboxImageDropdown) return;
	S.sandboxImageDropdown.textContent = "";

	// Default option
	addImageOption(DEFAULT_IMAGE, !S.sessionSandboxImage);

	// Fetch cached images
	fetch("/api/images/cached")
		.then((r) => r.json())
		.then((data) => {
			var images = data.images || [];
			for (var img of images) {
				var isCurrent = S.sessionSandboxImage === img.tag;
				addImageOption(img.tag, isCurrent, `${img.skill_name} (${img.size})`);
			}
			requestAnimationFrame(positionImageDropdown);
		})
		.catch(() => {
			// Silently ignore fetch errors for image list
		});
}

function addImageOption(tag, isActive, subtitle) {
	var opt = document.createElement("div");
	opt.className = "px-3 py-2 text-xs cursor-pointer hover:bg-[var(--surface2)] transition-colors";
	if (isActive) {
		opt.style.color = "var(--accent, #f59e0b)";
		opt.style.fontWeight = "600";
	}

	var label = document.createElement("div");
	label.textContent = truncateHash(tag);
	label.title = tag;
	opt.appendChild(label);

	if (subtitle) {
		var sub = document.createElement("div");
		sub.textContent = subtitle;
		sub.style.color = "var(--muted)";
		sub.style.fontSize = "0.65rem";
		opt.appendChild(sub);
	}

	opt.addEventListener("click", (e) => {
		e.stopPropagation();
		selectImage(tag === DEFAULT_IMAGE ? null : tag);
	});

	S.sandboxImageDropdown.appendChild(opt);
}

function selectImage(tag) {
	var value = tag || "";
	sendRpc("sessions.patch", {
		key: S.activeSessionKey,
		sandboxImage: value,
	}).then((res) => {
		if (res?.result) {
			updateSandboxImageUI(res.result.sandbox_image);
		} else {
			updateSandboxImageUI(tag);
		}
	});
	if (S.sandboxImageDropdown) {
		S.sandboxImageDropdown.classList.add("hidden");
	}
}

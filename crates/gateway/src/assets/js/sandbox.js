// ── Sandbox toggle + image selector ─────────────────────────

import { sendRpc } from "./helpers.js";
import * as S from "./state.js";

var SANDBOX_DISABLED_HINT =
	"Sandboxes are disabled on cloud deploys without a container runtime. Install on a VM with Docker or Apple Container to enable this feature.";

function sandboxRuntimeAvailable() {
	return (S.sandboxInfo?.backend || "none") !== "none";
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: UI state management with multiple controls
function applySandboxControlAvailability() {
	var available = sandboxRuntimeAvailable();
	var title = available ? null : SANDBOX_DISABLED_HINT;

	if (S.sandboxToggleBtn) {
		S.sandboxToggleBtn.disabled = !available;
		S.sandboxToggleBtn.style.opacity = available ? "" : "0.55";
		S.sandboxToggleBtn.style.cursor = available ? "pointer" : "not-allowed";
		if (title) {
			S.sandboxToggleBtn.title = title;
		} else {
			S.sandboxToggleBtn.title = "Toggle sandbox mode";
		}
	}

	if (S.sandboxImageBtn) {
		S.sandboxImageBtn.disabled = !available;
		S.sandboxImageBtn.style.opacity = available ? "" : "0.55";
		S.sandboxImageBtn.style.cursor = available ? "pointer" : "not-allowed";
		if (title) {
			S.sandboxImageBtn.title = title;
		} else {
			S.sandboxImageBtn.title = "Sandbox image";
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
	if (!(S.sandboxLabel && S.sandboxToggleBtn)) return;
	if (!applySandboxControlAvailability()) {
		S.sandboxLabel.textContent = "disabled";
		S.sandboxToggleBtn.style.borderColor = "";
		S.sandboxToggleBtn.style.color = "var(--muted)";
		return;
	}
	if (S.sessionSandboxEnabled) {
		S.sandboxLabel.textContent = "sandboxed";
		S.sandboxToggleBtn.style.borderColor = "var(--accent, #f59e0b)";
		S.sandboxToggleBtn.style.color = "var(--accent, #f59e0b)";
	} else {
		S.sandboxLabel.textContent = "direct";
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
			sandbox_enabled: newVal,
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

export function updateSandboxImageUI(image) {
	S.setSessionSandboxImage(image || null);
	if (!S.sandboxImageLabel) return;
	if (!applySandboxControlAvailability()) {
		S.sandboxImageLabel.textContent = "unavailable";
		return;
	}
	S.sandboxImageLabel.textContent = image || DEFAULT_IMAGE;
}

export function bindSandboxImageEvents() {
	if (!S.sandboxImageBtn) return;

	S.sandboxImageBtn.addEventListener("click", (e) => {
		if (!sandboxRuntimeAvailable()) return;
		e.stopPropagation();
		toggleImageDropdown();
	});

	document.addEventListener("click", () => {
		if (S.sandboxImageDropdown) {
			S.sandboxImageDropdown.classList.add("hidden");
		}
	});
}

function toggleImageDropdown() {
	if (!S.sandboxImageDropdown) return;
	var isHidden = S.sandboxImageDropdown.classList.contains("hidden");
	if (isHidden) {
		populateImageDropdown();
		S.sandboxImageDropdown.classList.remove("hidden");
	} else {
		S.sandboxImageDropdown.classList.add("hidden");
	}
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
	label.textContent = tag;
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
		sandbox_image: value,
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

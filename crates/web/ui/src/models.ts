// ── Model selector ──────────────────────────────────────────

import { sendRpc } from "./helpers";
import { t } from "./i18n";
import { showModelNotice } from "./pages/ChatPage";
import * as S from "./state";
import { modelStore, REASONING_SEP } from "./stores/model-store";
import type { ModelInfo } from "./types";

function setSessionModel(sessionKey: string, modelId: string): void {
	sendRpc("sessions.patch", { key: sessionKey, model: modelId });
}

export { setSessionModel };

function updateModelComboLabel(model: ModelInfo): void {
	if (S.modelComboLabel) S.modelComboLabel.textContent = model.displayName || model.id;
}

export function fetchModels(): Promise<void> {
	return modelStore.fetch().then(() => {
		// Dual-write to state.js for backward compat
		S.setModels(modelStore.models.value);
		S.setSelectedModelId(modelStore.selectedModelId.value);
		const model = modelStore.selectedModel.value;
		if (model) updateModelComboLabel(model);

		// If the dropdown is currently open, re-render to reflect updated flags
		// (for example when a model becomes unsupported via a WS event).
		if (S.modelDropdown && !S.modelDropdown.classList.contains("hidden")) {
			const query = S.modelSearchInput ? (S.modelSearchInput as HTMLInputElement).value.trim() : "";
			renderModelList(query);
		}
	});
}

export function selectModel(m: ModelInfo): void {
	modelStore.select(m.id);
	// Dual-write to state.js for backward compat
	S.setSelectedModelId(m.id);
	updateModelComboLabel(m);
	localStorage.setItem("moltis-model", m.id);
	setSessionModel(S.activeSessionKey, m.id);
	closeModelDropdown();
	// Show notice if model doesn't support tools
	showModelNotice(m);
}

export function openModelDropdown(): void {
	if (!S.modelDropdown) return;
	S.modelDropdown.classList.remove("hidden");
	(S.modelSearchInput as HTMLInputElement).value = "";
	S.setModelIdx(-1);
	renderModelList("");
	requestAnimationFrame(() => {
		if (S.modelSearchInput) S.modelSearchInput.focus();
	});
}

export function closeModelDropdown(): void {
	if (!S.modelDropdown) return;
	S.modelDropdown.classList.add("hidden");
	if (S.modelSearchInput) (S.modelSearchInput as HTMLInputElement).value = "";
	S.setModelIdx(-1);
}

function buildModelItem(m: ModelInfo, currentId: string): HTMLDivElement {
	const el = document.createElement("div");
	el.className = "model-dropdown-item";
	if (m.id === currentId) el.classList.add("selected");
	if (m.unsupported) el.classList.add("model-dropdown-item-unsupported");

	const label = document.createElement("span");
	label.className = "model-item-label";
	label.textContent = m.displayName || m.id;
	el.appendChild(label);

	const meta = document.createElement("span");
	meta.className = "model-item-meta";

	if (m.provider) {
		const prov = document.createElement("span");
		prov.className = "model-item-provider";
		prov.textContent = m.provider;
		meta.appendChild(prov);
	}

	if (m.supportsReasoning) {
		const brainIcon = document.createElement("span");
		brainIcon.className = "icon icon-xs icon-brain";
		brainIcon.title = "Supports reasoning";
		brainIcon.style.cssText = "opacity:0.5;flex-shrink:0;";
		meta.appendChild(brainIcon);
	}

	if (m.unsupported) {
		const badge = document.createElement("span");
		badge.className = "model-item-unsupported";
		badge.textContent = t("common:labels.unsupported");
		if (m.unsupportedReason) badge.title = m.unsupportedReason;
		meta.appendChild(badge);
	}

	if (meta.childNodes.length > 0) el.appendChild(meta);
	el.addEventListener("click", () => selectModel(m));
	return el;
}

export function renderModelList(query: string): void {
	if (!S.modelDropdownList) return;
	S.modelDropdownList.textContent = "";
	const q = query.toLowerCase();
	const allModels = modelStore.models.value;
	const filtered = allModels.filter((m) => {
		// Hide @reasoning-* virtual variants — the reasoning toggle handles these.
		if (m.id.indexOf(REASONING_SEP) !== -1) return false;
		const label = (m.displayName || m.id).toLowerCase();
		const provider = (m.provider || "").toLowerCase();
		return !q || label.indexOf(q) !== -1 || provider.indexOf(q) !== -1 || m.id.toLowerCase().indexOf(q) !== -1;
	});
	if (filtered.length === 0) {
		const empty = document.createElement("div");
		empty.className = "model-dropdown-empty";
		empty.textContent = t("common:labels.noMatchingModels");
		S.modelDropdownList.appendChild(empty);
		return;
	}
	const currentId = modelStore.selectedModelId.value;
	let lastPreferredIdx = -1;
	for (let i = filtered.length - 1; i >= 0; i--) {
		if (filtered[i].preferred) {
			lastPreferredIdx = i;
			break;
		}
	}
	filtered.forEach((m, idx) => {
		S.modelDropdownList?.appendChild(buildModelItem(m, currentId));

		if (idx === lastPreferredIdx && lastPreferredIdx < filtered.length - 1) {
			const divider = document.createElement("div");
			divider.className = "model-dropdown-divider";
			S.modelDropdownList?.appendChild(divider);
		}
	});
}

function updateModelActive(): void {
	if (!S.modelDropdownList) return;
	const items = S.modelDropdownList.querySelectorAll<HTMLElement>(".model-dropdown-item");
	items.forEach((el, i) => {
		el.classList.toggle("kb-active", i === S.modelIdx);
	});
	if (S.modelIdx >= 0 && items[S.modelIdx]) {
		items[S.modelIdx].scrollIntoView({ block: "nearest" });
	}
}

export function bindModelComboEvents(): void {
	if (!(S.modelComboBtn && S.modelSearchInput && S.modelDropdownList && S.modelCombo)) return;

	S.modelComboBtn.addEventListener("click", () => {
		if (S.modelDropdown?.classList.contains("hidden")) {
			openModelDropdown();
		} else {
			closeModelDropdown();
		}
	});

	S.modelSearchInput.addEventListener("input", () => {
		S.setModelIdx(-1);
		renderModelList((S.modelSearchInput as HTMLInputElement).value.trim());
	});

	S.modelSearchInput.addEventListener("keydown", (e: Event) => {
		const ke = e as KeyboardEvent;
		const items = S.modelDropdownList?.querySelectorAll<HTMLElement>(".model-dropdown-item");
		if (!items) return;
		if (ke.key === "ArrowDown") {
			ke.preventDefault();
			S.setModelIdx(Math.min(S.modelIdx + 1, items.length - 1));
			updateModelActive();
		} else if (ke.key === "ArrowUp") {
			ke.preventDefault();
			S.setModelIdx(Math.max(S.modelIdx - 1, 0));
			updateModelActive();
		} else if (ke.key === "Enter") {
			ke.preventDefault();
			if (S.modelIdx >= 0 && items[S.modelIdx]) {
				items[S.modelIdx].click();
			} else if (items.length === 1) {
				items[0].click();
			}
		} else if (ke.key === "Escape") {
			closeModelDropdown();
			S.modelComboBtn?.focus();
		}
	});
}

document.addEventListener("click", (e: MouseEvent) => {
	if (S.modelCombo && !S.modelCombo.contains(e.target as Node)) {
		closeModelDropdown();
	}
});

window.addEventListener("moltis:locale-changed", () => {
	if (S.modelDropdown && !S.modelDropdown.classList.contains("hidden")) {
		const query = S.modelSearchInput ? (S.modelSearchInput as HTMLInputElement).value.trim() : "";
		renderModelList(query);
	}
});

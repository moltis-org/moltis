// ── Model selector ──────────────────────────────────────────

import { sendRpc } from "./helpers";
import { t } from "./i18n";
import { showModelNotice } from "./pages/ChatPage";
import * as S from "./state";
import { modelStore, REASONING_SEP } from "./stores/model-store";
import { sessionStore } from "./stores/session-store";
import type { ExternalAgentInfo, ModelInfo } from "./types";
import { showToast } from "./ui";

let externalAgents: ExternalAgentInfo[] = [];
let externalAgentsLoaded = false;
let modelsLoaded = false;
let fetchModelsGeneration = 0;
const switchingBackendSessions = new Set<string>();
const externalAgentAutoBindAttempted = new Set<string>();
const externalAgentAutoBindInFlight = new Set<string>();
const externalAgentAutoBindFailures = new Map<string, number>();
const externalAgentAutoBindRetryTimers = new Map<string, number>();
const ACP_AUTO_BIND_RETRY_BASE_MS = 1_000;
const ACP_AUTO_BIND_RETRY_MAX_MS = 30_000;

function installedSelectableExternalAgents(): ExternalAgentInfo[] {
	return externalAgents.filter((agent) => agent.installed && (agent.isAcp || agent.kind === "agy"));
}

function selectableExternalAgents(): ExternalAgentInfo[] {
	return sessionStore.activeSessionKey.value.startsWith("cron:") ? [] : installedSelectableExternalAgents();
}

function activeExternalAgent(): ExternalAgentInfo | null {
	const kind = sessionStore.activeSession.value?.external_agent_kind || "";
	if (!kind) return null;
	return externalAgents.find((agent) => agent.kind === kind) || null;
}

export function updateModelComboAvailability(): void {
	if (!(S.modelComboBtn && S.modelComboLabel)) return;
	const sessionKey = sessionStore.activeSessionKey.value;
	const switchingBackend = switchingBackendSessions.has(sessionKey);
	const externalKind = sessionStore.activeSession.value?.external_agent_kind || "";
	const externalAgent = activeExternalAgent();
	document
		.getElementById("reasoningCombo")
		?.classList.toggle("hidden", Boolean(externalKind) || !modelStore.supportsReasoning.value);
	(S.modelComboBtn as HTMLButtonElement).disabled = switchingBackend;
	S.modelComboBtn.setAttribute("aria-disabled", switchingBackend ? "true" : "false");
	S.modelComboBtn.title = switchingBackend ? "Switching chat backend" : "Select model or external agent";
	if (externalKind) {
		const sessionModel = sessionStore.activeSession.value?.model || "";
		let modelSuffix = "";
		if (sessionModel.startsWith("external-agent::")) {
			const parts = sessionModel.split("::");
			if (parts[2] && parts[2] !== "default") {
				modelSuffix = parts[2];
			}
		}
		const baseLabel = externalAgent?.name || externalKind;
		const label = modelSuffix ? `${baseLabel}: ${modelSuffix}` : baseLabel;
		const unavailable = externalAgent?.installed === false;
		S.modelComboLabel.textContent = unavailable ? `${label} (unavailable)` : label;
		S.modelComboLabel.title = unavailable ? `${label} is unavailable` : `Using ${label}`;
		return;
	}
	const model = modelStore.selectedModel.value;
	if (model) updateModelComboLabel(model);
	else updateExternalAgentOnlyModelComboLabel();
	maybeAutoBindExternalAgent();
}

function fetchExternalAgents(): Promise<ExternalAgentInfo[] | null> {
	return sendRpc<ExternalAgentInfo[]>("external_agents.list", {})
		.then((res) => {
			if (!res?.ok) return null;
			return res.payload || [];
		})
		.catch(() => {
			return null;
		});
}

function updateExternalAgentOnlyModelComboLabel(): void {
	if (!(S.modelComboLabel && modelStore.models.value.length === 0 && selectableExternalAgents().length > 0)) return;
	S.modelComboLabel.textContent = "External agent";
	S.modelComboLabel.title = "Select an external agent";
}

function setExternalAgentKind(sessionKey: string, kind: string | null): void {
	const session = sessionStore.getByKey(sessionKey);
	if (!session) return;
	session.external_agent_kind = kind;
	session.dataVersion.value++;
}

function refreshSessionMetadata(): void {
	void import("./sessions").then(({ fetchSessions }) => fetchSessions());
}

function clearExternalAgentAutoBindRetry(sessionKey: string): void {
	const timer = externalAgentAutoBindRetryTimers.get(sessionKey);
	if (timer !== undefined) window.clearTimeout(timer);
	externalAgentAutoBindRetryTimers.delete(sessionKey);
	externalAgentAutoBindFailures.delete(sessionKey);
}

function scheduleExternalAgentAutoBindRetry(sessionKey: string): void {
	if (externalAgentAutoBindRetryTimers.has(sessionKey)) return;
	const failures = (externalAgentAutoBindFailures.get(sessionKey) || 0) + 1;
	externalAgentAutoBindFailures.set(sessionKey, failures);
	const delay = Math.min(ACP_AUTO_BIND_RETRY_BASE_MS * 2 ** Math.min(failures - 1, 5), ACP_AUTO_BIND_RETRY_MAX_MS);
	const timer = window.setTimeout(() => {
		externalAgentAutoBindRetryTimers.delete(sessionKey);
		if (sessionStore.activeSessionKey.value === sessionKey) updateModelComboAvailability();
	}, delay);
	externalAgentAutoBindRetryTimers.set(sessionKey, timer);
}

function maybeAutoBindExternalAgent(): void {
	const session = sessionStore.activeSession.value;
	const sessionKey = sessionStore.activeSessionKey.value;
	if (
		!(session && sessionKey) ||
		sessionKey.startsWith("cron:") ||
		!modelsLoaded ||
		!externalAgentsLoaded ||
		modelStore.models.value.length > 0 ||
		session.external_agent_kind ||
		externalAgentAutoBindAttempted.has(sessionKey) ||
		externalAgentAutoBindInFlight.has(sessionKey)
	) {
		return;
	}
	const agent = installedSelectableExternalAgents()[0];
	if (!agent) return;
	externalAgentAutoBindInFlight.add(sessionKey);
	void bindExternalAgent(agent, undefined, false)
		.then((bound) => {
			if (bound) {
				externalAgentAutoBindAttempted.add(sessionKey);
				clearExternalAgentAutoBindRetry(sessionKey);
			}
		})
		.finally(() => {
			externalAgentAutoBindInFlight.delete(sessionKey);
			if (!externalAgentAutoBindAttempted.has(sessionKey)) scheduleExternalAgentAutoBindRetry(sessionKey);
		});
}

async function bindExternalAgent(agent: ExternalAgentInfo, modelId?: string, notifyFailure = true): Promise<boolean> {
	const sessionKey = sessionStore.activeSessionKey.value;
	if (!sessionKey || sessionKey.startsWith("cron:")) return false;
	if (switchingBackendSessions.has(sessionKey)) return false;
	const activeSession = sessionStore.activeSession.value;
	const currentModel = activeSession?.model || "";
	const targetModelId = modelId ? `external-agent::${agent.kind}::${modelId}` : `external-agent::${agent.kind}`;
	if (activeSession?.external_agent_kind === agent.kind && (!modelId || currentModel === targetModelId)) {
		closeModelDropdown();
		return true;
	}
	switchingBackendSessions.add(sessionKey);
	updateModelComboAvailability();
	try {
		const res = await sendRpc("external_agents.bind", { sessionKey, kind: agent.kind, model: modelId });
		if (!res?.ok) {
			if (notifyFailure) showToast(res?.error?.message || "Failed to select external agent", "error");
			return false;
		}
		setExternalAgentKind(sessionKey, agent.kind);
		if (activeSession) {
			activeSession.model = targetModelId;
		}
		refreshSessionMetadata();
		closeModelDropdown();
		return true;
	} catch {
		if (notifyFailure) showToast("Failed to select external agent", "error");
		return false;
	} finally {
		switchingBackendSessions.delete(sessionKey);
		updateModelComboAvailability();
	}
}

function setSessionModel(sessionKey: string, modelId: string): void {
	sendRpc("sessions.patch", { key: sessionKey, model: modelId });
}

export { setSessionModel };

export interface ModelLabelInfo {
	id: string;
	displayName?: string;
}

export function modelDisplayLabel(model: ModelLabelInfo): string {
	return model.displayName || model.id;
}

export function modelTitle(model: ModelLabelInfo): string {
	const label = modelDisplayLabel(model);
	return model.displayName && model.displayName !== model.id ? `${model.displayName} (${model.id})` : label;
}

function updateModelComboLabel(model: ModelInfo): void {
	if (!S.modelComboLabel) return;
	const label = modelDisplayLabel(model);
	S.modelComboLabel.textContent = label;
	S.modelComboLabel.title = modelTitle(model);
}

export function fetchModels(): Promise<void> {
	const generation = ++fetchModelsGeneration;
	return Promise.all([modelStore.fetch(), fetchExternalAgents()]).then(([didLoadModels, agents]) => {
		if (generation !== fetchModelsGeneration) return;
		modelsLoaded = didLoadModels;
		externalAgentsLoaded = agents !== null;
		if (agents !== null) externalAgents = agents;
		// Dual-write to state.js for backward compat
		S.setModels(modelStore.models.value);
		S.setSelectedModelId(modelStore.selectedModelId.value);
		const model = modelStore.selectedModel.value;
		if (model) updateModelComboLabel(model);
		else updateExternalAgentOnlyModelComboLabel();
		updateModelComboAvailability();

		// If the dropdown is currently open, re-render to reflect updated flags
		// (for example when a model becomes unsupported via a WS event).
		if (S.modelDropdown && !S.modelDropdown.classList.contains("hidden")) {
			const query = S.modelSearchInput ? (S.modelSearchInput as HTMLInputElement).value.trim() : "";
			renderModelList(query);
		}
	});
}

function commitModelSelection(m: ModelInfo, sessionKey = S.activeSessionKey): void {
	modelStore.select(m.id);
	// Dual-write to state.js for backward compat
	S.setSelectedModelId(m.id);
	updateModelComboLabel(m);
	localStorage.setItem("moltis-model", m.id);
	setSessionModel(sessionKey, m.id);
	closeModelDropdown();
	// Show notice if model doesn't support tools
	showModelNotice(m);
}

export function selectModel(m: ModelInfo): void {
	const externalKind = sessionStore.activeSession.value?.external_agent_kind || "";
	if (!externalKind) {
		commitModelSelection(m);
		return;
	}
	const sessionKey = sessionStore.activeSessionKey.value;
	if (!sessionKey) return;
	if (switchingBackendSessions.has(sessionKey)) return;
	switchingBackendSessions.add(sessionKey);
	updateModelComboAvailability();
	void sendRpc("external_agents.unbind", { sessionKey })
		.then((res) => {
			if (!res?.ok) {
				showToast(res?.error?.message || "Failed to switch to the selected model", "error");
				return;
			}
			setExternalAgentKind(sessionKey, null);
			refreshSessionMetadata();
			if (sessionStore.activeSessionKey.value === sessionKey) {
				commitModelSelection(m, sessionKey);
			} else {
				setSessionModel(sessionKey, m.id);
			}
		})
		.catch(() => {
			showToast("Failed to switch to the selected model", "error");
		})
		.finally(() => {
			switchingBackendSessions.delete(sessionKey);
			updateModelComboAvailability();
		});
}

export function openModelDropdown(): void {
	if ((S.modelComboBtn as HTMLButtonElement | null)?.disabled) return;
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
	el.dataset.modelId = m.id;
	if (m.id === currentId) el.classList.add("selected");
	if (m.unsupported) el.classList.add("model-dropdown-item-unsupported");

	const label = document.createElement("span");
	label.className = "model-item-label";
	label.textContent = modelDisplayLabel(m);
	label.title = modelTitle(m);
	el.title = label.title;
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

function buildExternalAgentItem(agent: ExternalAgentInfo, currentKind: string): HTMLDivElement {
	const el = document.createElement("div");
	el.className = "model-dropdown-item";
	el.dataset.externalAgentKind = agent.kind;
	if (agent.kind === currentKind) el.classList.add("selected");
	el.title = `${agent.name} (${agent.kind})`;

	const label = document.createElement("span");
	label.className = "model-item-label";
	label.textContent = agent.name;
	label.title = el.title;
	el.appendChild(label);

	const meta = document.createElement("span");
	meta.className = "model-item-meta";
	const provider = document.createElement("span");
	provider.className = "model-item-provider";
	provider.textContent = agent.isAcp ? "ACP agent" : "External agent";
	meta.appendChild(provider);
	el.appendChild(meta);
	el.addEventListener("click", () => void bindExternalAgent(agent));
	return el;
}

function buildExternalAgentModelItem(
	agent: ExternalAgentInfo,
	model: string,
	currentKind: string,
	currentModel: string,
): HTMLDivElement {
	const el = document.createElement("div");
	el.className = "model-dropdown-item";
	el.dataset.externalAgentKind = agent.kind;
	el.dataset.externalAgentModel = model;
	const expectedModelId = `external-agent::${agent.kind}::${model}`;
	const isSelected = agent.kind === currentKind && currentModel === expectedModelId;
	if (isSelected) el.classList.add("selected");
	el.title = `${agent.name}: ${model}`;

	const label = document.createElement("span");
	label.className = "model-item-label";
	label.textContent = model;
	label.title = el.title;
	el.appendChild(label);

	const meta = document.createElement("span");
	meta.className = "model-item-meta";
	const provider = document.createElement("span");
	provider.className = "model-item-provider";
	provider.textContent = `${agent.name} agent`;
	meta.appendChild(provider);
	el.appendChild(meta);
	el.addEventListener("click", () => void bindExternalAgent(agent, model));
	return el;
}

function appendDivider(): void {
	const divider = document.createElement("div");
	divider.className = "model-dropdown-divider";
	S.modelDropdownList?.appendChild(divider);
}

export function renderModelList(query: string): void {
	if (!S.modelDropdownList) return;
	S.modelDropdownList.textContent = "";
	const q = query.toLowerCase();
	const allModels = modelStore.models.value;
	const selectableAgents = selectableExternalAgents().filter((agent) => {
		const name = agent.name.toLowerCase();
		const modelsMatch = (agent.models || []).some((m) => m.toLowerCase().includes(q));
		return (
			!q ||
			name.includes(q) ||
			agent.kind.toLowerCase().includes(q) ||
			"external agent".includes(q) ||
			(agent.isAcp && "acp agent".includes(q)) ||
			modelsMatch
		);
	});
	const filtered = allModels.filter((m) => {
		// Hide @reasoning-* virtual variants — the reasoning toggle handles these.
		if (m.id.indexOf(REASONING_SEP) !== -1) return false;
		const label = (m.displayName || m.id).toLowerCase();
		const provider = (m.provider || "").toLowerCase();
		return !q || label.indexOf(q) !== -1 || provider.indexOf(q) !== -1 || m.id.toLowerCase().indexOf(q) !== -1;
	});
	if (filtered.length === 0 && selectableAgents.length === 0) {
		const empty = document.createElement("div");
		empty.className = "model-dropdown-empty";
		empty.textContent = t("common:labels.noMatchingModels");
		S.modelDropdownList.appendChild(empty);
		return;
	}
	const currentKind = sessionStore.activeSession.value?.external_agent_kind || "";
	const currentModel = sessionStore.activeSession.value?.model || "";
	for (const agent of selectableAgents) {
		if (agent.models && agent.models.length > 0) {
			const matchingModels = agent.models.filter(
				(m) =>
					!q ||
					m.toLowerCase().includes(q) ||
					agent.name.toLowerCase().includes(q) ||
					agent.kind.toLowerCase().includes(q),
			);
			for (const model of matchingModels) {
				S.modelDropdownList.appendChild(buildExternalAgentModelItem(agent, model, currentKind, currentModel));
			}
		} else {
			S.modelDropdownList.appendChild(buildExternalAgentItem(agent, currentKind));
		}
	}
	if (selectableAgents.length > 0 && filtered.length > 0) appendDivider();
	const currentId = currentKind ? "" : modelStore.selectedModelId.value;
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
			appendDivider();
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
		if ((S.modelComboBtn as HTMLButtonElement | null)?.disabled) return;
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

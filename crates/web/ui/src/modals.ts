// ── Modals: create modal DOM on demand ───────────────────────

import { t } from "./i18n";

const root = document.getElementById("modalRoot");

function createModal(id: string, titleId: string, bodyId: string, closeId: string): HTMLElement {
	const existing = document.getElementById(id);
	if (existing) return existing;

	const backdrop = document.createElement("div");
	backdrop.id = id;
	backdrop.className = "provider-modal-backdrop hidden";

	const modal = document.createElement("div");
	modal.className = "provider-modal";

	const header = document.createElement("div");
	header.className = "provider-modal-header";

	const title = document.createElement("span");
	title.id = titleId;
	title.className = "text-sm font-medium text-[var(--text-strong)]";
	header.appendChild(title);

	const closeBtn = document.createElement("button");
	closeBtn.id = closeId;
	closeBtn.className =
		"text-[var(--muted)] hover:text-[var(--text)] cursor-pointer bg-transparent border-none text-lg leading-none";
	closeBtn.textContent = "\u00D7";
	header.appendChild(closeBtn);

	modal.appendChild(header);

	const body = document.createElement("div");
	body.id = bodyId;
	body.className = "provider-modal-body";
	modal.appendChild(body);

	backdrop.appendChild(modal);
	root?.appendChild(backdrop);
	return backdrop;
}

export function ensureProviderModal(): HTMLElement {
	const el = createModal("providerModal", "providerModalTitle", "providerModalBody", "providerModalClose");
	const title = document.getElementById("providerModalTitle");
	if (title) title.textContent = t("common:modals.addProvider");
	return el;
}

export function ensureChannelModal(): HTMLElement {
	const el = createModal("channelModal", "channelModalTitle", "channelModalBody", "channelModalClose");
	const title = document.getElementById("channelModalTitle");
	if (title) title.textContent = t("common:modals.addChannel");
	return el;
}

export function ensureProjectModal(): HTMLElement {
	const el = createModal("projectModal", "projectModalTitle", "projectModalBody", "projectModalClose");
	const title = document.getElementById("projectModalTitle");
	if (title) title.textContent = t("common:modals.manageProjects");
	return el;
}

function refreshModalTitles(): void {
	const provider = document.getElementById("providerModalTitle");
	if (provider) provider.textContent = t("common:modals.addProvider");
	const channel = document.getElementById("channelModalTitle");
	if (channel) channel.textContent = t("common:modals.addChannel");
	const project = document.getElementById("projectModalTitle");
	if (project) project.textContent = t("common:modals.manageProjects");
}

window.addEventListener("moltis:locale-changed", refreshModalTitles);

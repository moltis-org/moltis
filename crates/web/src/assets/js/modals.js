// ── Modals: create modal DOM on demand ───────────────────────

var root = document.getElementById("modalRoot");

function createModal(id, titleId, bodyId, closeId) {
	var existing = document.getElementById(id);
	if (existing) return existing;

	var backdrop = document.createElement("div");
	backdrop.id = id;
	backdrop.className = "provider-modal-backdrop hidden";

	var modal = document.createElement("div");
	modal.className = "provider-modal";

	var header = document.createElement("div");
	header.className = "provider-modal-header";

	var title = document.createElement("span");
	title.id = titleId;
	title.className = "text-sm font-medium text-[var(--text-strong)]";
	header.appendChild(title);

	var closeBtn = document.createElement("button");
	closeBtn.id = closeId;
	closeBtn.className =
		"text-[var(--muted)] hover:text-[var(--text)] cursor-pointer bg-transparent border-none text-lg leading-none";
	closeBtn.textContent = "\u00D7";
	header.appendChild(closeBtn);

	modal.appendChild(header);

	var body = document.createElement("div");
	body.id = bodyId;
	body.className = "provider-modal-body";
	modal.appendChild(body);

	backdrop.appendChild(modal);
	root.appendChild(backdrop);
	return backdrop;
}

export function ensureProviderModal() {
	var el = createModal("providerModal", "providerModalTitle", "providerModalBody", "providerModalClose");
	var title = document.getElementById("providerModalTitle");
	title.textContent = "Add Provider";
	return el;
}

export function ensureChannelModal() {
	var el = createModal("channelModal", "channelModalTitle", "channelModalBody", "channelModalClose");
	var title = document.getElementById("channelModalTitle");
	title.textContent = "Add Channel";
	return el;
}

export function ensureProjectModal() {
	var el = createModal("projectModal", "projectModalTitle", "projectModalBody", "projectModalClose");
	var title = document.getElementById("projectModalTitle");
	title.textContent = "Manage Projects";
	return el;
}

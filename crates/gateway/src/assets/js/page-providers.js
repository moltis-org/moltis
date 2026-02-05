// ── Providers page (Preact + HTM + Signals) ─────────────────

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import { sendRpc } from "./helpers.js";
import { fetchModels } from "./models.js";
import { updateNavCount } from "./nav-counts.js";
import { openProviderModal } from "./providers.js";
import { registerPage } from "./router.js";
import { connected } from "./signals.js";
import * as S from "./state.js";
import { ConfirmDialog, requestConfirm } from "./ui.js";

var configuredModels = signal([]);
var loading = signal(false);

function fetchProviders() {
	loading.value = true;
	// Fetch actual models from the registry - this shows all configured models
	sendRpc("models.list", {}).then((res) => {
		loading.value = false;
		if (!res?.ok) return;
		// Sort: local providers first, then alphabetically by provider name
		var models = (res.payload || []).sort((a, b) => {
			var aIsLocal = a.provider === "local-llm" || a.provider === "ollama";
			var bIsLocal = b.provider === "local-llm" || b.provider === "ollama";
			if (aIsLocal && !bIsLocal) return -1;
			if (!aIsLocal && bIsLocal) return 1;
			if (a.provider !== b.provider) return a.provider.localeCompare(b.provider);
			return (a.displayName || a.id).localeCompare(b.displayName || b.id);
		});
		configuredModels.value = models;
		updateNavCount("providers", models.length);
	});
}

function ModelCard(props) {
	var m = props.model;
	var isLocal = m.provider === "local-llm";

	function onRemove() {
		var msg = `Remove ${m.displayName || m.id}?`;
		requestConfirm(msg).then((yes) => {
			if (!yes) return;
			// For local-llm models, actually remove from config
			// For other providers, disable (hide) the model
			if (isLocal) {
				sendRpc("providers.local.remove_model", { modelId: m.id }).then((res) => {
					if (res?.ok) {
						fetchModels();
						fetchProviders();
					}
				});
			} else {
				// Disable (hide) the model without removing the provider
				sendRpc("models.disable", { modelId: m.id }).then((res) => {
					if (res?.ok) {
						fetchModels();
						fetchProviders();
					}
				});
			}
		});
	}

	function getProviderBadge() {
		if (m.provider === "local-llm") return "Local";
		if (m.provider === "ollama") return "Ollama";
		return m.provider;
	}

	var displayName = m.displayName || m.id;

	return html`<div class="provider-item" style="margin-bottom:0;cursor:default;">
		<div style="flex:1;min-width:0;">
			<div style="display:flex;align-items:center;gap:8px;">
				<span class="provider-item-name">${displayName}</span>
				<span class="provider-item-badge ${isLocal ? "local" : "api-key"}">
					${getProviderBadge()}
				</span>
				${m.supportsTools ? null : html`<span class="provider-item-badge" style="background:var(--warning-bg,rgba(234,179,8,.15));color:var(--warning,#eab308);border-color:var(--warning,#eab308);">Chat only</span>`}
			</div>
			<div style="font-size:.7rem;color:var(--muted);margin-top:4px;">
				<span style="font-family:var(--font-mono);">${m.id}</span>
			</div>
		</div>
		<button
			class="provider-btn provider-btn-danger"
			onClick=${onRemove}
		>
			Remove
		</button>
	</div>`;
}

function ProvidersPage() {
	useEffect(() => {
		if (connected.value) fetchProviders();
	}, [connected.value]);

	S.setRefreshProvidersPage(fetchProviders);

	return html`
		<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 class="text-lg font-medium text-[var(--text-strong)]">Providers</h2>
			<p class="text-xs text-[var(--muted)] leading-relaxed max-w-form" style="margin:0;">
				Configure LLM providers for chat and agent tasks. You can add multiple providers and switch between models.
			</p>

			<div style="max-width:600px;">
				${
					loading.value && configuredModels.value.length === 0
						? html`<div class="text-xs text-[var(--muted)]">Loading…</div>`
						: configuredModels.value.length === 0
							? html`<div class="text-xs text-[var(--muted)]" style="padding:12px 0;">No providers configured yet.</div>`
							: html`<div style="display:flex;flex-direction:column;gap:6px;margin-bottom:12px;">
								${configuredModels.value.map((m) => html`<${ModelCard} key=${m.id} model=${m} />`)}
							</div>`
				}

				<button
					class="provider-btn"
					onClick=${() => {
						if (connected.value) openProviderModal();
					}}
				>
					Add Provider
				</button>
			</div>
		</div>
		<${ConfirmDialog} />
	`;
}

registerPage(
	"/providers",
	function initProviders(container) {
		container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
		render(html`<${ProvidersPage} />`, container);
	},
	function teardownProviders() {
		S.setRefreshProvidersPage(null);
		var container = S.$("pageContent");
		if (container) render(null, container);
	},
);

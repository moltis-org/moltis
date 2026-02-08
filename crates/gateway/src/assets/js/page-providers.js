// ── Providers page (Preact + HTM + Signals) ─────────────────

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import { onEvent } from "./events.js";
import { sendRpc } from "./helpers.js";
import { fetchModels } from "./models.js";
import { updateNavCount } from "./nav-counts.js";
import { openProviderModal } from "./providers.js";
import { registerPage } from "./router.js";
import { connected } from "./signals.js";
import * as S from "./state.js";

var configuredModels = signal([]);
var loading = signal(false);
var detectingModels = signal(false);
var detectSummary = signal(null);
var detectError = signal("");
var detectProgress = signal(null);
var deletingProvider = signal("");
var providerActionError = signal("");

function providerSortRank(provider) {
	return provider === "local-llm" || provider === "ollama" ? 0 : 1;
}

function countUniqueProviders(models) {
	return new Set(models.map((m) => m.provider)).size;
}

function progressFromPayload(payload) {
	return {
		total: payload?.total || 0,
		checked: payload?.checked || 0,
		supported: payload?.supported || 0,
		unsupported: payload?.unsupported || 0,
		errors: payload?.errors || 0,
	};
}

function handleModelsUpdatedEvent(payload) {
	if (!payload?.phase) return;
	if (payload.phase === "start") {
		detectingModels.value = true;
		detectError.value = "";
		detectSummary.value = null;
		detectProgress.value = progressFromPayload(payload);
		return;
	}
	if (payload.phase === "progress") {
		detectingModels.value = true;
		detectProgress.value = progressFromPayload(payload);
		return;
	}
	if (payload.phase === "complete") {
		detectingModels.value = false;
		if (payload.summary) {
			detectSummary.value = payload.summary;
			detectProgress.value = progressFromPayload(payload.summary);
		}
		return;
	}
	if (payload.phase === "error") {
		detectingModels.value = false;
		detectError.value = payload.error || "Model detection failed.";
	}
}

function fetchProviders() {
	loading.value = true;
	return Promise.all([sendRpc("models.list_all", {}), sendRpc("providers.available", {})])
		.then(([modelsRes, providersRes]) => {
			loading.value = false;
			var providerMeta = new Map();
			var configuredProviders = [];
			if (providersRes?.ok) {
				configuredProviders = (providersRes.payload || []).filter(
					(configuredProvider) => configuredProvider.configured,
				);
				for (var providerMetaEntry of providersRes.payload || []) {
					providerMeta.set(providerMetaEntry.name, providerMetaEntry);
				}
			}

			var models = [];
			if (modelsRes?.ok) {
				models = (modelsRes.payload || []).map((m) => ({
					...m,
					providerDisplayName: providerMeta.get(m.provider)?.displayName || m.provider,
					authType: providerMeta.get(m.provider)?.authType || "api-key",
				}));
			}

			// Sort: local providers first, then alphabetically by provider name
			models = models.sort((a, b) => {
				var aRank = providerSortRank(a.provider);
				var bRank = providerSortRank(b.provider);
				if (aRank !== bRank) return aRank - bRank;
				if (a.provider !== b.provider) return a.provider.localeCompare(b.provider);
				return (a.displayName || a.id).localeCompare(b.displayName || b.id);
			});

			// Include configured providers that don't currently expose a model.
			var modelProviders = new Set(models.map((m) => m.provider));
			var providerOnlyRows = [];
			providerOnlyRows = configuredProviders
				.filter((providerWithoutModels) => !modelProviders.has(providerWithoutModels.name))
				.map((providerWithoutModels) => ({
					id: `provider:${providerWithoutModels.name}`,
					provider: providerWithoutModels.name,
					displayName: providerWithoutModels.displayName,
					providerDisplayName: providerWithoutModels.displayName,
					providerOnly: true,
					authType: providerWithoutModels.authType,
				}));

			configuredModels.value = [...models, ...providerOnlyRows];
			updateNavCount("providers", countUniqueProviders(configuredModels.value));
		})
		.catch(() => {
			loading.value = false;
		});
}

async function runDetectAllModels() {
	if (!connected.value || detectingModels.value) return;
	detectingModels.value = true;
	detectSummary.value = null;
	detectError.value = "";
	detectProgress.value = null;

	try {
		// Phase 1: show current full list first before probing.
		await Promise.all([fetchModels(), fetchProviders()]);
		await new Promise((resolve) => {
			requestAnimationFrame(resolve);
		});

		var res = await sendRpc("models.detect_supported", {});
		if (!res?.ok) {
			detectError.value = res?.error?.message || "Failed to detect model availability.";
			detectingModels.value = false;
			return;
		}
		if (res.payload?.skipped) {
			detectingModels.value = false;
			return;
		}
		detectSummary.value = res.payload || null;
		detectProgress.value = progressFromPayload(res.payload);
		await Promise.all([fetchModels(), fetchProviders()]);
		var p = detectProgress.value;
		if (!p || p.total === 0 || p.checked >= p.total) {
			detectingModels.value = false;
		}
	} catch (_err) {
		detectingModels.value = false;
	}
}

function groupProviderRows(models) {
	var groups = new Map();
	for (var row of models) {
		var key = row.provider;
		if (!groups.has(key)) {
			groups.set(key, {
				provider: key,
				providerDisplayName: row.providerDisplayName || row.displayName || key,
				authType: row.authType || "api-key",
				models: [],
			});
		}
		var groupEntry = groups.get(key);
		if (!row.providerOnly) {
			groupEntry.models.push(row);
		}
	}

	var result = Array.from(groups.values());
	result.sort((a, b) => {
		var aRank = providerSortRank(a.provider);
		var bRank = providerSortRank(b.provider);
		if (aRank !== bRank) return aRank - bRank;
		return a.providerDisplayName.localeCompare(b.providerDisplayName);
	});
	for (var providerGroup of result) {
		providerGroup.models.sort((a, b) => (a.displayName || a.id).localeCompare(b.displayName || b.id));
	}
	return result;
}

function ProviderSection(props) {
	var group = props.group;

	function onDeleteProvider() {
		if (deletingProvider.value) return;
		deletingProvider.value = group.provider;
		providerActionError.value = "";
		sendRpc("providers.remove_key", { provider: group.provider })
			.then((res) => {
				if (res?.ok) {
					configuredModels.value = configuredModels.value.filter((entry) => entry.provider !== group.provider);
					fetchModels();
					fetchProviders();
					return;
				}
				providerActionError.value = res?.error?.message || "Failed to delete provider.";
			})
			.catch(() => {
				providerActionError.value = "Failed to delete provider.";
			})
			.finally(() => {
				deletingProvider.value = "";
			});
	}

	function onToggleModel(model) {
		var method = model.disabled ? "models.enable" : "models.disable";
		sendRpc(method, { modelId: model.id }).then((res) => {
			if (res?.ok) {
				providerActionError.value = "";
				configuredModels.value = configuredModels.value.map((entry) =>
					entry.id === model.id ? { ...entry, disabled: !model.disabled } : entry,
				);
				fetchModels();
				fetchProviders();
			} else {
				providerActionError.value = res?.error?.message || "Failed to update model state.";
			}
		});
	}

	return html`<div class="max-w-form py-1">
		<div class="flex items-center justify-between gap-3">
			<div class="flex items-center gap-2 min-w-0">
				<h3 class="text-base font-semibold text-[var(--text-strong)] truncate">${group.providerDisplayName}</h3>
				<span class="provider-item-badge ${group.authType}">
					${group.authType === "oauth" ? "OAuth" : group.authType === "local" ? "Local" : "API Key"}
				</span>
			</div>
			<div class="shrink-0">
				<button
					class="provider-btn provider-btn-danger provider-btn-sm"
					disabled=${deletingProvider.value === group.provider}
					onClick=${onDeleteProvider}
				>
					${deletingProvider.value === group.provider ? "Deleting..." : "Delete"}
				</button>
			</div>
		</div>
		<div class="mt-2 border-b border-[var(--border)]"></div>
		${
			group.models.length === 0
				? html`<div class="mt-2 text-xs text-[var(--muted)]">No active models.</div>`
				: html`<div class="mt-2 flex flex-col gap-2">
					${group.models.map(
						(model) => html`<div key=${model.id} class="flex items-start justify-between gap-3 py-1">
							<div class="min-w-0 flex-1">
								<div class="flex items-center gap-2 min-w-0">
									<div class="text-m font-medium text-[var(--text-strong)] truncate">${model.displayName || model.id}</div>
									${model.unsupported ? html`<span class="provider-item-badge warning" title=${model.unsupportedReason || "Model is not supported for this account/provider"}>Unsupported</span>` : null}
									${model.supportsTools ? null : html`<span class="provider-item-badge warning">Chat only</span>`}
									${model.disabled ? html`<span class="provider-item-badge muted">Disabled</span>` : null}
								</div>
								<div class="mt-1 text-xs text-[var(--muted)] font-mono opacity-75">${model.id}</div>
							</div>
							<button class="provider-btn provider-btn-secondary provider-btn-sm" onClick=${() => onToggleModel(model)}>
								${model.disabled ? "Enable" : "Disable"}
							</button>
						</div>`,
					)}
				</div>`
		}
	</div>`;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Provider page UI composition keeps related sections in one render function.
function ProvidersPage() {
	useEffect(() => {
		if (connected.value) fetchProviders();
		var offModelsUpdated = onEvent("models.updated", handleModelsUpdatedEvent);

		return () => {
			offModelsUpdated();
		};
	}, [connected.value]);

	S.setRefreshProvidersPage(fetchProviders);

	var progressValue = detectProgress.value || { total: 0, checked: 0, supported: 0, unsupported: 0, errors: 0 };
	var progressPercent = progressValue.total > 0 ? Math.round((progressValue.checked / progressValue.total) * 100) : 0;

	return html`
		<div class="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<div class="flex items-center gap-3">
					<h2 class="text-lg font-medium text-[var(--text-strong)]">Providers</h2>
					<button
						class="provider-btn"
						onClick=${() => {
							if (connected.value) openProviderModal();
						}}
					>
						Add Provider
					</button>
					<button
						class="provider-btn provider-btn-secondary"
						disabled=${!connected.value || detectingModels.value}
						onClick=${runDetectAllModels}
					>
						${detectingModels.value ? "Detecting Models..." : "Detect All Models"}
					</button>
				</div>
				<p class="text-xs text-[var(--muted)] leading-relaxed max-w-form" style="margin:0;">
					Configure LLM providers for chat and agent tasks. You can add multiple providers and switch between models.
				</p>
				<p class="text-xs text-[var(--muted)] leading-relaxed max-w-form" style="margin:0;">
					Detect All Models sends a single word (<code class="font-mono">ping</code>) to each model to check if your account can use it, then flags unsupported models.
				</p>
				<div
					class="max-w-form rounded-md border border-[var(--border)] bg-[var(--surface2)] px-3 py-2 text-xs text-[var(--muted)]"
				>
					Unsupported models are advertised by the provider, but a probe ping returned an error for your account.
				</div>
				${
					detectError.value
						? html`<div class="text-xs text-[var(--danger,#ef4444)] max-w-form">${detectError.value}</div>`
						: null
				}
				${
					providerActionError.value
						? html`<div class="text-xs text-[var(--danger,#ef4444)] max-w-form">${providerActionError.value}</div>`
						: null
				}
				${
					detectSummary.value
						? html`<div class="text-xs text-[var(--muted)] max-w-form">
							Checked ${detectSummary.value.checked || 0}/${detectSummary.value.total || 0}.
							Supported: ${detectSummary.value.supported || 0},
							Unsupported: ${detectSummary.value.unsupported || 0},
							Errors: ${detectSummary.value.errors || 0}.
						</div>`
						: null
				}
				${
					detectingModels.value || (detectProgress.value && progressValue.total > 0)
						? html`<div class="max-w-form">
							<div class="h-2 w-full overflow-hidden rounded-sm border border-[var(--border)] bg-[var(--surface2)]">
								<div
									class="h-full bg-[var(--accent)] transition-all duration-150"
									style=${`width:${progressPercent}%;`}
								></div>
							</div>
							<div class="mt-1 text-xs text-[var(--muted)]">
								Probing models: ${progressValue.checked}/${progressValue.total} (${progressPercent}%)
							</div>
						</div>`
						: null
				}

				<div style="max-width:600px;">
				${
					loading.value && configuredModels.value.length === 0
						? html`<div class="text-xs text-[var(--muted)]">Loading…</div>`
						: configuredModels.value.length === 0
							? html`<div class="text-xs text-[var(--muted)]" style="padding:12px 0;">No providers configured yet.</div>`
							: html`<div style="display:flex;flex-direction:column;gap:6px;margin-bottom:12px;">
								${groupProviderRows(configuredModels.value).map((g) => html`<${ProviderSection} key=${g.provider} group=${g} />`)}
							</div>`
				}

				</div>
			</div>
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

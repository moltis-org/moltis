// ── LLMs page (Preact + HTM + Signals) ──────────────────────

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect } from "preact/hooks";
import { onEvent } from "./events.js";
import { sendRpc } from "./helpers.js";
import { fetchModels } from "./models.js";
import { updateNavCount } from "./nav-counts.js";
import { openModelSelectorForProvider, openProviderModal } from "./providers.js";
import { connected } from "./signals.js";
import * as S from "./state.js";
import { ConfirmDialog, requestConfirm } from "./ui.js";

var configuredModels = signal([]);
var providerMetaSig = signal(new Map());
var loading = signal(false);
var detectingModels = signal(false);
var detectSummary = signal(null);
var detectError = signal("");
var detectProgress = signal(null);
var deletingProvider = signal("");
var providerActionError = signal("");

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
			providerMetaSig.value = providerMeta;

			var models = [];
			if (modelsRes?.ok) {
				models = (modelsRes.payload || []).map((m) => ({
					...m,
					providerDisplayName: providerMeta.get(m.provider)?.displayName || m.provider,
					authType: providerMeta.get(m.provider)?.authType || "api-key",
				}));
			}

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

function groupProviderRows(models, metaMap) {
	var groups = new Map();
	for (var row of models) {
		var key = row.provider;
		if (!groups.has(key)) {
			groups.set(key, {
				provider: key,
				providerDisplayName: row.providerDisplayName || row.displayName || key,
				authType: row.authType || "api-key",
				selectedModel: metaMap?.get(key)?.model || null,
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
		var aOrder = metaMap?.get(a.provider)?.uiOrder;
		var bOrder = metaMap?.get(b.provider)?.uiOrder;
		var hasAOrder = Number.isFinite(aOrder);
		var hasBOrder = Number.isFinite(bOrder);
		if (hasAOrder && hasBOrder && aOrder !== bOrder) return aOrder - bOrder;
		if (hasAOrder && !hasBOrder) return -1;
		if (!hasAOrder && hasBOrder) return 1;
		return a.providerDisplayName.localeCompare(b.providerDisplayName);
	});
	for (var providerGroup of result) {
		providerGroup.models.sort((a, b) => {
			var aTime = a.createdAt || 0;
			var bTime = b.createdAt || 0;
			if (aTime !== bTime) return bTime - aTime;
			return (a.displayName || a.id).localeCompare(b.displayName || b.id);
		});
	}
	return result;
}

function ProviderSection(props) {
	var group = props.group;

	function onDeleteProvider() {
		if (deletingProvider.value) return;
		requestConfirm(`Remove ${group.providerDisplayName} and all its credentials?`).then((yes) => {
			if (!yes) return;
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

	function onSelectModels() {
		openModelSelectorForProvider(group.provider, group.providerDisplayName);
	}

	return html`<div id=${`provider-${group.provider}`} class="max-w-form py-1">
		<div class="flex items-center justify-between gap-3">
			<div class="flex items-center gap-2 min-w-0">
				<h3 class="text-base font-semibold text-[var(--text-strong)] truncate">${group.providerDisplayName}</h3>
				<span class="provider-item-badge ${group.authType}">
					${group.authType === "oauth" ? "OAuth" : group.authType === "local" ? "Local" : "API Key"}
				</span>
			</div>
			<div class="flex gap-2 shrink-0">
				${group.models.length > 0 ? html`<button class="provider-btn provider-btn-secondary provider-btn-sm" onClick=${onSelectModels}>Preferred Models</button>` : null}
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
									<div class="text-sm font-medium text-[var(--text-strong)] truncate">${model.displayName || model.id}</div>
									${model.unsupported ? html`<span class="provider-item-badge warning" title=${model.unsupportedReason || "Model is not supported for this account"}>Unsupported</span>` : null}
									${model.supportsTools ? null : html`<span class="provider-item-badge warning">Chat only</span>`}
									${model.disabled ? html`<span class="provider-item-badge muted">Disabled</span>` : null}
								</div>
								<div class="mt-1 text-xs text-[var(--muted)] font-mono opacity-75">${model.id}</div>
								${model.createdAt ? html`<time class="mt-0.5 text-xs text-[var(--muted)] opacity-60 block" data-epoch-ms=${model.createdAt * 1000} data-format="year-month"></time>` : null}
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
					<h2 class="text-lg font-medium text-[var(--text-strong)]">LLMs</h2>
					<button
						class="provider-btn"
						onClick=${() => {
							if (connected.value) openProviderModal();
						}}
					>
						Add LLM
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
				${
					detectError.value || providerActionError.value
						? html`<div class="text-xs text-[var(--danger,#ef4444)] max-w-form">${detectError.value || providerActionError.value}</div>`
						: null
				}
				${
					detectingModels.value
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
						: detectSummary.value
							? html`<div class="text-xs text-[var(--muted)] max-w-form">
								Detected ${detectSummary.value.supported || 0} supported, ${detectSummary.value.unsupported || 0} unsupported out of ${detectSummary.value.total || 0} models.
							</div>`
							: null
				}

				${(() => {
					var groups = groupProviderRows(configuredModels.value, providerMetaSig.value);
					if (loading.value && configuredModels.value.length === 0) {
						return html`<div class="text-xs text-[var(--muted)]">Loading…</div>`;
					}
					if (configuredModels.value.length === 0) {
						return html`<div class="text-xs text-[var(--muted)]" style="padding:12px 0;">No LLM providers configured yet.</div>`;
					}
					return html`<div style="max-width:600px;">
						${
							groups.length > 1
								? html`<div class="flex flex-wrap gap-1 mb-3">
							${groups.map(
								(g) => html`<button
								key=${g.provider}
								class="text-xs px-2 py-1 rounded-md border border-[var(--border)] bg-[var(--surface)] text-[var(--muted)] hover:text-[var(--text)] hover:border-[var(--border-strong)] cursor-pointer"
								onClick=${() => {
									var el = document.getElementById(`provider-${g.provider}`);
									if (el) el.scrollIntoView({ behavior: "smooth", block: "start" });
								}}
							>${g.providerDisplayName}<span class="ml-1 opacity-60">${g.models.length}</span></button>`,
							)}
						</div>`
								: null
						}
						<div style="display:flex;flex-direction:column;gap:6px;margin-bottom:12px;">
							${groups.map((g) => html`<${ProviderSection} key=${g.provider} group=${g} />`)}
						</div>
					</div>`;
				})()}
			</div>
		<${ConfirmDialog} />
		`;
}

var _providersContainer = null;

export function initProviders(container) {
	_providersContainer = container;
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	render(html`<${ProvidersPage} />`, container);
}

export function teardownProviders() {
	S.setRefreshProvidersPage(null);
	if (_providersContainer) render(null, _providersContainer);
	_providersContainer = null;
}

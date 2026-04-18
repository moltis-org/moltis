// ── LLMs page (Preact + Signals) ──────────────────────────────

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import { onEvent } from "../events";
import { modelVersionScore, sendRpc } from "../helpers";
import { t } from "../i18n";
import { fetchModels } from "../models";
import { updateNavCount } from "../nav-counts";
import { testModel } from "../provider-validation";
import { openModelSelectorForProvider, openProviderModal } from "../providers";
import { connected } from "../signals";
import * as S from "../state";
import { ConfirmDialog, requestConfirm } from "../ui";

// ── Types ───────────────────────────────────────────────────

interface ConfiguredModelEntry {
	id: string;
	provider: string;
	displayName?: string;
	providerDisplayName?: string;
	authType?: string;
	preferred?: boolean;
	recommended?: boolean;
	createdAt?: number | null;
	disabled?: boolean;
	unsupported?: boolean;
	unsupportedReason?: string | null;
	supportsTools?: boolean;
	providerOnly?: boolean;
	[key: string]: unknown;
}

interface ProviderMetaEntry {
	name: string;
	displayName?: string;
	authType?: string;
	configured?: boolean;
	model?: string;
	uiOrder?: number;
	[key: string]: unknown;
}

interface ProviderGroup {
	provider: string;
	providerDisplayName: string;
	authType: string;
	selectedModel: string | null;
	models: ConfiguredModelEntry[];
}

interface DetectProgressData {
	total: number;
	checked: number;
	supported: number;
	unsupported: number;
	errors: number;
}

interface DetectSummaryData {
	total?: number;
	checked?: number;
	supported?: number;
	unsupported?: number;
	errors?: number;
}

interface TestResult {
	provider: string;
	ok: boolean;
	error?: string;
}

// ── Signals ─────────────────────────────────────────────────

const configuredModels = signal<ConfiguredModelEntry[]>([]);
const providerMetaSig = signal<Map<string, ProviderMetaEntry>>(new Map());
const loading = signal(false);
const detectingModels = signal(false);
const detectSummary = signal<DetectSummaryData | null>(null);
const detectError = signal("");
const detectProgress = signal<DetectProgressData | null>(null);
const deletingProvider = signal("");
const testingProvider = signal("");
const testResult = signal<TestResult | null>(null);
const providerActionError = signal("");

function countUniqueProviders(models: ConfiguredModelEntry[]): number {
	return new Set(models.map((m) => m.provider)).size;
}

function progressFromPayload(payload: Partial<DetectProgressData> | null | undefined): DetectProgressData {
	return {
		total: payload?.total || 0,
		checked: payload?.checked || 0,
		supported: payload?.supported || 0,
		unsupported: payload?.unsupported || 0,
		errors: payload?.errors || 0,
	};
}

interface ModelsUpdatedEvent {
	phase?: string;
	total?: number;
	checked?: number;
	supported?: number;
	unsupported?: number;
	errors?: number;
	summary?: DetectSummaryData & DetectProgressData;
	error?: string;
}

function handleModelsUpdatedEvent(payload: unknown): void {
	const data = payload as ModelsUpdatedEvent | null;
	if (!data?.phase) return;
	if (data.phase === "start") {
		detectingModels.value = true;
		detectError.value = "";
		detectSummary.value = null;
		detectProgress.value = progressFromPayload(data);
		return;
	}
	if (data.phase === "progress") {
		detectingModels.value = true;
		detectProgress.value = progressFromPayload(data);
		return;
	}
	if (data.phase === "complete") {
		detectingModels.value = false;
		if (data.summary) {
			detectSummary.value = data.summary;
			detectProgress.value = progressFromPayload(data.summary);
		}
		return;
	}
	if (data.phase === "cancelled") {
		detectingModels.value = false;
		detectError.value = t("providers:detectionCancelled");
		if (data.summary) {
			detectSummary.value = data.summary;
			detectProgress.value = progressFromPayload(data.summary);
		}
		return;
	}
	if (data.phase === "error") {
		detectingModels.value = false;
		detectError.value = data.error || t("providers:modelDetectionFailed");
	}
}

function fetchProviders(): Promise<void> {
	loading.value = true;
	testResult.value = null;
	return Promise.all([sendRpc("models.list_all", {}), sendRpc("providers.available", {})])
		.then(([modelsRes, providersRes]) => {
			loading.value = false;
			const providerMeta = new Map<string, ProviderMetaEntry>();
			let configuredProviders: ProviderMetaEntry[] = [];
			if (providersRes?.ok) {
				configuredProviders = ((providersRes.payload as ProviderMetaEntry[]) || []).filter(
					(configuredProvider) => configuredProvider.configured,
				);
				for (const providerMetaEntry of (providersRes.payload as ProviderMetaEntry[]) || []) {
					providerMeta.set(providerMetaEntry.name, providerMetaEntry);
				}
			}
			providerMetaSig.value = providerMeta;

			let models: ConfiguredModelEntry[] = [];
			if (modelsRes?.ok) {
				models = ((modelsRes.payload as ConfiguredModelEntry[]) || []).map((m) => ({
					...m,
					providerDisplayName: providerMeta.get(m.provider)?.displayName || m.provider,
					authType: providerMeta.get(m.provider)?.authType || "api-key",
				}));
			}

			// Include configured providers that don't currently expose a model.
			const modelProviders = new Set(models.map((m) => m.provider));
			const providerOnlyRows: ConfiguredModelEntry[] = configuredProviders
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

async function runDetectAllModels(): Promise<void> {
	if (!connected.value || detectingModels.value) return;
	detectingModels.value = true;
	detectSummary.value = null;
	detectError.value = "";
	detectProgress.value = null;

	try {
		// Phase 1: show current full list first before probing.
		await Promise.all([fetchModels(), fetchProviders()]);
		await new Promise<void>((resolve) => {
			requestAnimationFrame(() => resolve());
		});

		const res = await sendRpc("models.detect_supported", {});
		if (!res?.ok) {
			detectError.value = res?.error?.message || t("providers:failedToDetectModels");
			detectingModels.value = false;
			return;
		}
		interface DetectSupportedPayload extends DetectSummaryData {
			skipped?: boolean;
		}

		const resPayload = res.payload as DetectSupportedPayload | undefined;
		if (resPayload?.skipped) {
			detectingModels.value = false;
			return;
		}
		detectSummary.value = resPayload || null;
		detectProgress.value = progressFromPayload(resPayload);
		await Promise.all([fetchModels(), fetchProviders()]);
		const p = detectProgress.value;
		if (!p || p.total === 0 || p.checked >= p.total) {
			detectingModels.value = false;
		}
	} catch {
		detectingModels.value = false;
	}
}

async function cancelDetection(): Promise<void> {
	const res = await sendRpc("models.cancel_detect", {});
	if (!res?.ok) {
		detectError.value = res?.error?.message || t("providers:modelDetectionFailed");
	}
}

function groupProviderRows(models: ConfiguredModelEntry[], metaMap: Map<string, ProviderMetaEntry>): ProviderGroup[] {
	const groups = new Map<string, ProviderGroup>();
	for (const row of models) {
		const key = row.provider;
		if (!groups.has(key)) {
			groups.set(key, {
				provider: key,
				providerDisplayName: row.providerDisplayName || row.displayName || key,
				authType: row.authType || "api-key",
				selectedModel: metaMap?.get(key)?.model || null,
				models: [],
			});
		}
		const groupEntry = groups.get(key)!;
		if (!row.providerOnly) {
			groupEntry.models.push(row);
		}
	}

	const result = Array.from(groups.values());
	result.sort((a, b) => {
		const aOrder = metaMap?.get(a.provider)?.uiOrder;
		const bOrder = metaMap?.get(b.provider)?.uiOrder;
		const hasAOrder = Number.isFinite(aOrder);
		const hasBOrder = Number.isFinite(bOrder);
		if (hasAOrder && hasBOrder && aOrder !== bOrder) return aOrder! - bOrder!;
		if (hasAOrder && !hasBOrder) return -1;
		if (!hasAOrder && hasBOrder) return 1;
		return a.providerDisplayName.localeCompare(b.providerDisplayName);
	});
	for (const providerGroup of result) {
		providerGroup.models.sort((a, b) => {
			// Preferred > recommended > newest date > highest version number > alpha.
			const aPref = a.preferred ? 1 : 0;
			const bPref = b.preferred ? 1 : 0;
			if (aPref !== bPref) return bPref - aPref;
			const aRec = a.recommended ? 1 : 0;
			const bRec = b.recommended ? 1 : 0;
			if (aRec !== bRec) return bRec - aRec;
			const aTime = a.createdAt || 0;
			const bTime = b.createdAt || 0;
			if (aTime !== bTime) return bTime - aTime;
			const aVer = modelVersionScore(a.id);
			const bVer = modelVersionScore(b.id);
			if (aVer !== bVer) return bVer - aVer;
			return (a.displayName || a.id).localeCompare(b.displayName || b.id);
		});
	}
	return result;
}

const DEFAULT_VISIBLE_MODELS = 3;

function ProviderSection({ group }: { group: ProviderGroup }): VNode {
	const [expanded, setExpanded] = useState(false);
	const hasMore = group.models.length > DEFAULT_VISIBLE_MODELS;
	const visibleModels = expanded || !hasMore ? group.models : group.models.slice(0, DEFAULT_VISIBLE_MODELS);
	const hiddenCount = group.models.length - DEFAULT_VISIBLE_MODELS;

	function onDeleteProvider(): void {
		if (deletingProvider.value) return;
		requestConfirm(t("providers:removeProviderConfirm", { name: group.providerDisplayName })).then((yes) => {
			if (!yes) return;
			deletingProvider.value = group.provider;
			providerActionError.value = "";
			sendRpc("providers.remove_key", { provider: group.provider })
				.then((res) => {
					if (res?.ok) {
						if (testResult.value?.provider === group.provider) testResult.value = null;
						configuredModels.value = configuredModels.value.filter((entry) => entry.provider !== group.provider);
						fetchModels();
						fetchProviders();
						return;
					}
					providerActionError.value = res?.error?.message || t("providers:failedToDeleteProvider");
				})
				.catch(() => {
					providerActionError.value = t("providers:failedToDeleteProvider");
				})
				.finally(() => {
					deletingProvider.value = "";
				});
		});
	}

	function onToggleModel(model: ConfiguredModelEntry): void {
		const method = model.disabled ? "models.enable" : "models.disable";
		sendRpc(method, { modelId: model.id }).then((res) => {
			if (res?.ok) {
				providerActionError.value = "";
				configuredModels.value = configuredModels.value.map((entry) =>
					entry.id === model.id ? { ...entry, disabled: !model.disabled } : entry,
				);
				fetchModels();
				fetchProviders();
			} else {
				providerActionError.value = res?.error?.message || t("providers:failedToUpdateModel");
			}
		});
	}

	function onSelectModels(): void {
		openModelSelectorForProvider(group.provider, group.providerDisplayName);
	}

	function onTestProvider(): void {
		if (testingProvider.value || group.models.length === 0) return;
		const firstModel = group.models[0];
		requestConfirm(t("providers:testProviderConfirm", { name: group.providerDisplayName })).then((yes) => {
			if (!yes) return;
			testingProvider.value = group.provider;
			testResult.value = null;
			providerActionError.value = "";
			testModel(firstModel.id)
				.then((res) => {
					if (res.ok) {
						testResult.value = { provider: group.provider, ok: true };
					} else {
						testResult.value = { provider: group.provider, ok: false, error: res.error };
					}
				})
				.catch(() => {
					testResult.value = {
						provider: group.provider,
						ok: false,
						error: t("providers:testFailed"),
					};
				})
				.finally(() => {
					testingProvider.value = "";
				});
		});
	}

	const isTesting = testingProvider.value === group.provider;
	const providerTestResult = testResult.value?.provider === group.provider ? testResult.value : null;

	return (
		<div id={`provider-${group.provider}`} className="max-w-form py-1">
			<div className="flex items-center justify-between gap-3">
				<div className="flex items-center gap-2 min-w-0">
					<h3 className="text-base font-semibold text-[var(--text-strong)] truncate">{group.providerDisplayName}</h3>
					<span className={`provider-item-badge ${group.authType}`}>
						{group.authType === "oauth"
							? t("providers:oauth")
							: group.authType === "local"
								? t("providers:local")
								: t("providers:apiKey")}
					</span>
				</div>
				<div className="flex gap-2 shrink-0">
					{group.models.length > 0 ? (
						<button
							className="provider-btn provider-btn-secondary provider-btn-sm"
							disabled={isTesting}
							onClick={onTestProvider}
						>
							{isTesting ? t("providers:testing") : t("providers:test")}
						</button>
					) : null}
					{group.models.length > 0 ? (
						<button className="provider-btn provider-btn-secondary provider-btn-sm" onClick={onSelectModels}>
							{t("providers:preferredModels.button")}
						</button>
					) : null}
					<button
						className="provider-btn provider-btn-danger provider-btn-sm"
						disabled={deletingProvider.value === group.provider}
						onClick={onDeleteProvider}
					>
						{deletingProvider.value === group.provider ? t("common:status.deleting") : t("common:actions.delete")}
					</button>
				</div>
			</div>
			{providerTestResult ? (
				<div
					className={`mt-1 text-xs ${providerTestResult.ok ? "text-[var(--success,#22c55e)]" : "text-[var(--danger,#ef4444)]"}`}
				>
					{providerTestResult.ok ? t("providers:testSuccess") : providerTestResult.error}
				</div>
			) : null}
			<div className="mt-2 border-b border-[var(--border)]" />
			{group.models.length === 0 ? (
				<div className="mt-2 text-xs text-[var(--muted)]">{t("providers:noActiveModels")}</div>
			) : (
				<div className="mt-2 flex flex-col gap-2">
					{visibleModels.map((model) => (
						<div key={model.id} className="flex items-start justify-between gap-3 py-1">
							<div className="min-w-0 flex-1">
								<div className="flex items-center gap-2 min-w-0">
									<div className="text-sm font-medium text-[var(--text-strong)] truncate">
										{model.displayName || model.id}
									</div>
									{model.preferred ? <span className="recommended-badge">{t("providers:preferred")}</span> : null}
									{model.unsupported ? (
										<span
											className="provider-item-badge warning"
											title={model.unsupportedReason || t("providers:modelNotSupported")}
										>
											{t("providers:unsupported")}
										</span>
									) : null}
									{model.supportsTools ? null : (
										<span className="provider-item-badge warning">{t("providers:chatOnly")}</span>
									)}
									{model.disabled ? <span className="provider-item-badge muted">{t("providers:disabled")}</span> : null}
								</div>
								<div className="mt-1 text-xs text-[var(--muted)] font-mono opacity-75">{model.id}</div>
								{model.unsupported && model.unsupportedReason ? (
									<div className="mt-0.5 text-xs font-medium text-[var(--danger,#ef4444)]">
										{model.unsupportedReason}
									</div>
								) : null}
								{model.createdAt ? (
									<time
										className="mt-0.5 text-xs text-[var(--muted)] opacity-60 block"
										data-epoch-ms={model.createdAt * 1000}
										data-format="year-month"
									/>
								) : null}
							</div>
							<button
								className="provider-btn provider-btn-secondary provider-btn-sm"
								onClick={() => onToggleModel(model)}
							>
								{model.disabled ? t("common:actions.enable") : t("common:actions.disable")}
							</button>
						</div>
					))}
					{hasMore ? (
						<button
							className="text-xs text-[var(--accent)] cursor-pointer bg-transparent border-none py-1 text-left hover:underline"
							onClick={() => setExpanded(!expanded)}
						>
							{expanded ? t("providers:showFewerModels") : t("providers:showAllModels", { count: hiddenCount })}
						</button>
					) : null}
				</div>
			)}
		</div>
	);
}

function ProvidersPageComponent(): VNode {
	useEffect(() => {
		if (connected.value) fetchProviders();
		const offModelsUpdated = onEvent("models.updated", handleModelsUpdatedEvent);

		return () => {
			offModelsUpdated();
		};
	}, [connected.value]);

	S.setRefreshProvidersPage(fetchProviders);

	const progressValue = detectProgress.value || { total: 0, checked: 0, supported: 0, unsupported: 0, errors: 0 };
	const progressPercent = progressValue.total > 0 ? Math.round((progressValue.checked / progressValue.total) * 100) : 0;

	return (
		<>
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<div className="flex items-center gap-3">
					<h2 id="providersTitle" className="text-lg font-medium text-[var(--text-strong)]">
						{t("providers:title")}
					</h2>
					<button
						id="providersAddLlmBtn"
						data-testid="providers-add-llm"
						className="provider-btn"
						onClick={() => {
							if (connected.value) openProviderModal();
						}}
					>
						{t("providers:addLlm")}
					</button>
					<button
						id="providersDetectModelsBtn"
						data-testid="providers-detect-models"
						className="provider-btn provider-btn-secondary"
						disabled={!connected.value || detectingModels.value}
						onClick={runDetectAllModels}
					>
						{detectingModels.value ? t("providers:detectingModels") : t("providers:detectAllModels")}
					</button>
				</div>
				<p className="text-xs text-[var(--muted)] leading-relaxed max-w-form" style={{ margin: 0 }}>
					{t("providers:description")}
				</p>
				{detectError.value || providerActionError.value ? (
					<div className="text-xs text-[var(--danger,#ef4444)] max-w-form">
						{detectError.value || providerActionError.value}
					</div>
				) : null}
				{detectingModels.value ? (
					<div className="max-w-form">
						<div className="flex items-center gap-2">
							<div className="flex-1 h-2 overflow-hidden rounded-sm border border-[var(--border)] bg-[var(--surface2)]">
								<div
									className="h-full bg-[var(--accent)] transition-all duration-150"
									style={{ width: `${progressPercent}%` }}
								/>
							</div>
							<button className="provider-btn provider-btn-danger provider-btn-sm" onClick={cancelDetection}>
								{t("providers:stopDetection")}
							</button>
						</div>
						<div className="mt-1 text-xs text-[var(--muted)]">
							{t("providers:probingModels", {
								checked: progressValue.checked,
								total: progressValue.total,
								pct: progressPercent,
							})}
						</div>
					</div>
				) : detectSummary.value ? (
					<div className="text-xs text-[var(--muted)] max-w-form">
						{t("providers:detectSummary", {
							supported: detectSummary.value.supported || 0,
							unsupported: detectSummary.value.unsupported || 0,
							total: detectSummary.value.total || 0,
						})}
					</div>
				) : null}

				{(() => {
					const groups = groupProviderRows(configuredModels.value, providerMetaSig.value);
					if (loading.value && configuredModels.value.length === 0) {
						return (
							<div id="providersLoadingState" className="text-xs text-[var(--muted)]">
								{t("common:status.loading")}
							</div>
						);
					}
					if (configuredModels.value.length === 0) {
						return (
							<div
								id="providersEmptyState"
								data-testid="providers-empty-state"
								className="text-xs text-[var(--muted)]"
								style={{ padding: "12px 0" }}
							>
								{t("providers:noProvidersConfigured")}
							</div>
						);
					}
					return (
						<div id="providersConfiguredList" data-testid="providers-configured-list" style={{ maxWidth: "600px" }}>
							{groups.length > 1 ? (
								<div className="flex flex-wrap gap-1 mb-3">
									{groups.map((g) => (
										<button
											key={g.provider}
											className="text-xs px-2 py-1 rounded-md border border-[var(--border)] bg-[var(--surface)] text-[var(--muted)] hover:text-[var(--text)] hover:border-[var(--border-strong)] cursor-pointer"
											onClick={() => {
												const el = document.getElementById(`provider-${g.provider}`);
												if (el)
													el.scrollIntoView({
														behavior: "smooth",
														block: "start",
													});
											}}
										>
											{g.providerDisplayName}
											<span className="ml-1 opacity-60">{g.models.length}</span>
										</button>
									))}
								</div>
							) : null}
							<div
								style={{
									display: "flex",
									flexDirection: "column",
									gap: "6px",
									marginBottom: "12px",
								}}
							>
								{groups.map((g) => (
									<ProviderSection key={g.provider} group={g} />
								))}
							</div>
						</div>
					);
				})()}
			</div>
			<ConfirmDialog />
		</>
	);
}

let _providersContainer: HTMLElement | null = null;

export function initProviders(container: HTMLElement): void {
	_providersContainer = container;
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	render(<ProvidersPageComponent />, container);
}

export function teardownProviders(): void {
	S.setRefreshProvidersPage(null);
	if (_providersContainer) render(null, _providersContainer);
	_providersContainer = null;
}

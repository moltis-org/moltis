// ── Provider step (provider config, model selection) ─────────

import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { modelVersionScore, sendRpc } from "../../helpers";
import { t } from "../../i18n";
import { providerApiKeyHelp } from "../../provider-key-help";
import { completeProviderOAuth, startProviderOAuth } from "../../provider-oauth";
import {
	humanizeProbeError,
	isModelServiceNotConfigured,
	saveProviderKey,
	testModel,
	validateProviderKey,
} from "../../provider-validation";
import { targetValue } from "../../typed-events";
import { ErrorPanel } from "../shared";
import type {
	KeyHelp,
	LocalModel,
	ModelSelectorRow,
	OAuthInfo,
	ProbeResult,
	ProviderInfo,
	RawModelRow,
	SysInfo,
	ValidationResult,
} from "../types";

// ── Constants ───────────────────────────────────────────────

const OPENAI_COMPATIBLE = ["openai", "mistral", "openrouter", "cerebras", "minimax", "moonshot", "venice", "ollama"];
const BYOM_PROVIDERS = ["venice"];
const RECOMMENDED_PROVIDERS = new Set([
	"anthropic",
	"openai",
	"gemini",
	"deepseek",
	"minimax",
	"zai",
	"ollama",
	"local-llm",
	"lmstudio",
]);

const WS_RETRY_LIMIT = 75;
const WS_RETRY_DELAY_MS = 200;

// ── Helper functions ────────────────────────────────────────

export function sortProviders(list: ProviderInfo[]): ProviderInfo[] {
	list.sort((a, b) => {
		const aOrder = Number.isFinite(a.uiOrder) ? (a.uiOrder as number) : Number.MAX_SAFE_INTEGER;
		const bOrder = Number.isFinite(b.uiOrder) ? (b.uiOrder as number) : Number.MAX_SAFE_INTEGER;
		if (aOrder !== bOrder) return aOrder - bOrder;
		return a.displayName.localeCompare(b.displayName);
	});
	return list;
}

function normalizeProviderToken(value: string | undefined): string {
	return String(value || "")
		.toLowerCase()
		.replace(/[^a-z0-9]/g, "");
}

function normalizeModelToken(value: string | undefined): string {
	return String(value || "")
		.trim()
		.toLowerCase();
}

function stripModelNamespace(modelId: string | undefined): string {
	if (!modelId || typeof modelId !== "string") return "";
	const sep = modelId.lastIndexOf("::");
	return sep >= 0 ? modelId.slice(sep + 2) : modelId;
}

export function resolveSavedModelSelection(
	savedModels: string[] | undefined,
	availableModels: ModelSelectorRow[],
): Set<string> {
	const selected = new Set<string>();
	if (!(savedModels?.length && savedModels.length > 0) || availableModels.length === 0) return selected;

	const exactIdLookup = new Map<string, string>();
	const rawIdLookup = new Map<string, string>();
	for (const mdl of availableModels) {
		const id = String(mdl?.id || "").trim();
		if (!id) continue;
		exactIdLookup.set(normalizeModelToken(id), id);
		const rawId = normalizeModelToken(stripModelNamespace(id));
		if (rawId && !rawIdLookup.has(rawId)) rawIdLookup.set(rawId, id);
	}

	for (const savedModel of savedModels) {
		const savedNorm = normalizeModelToken(savedModel);
		if (!savedNorm) continue;
		const exact = exactIdLookup.get(savedNorm);
		if (exact) {
			selected.add(exact);
			continue;
		}
		const raw = normalizeModelToken(stripModelNamespace(savedModel));
		const mapped = rawIdLookup.get(raw);
		if (mapped) selected.add(mapped);
	}
	return selected;
}

function modelBelongsToProvider(providerName: string, mdl: ModelSelectorRow): boolean {
	const needle = normalizeProviderToken(providerName);
	if (!needle) return false;
	const modelProvider = normalizeProviderToken(mdl?.provider);
	if (modelProvider?.includes(needle)) return true;
	const modelId = String(mdl?.id || "");
	const modelPrefix = normalizeProviderToken(modelId.split("::")[0]);
	return modelPrefix === needle;
}

function toModelSelectorRow(modelRow: RawModelRow): ModelSelectorRow {
	return {
		id: modelRow.id,
		displayName: modelRow.displayName || modelRow.id,
		provider: modelRow.provider,
		supportsTools: modelRow.supportsTools,
		createdAt: modelRow.createdAt || 0,
	};
}

// ── ModelSelectCard ─────────────────────────────────────────

export function ModelSelectCard({
	model,
	selected,
	probe,
	onToggle,
}: {
	model: ModelSelectorRow;
	selected: boolean;
	probe: string | ProbeResult | undefined;
	onToggle: () => void;
}): VNode {
	const probeError = probe && probe !== "ok" && probe !== "probing" ? (probe as ProbeResult).error || "" : "";
	return (
		<div className={`model-card ${selected ? "selected" : ""}`} onClick={onToggle}>
			<div className="flex flex-wrap items-center justify-between gap-2">
				<span className="text-sm font-medium text-[var(--text)]">{model.displayName}</span>
				<div className="flex flex-wrap gap-2 justify-end">
					{model.supportsTools ? <span className="recommended-badge">Tools</span> : null}
					{probe === "probing" ? <span className="tier-badge">Probing{"\u2026"}</span> : null}
					{probeError ? <span className="provider-item-badge warning">Unsupported</span> : null}
				</div>
			</div>
			<div className="text-xs text-[var(--muted)] mt-1 font-mono">{model.id}</div>
			{probeError ? <div className="text-xs font-medium text-[var(--danger,#ef4444)] mt-0.5">{probeError}</div> : null}
			{model.createdAt ? (
				<time
					className="text-xs text-[var(--muted)] mt-0.5 opacity-60 block"
					data-epoch-ms={model.createdAt * 1000}
					data-format="year-month"
				/>
			) : null}
		</div>
	);
}

// ── OnboardingProviderRow ───────────────────────────────────

interface OnboardingProviderRowProps {
	provider: ProviderInfo;
	configuring: string | null;
	phase: string;
	providerModels: ModelSelectorRow[];
	selectedModels: Set<string>;
	probeResults: Map<string, string | ProbeResult>;
	modelSearch: string;
	setModelSearch: (v: string) => void;
	oauthProvider: string | null;
	oauthInfo: OAuthInfo | null;
	oauthCallbackInput: string;
	setOauthCallbackInput: (v: string) => void;
	oauthSubmitting: boolean;
	localProvider: string | null;
	sysInfo: SysInfo | null;
	localModels: LocalModel[];
	selectedBackend: string | null;
	setSelectedBackend: (v: string) => void;
	apiKey: string;
	setApiKey: (v: string) => void;
	endpoint: string;
	setEndpoint: (v: string) => void;
	model: string;
	setModel: (v: string) => void;
	saving: boolean;
	savingModels: boolean;
	error: string | null;
	validationResult: ValidationResult | null;
	onStartConfigure: (name: string) => void;
	onCancelConfigure: () => void;
	onSaveKey: (e: Event) => void;
	onToggleModel: (id: string) => void;
	onSaveModels: () => void;
	onSubmitOAuthCallback: (name: string) => void;
	onCancelOAuth: () => void;
	onConfigureLocalModel: (mdl: LocalModel) => void;
	onCancelLocal: () => void;
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: provider row renders inline config forms for api-key, oauth, and local flows
export function OnboardingProviderRow(props: OnboardingProviderRowProps): VNode {
	const {
		provider,
		configuring,
		phase,
		providerModels,
		selectedModels,
		probeResults,
		modelSearch,
		setModelSearch,
		oauthProvider,
		oauthInfo,
		oauthCallbackInput,
		setOauthCallbackInput,
		oauthSubmitting,
		localProvider,
		sysInfo,
		localModels,
		selectedBackend,
		setSelectedBackend,
		apiKey,
		setApiKey,
		endpoint,
		setEndpoint,
		model,
		setModel,
		saving,
		savingModels,
		error,
		validationResult,
		onStartConfigure,
		onCancelConfigure,
		onSaveKey,
		onToggleModel,
		onSaveModels,
		onSubmitOAuthCallback,
		onCancelOAuth,
		onConfigureLocalModel,
		onCancelLocal,
	} = props;

	const isApiKeyForm = configuring === provider.name && (phase === "form" || phase === "validating");
	const isModelSelect = configuring === provider.name && phase === "selectModel";
	const isOAuth = oauthProvider === provider.name;
	const isLocal = localProvider === provider.name;
	const isExpanded = isApiKeyForm || isModelSelect || isOAuth || isLocal;
	const keyInputRef = useRef<HTMLInputElement>(null);
	const rowRef = useRef<HTMLDivElement>(null);

	useEffect(() => {
		if (isApiKeyForm && keyInputRef.current) keyInputRef.current.focus();
	}, [isApiKeyForm]);

	useEffect(() => {
		if (isExpanded && rowRef.current) rowRef.current.scrollIntoView({ behavior: "smooth", block: "nearest" });
	}, [isExpanded]);

	const supportsEndpoint = OPENAI_COMPATIBLE.includes(provider.name);
	const needsModel = BYOM_PROVIDERS.includes(provider.name);
	const keyHelp = providerApiKeyHelp(provider) as KeyHelp | null;

	const [showAllModels, setShowAllModels] = useState(false);
	const DEFAULT_VISIBLE = 3;

	const sortedModels = (providerModels || []).slice().sort((a, b) => {
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

	const filteredModels = sortedModels.filter(
		(m) =>
			!modelSearch ||
			m.displayName.toLowerCase().includes(modelSearch.toLowerCase()) ||
			m.id.toLowerCase().includes(modelSearch.toLowerCase()),
	);

	const hasMoreModels = filteredModels.length > DEFAULT_VISIBLE && !modelSearch;
	const visibleModels = showAllModels || modelSearch ? filteredModels : filteredModels.slice(0, DEFAULT_VISIBLE);
	const hiddenModelCount = filteredModels.length - DEFAULT_VISIBLE;

	return (
		<div ref={rowRef} className="rounded-md border border-[var(--border)] bg-[var(--surface)] p-3">
			<div className="flex items-center gap-3">
				<div className="flex-1 min-w-0 flex flex-col gap-0.5">
					<div className="flex items-center gap-2 flex-wrap">
						<span className="text-sm font-medium text-[var(--text-strong)]">{provider.displayName}</span>
						{provider.configured ? <span className="provider-item-badge configured">configured</span> : null}
						{validationResult?.ok === true ? (
							<span className="icon icon-md icon-check-circle inline-block" style={{ color: "var(--ok)" }} />
						) : null}
						<span className={`provider-item-badge ${provider.authType}`}>
							{provider.authType === "oauth" ? "OAuth" : provider.authType === "local" ? "Local" : "API Key"}
						</span>
					</div>
				</div>
				<div className="shrink-0">
					{isExpanded ? null : (
						<button
							className="provider-btn provider-btn-secondary provider-btn-sm"
							onClick={() => onStartConfigure(provider.name)}
						>
							{provider.configured ? "Choose Model" : "Configure"}
						</button>
					)}
				</div>
			</div>
			{validationResult?.ok === false && !isExpanded ? (
				<div className="text-xs text-[var(--warning)] mt-1">{validationResult.message}</div>
			) : null}
			{isApiKeyForm ? (
				<form onSubmit={onSaveKey} className="flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3">
					<div>
						<label className="text-xs text-[var(--muted)] mb-1 block">API Key</label>
						<input
							type="password"
							className="provider-key-input w-full"
							ref={keyInputRef}
							value={apiKey}
							onInput={(e) => setApiKey(targetValue(e))}
							placeholder={provider.keyOptional ? "(optional)" : "sk-..."}
						/>
						{keyHelp ? (
							<div className="text-xs text-[var(--muted)] mt-1">
								{keyHelp.url ? (
									<>
										{keyHelp.text}{" "}
										<a
											href={keyHelp.url}
											target="_blank"
											rel="noopener noreferrer"
											className="text-[var(--accent)] underline"
										>
											{keyHelp.label || keyHelp.url}
										</a>
									</>
								) : (
									keyHelp.text
								)}
							</div>
						) : null}
					</div>
					{supportsEndpoint ? (
						<div>
							<label className="text-xs text-[var(--muted)] mb-1 block">Endpoint (optional)</label>
							<input
								type="text"
								className="provider-key-input w-full"
								value={endpoint}
								onInput={(e) => setEndpoint(targetValue(e))}
								placeholder={provider.defaultBaseUrl || "https://api.example.com/v1"}
							/>
							<div className="text-xs text-[var(--muted)] mt-1">Leave empty to use the default endpoint.</div>
						</div>
					) : null}
					{needsModel ? (
						<div>
							<label className="text-xs text-[var(--muted)] mb-1 block">Model ID</label>
							<input
								type="text"
								className="provider-key-input w-full"
								value={model}
								onInput={(e) => setModel(targetValue(e))}
								placeholder="model-id"
							/>
						</div>
					) : null}
					{error ? <ErrorPanel message={error} /> : null}
					<div className="flex items-center gap-2 mt-1">
						<button
							key={`prov-${phase}`}
							type="submit"
							className="provider-btn provider-btn-sm"
							disabled={phase === "validating"}
						>
							{phase === "validating" ? "Saving\u2026" : "Save"}
						</button>
						<button
							type="button"
							className="provider-btn provider-btn-secondary provider-btn-sm"
							onClick={onCancelConfigure}
							disabled={phase === "validating"}
						>
							Cancel
						</button>
					</div>
					{phase === "validating" ? (
						<div className="text-xs text-[var(--muted)] mt-1">Discovering available models{"\u2026"}</div>
					) : null}
				</form>
			) : null}
			{isModelSelect ? (
				<div className="flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3">
					<div className="text-xs font-medium text-[var(--text-strong)]">Select preferred models</div>
					<div className="text-xs text-[var(--muted)]">Selected models appear first in the session model selector.</div>
					{(providerModels || []).length > 5 ? (
						<input
							type="text"
							className="provider-key-input w-full text-xs"
							placeholder={"Search models\u2026"}
							value={modelSearch}
							onInput={(e) => setModelSearch(targetValue(e))}
						/>
					) : null}
					<div className="flex flex-col gap-1">
						{visibleModels.length === 0 ? (
							<div className="text-xs text-[var(--muted)] py-4 text-center">No models match your search.</div>
						) : (
							visibleModels.map((m) => (
								<ModelSelectCard
									key={m.id}
									model={m}
									selected={selectedModels.has(m.id)}
									probe={probeResults.get(m.id)}
									onToggle={() => onToggleModel(m.id)}
								/>
							))
						)}
						{hasMoreModels ? (
							<button
								className="text-xs text-[var(--accent)] cursor-pointer bg-transparent border-none py-1 text-left hover:underline"
								onClick={() => setShowAllModels(!showAllModels)}
							>
								{showAllModels
									? t("providers:showFewerModels")
									: t("providers:showAllModels", { count: hiddenModelCount })}
							</button>
						) : null}
					</div>
					<div className="text-xs text-[var(--muted)]">
						{selectedModels.size === 0
							? "No models selected"
							: `${selectedModels.size} model${selectedModels.size > 1 ? "s" : ""} selected`}
					</div>
					{error ? <ErrorPanel message={error} /> : null}
					<div className="flex items-center gap-2 mt-1">
						<button
							type="button"
							className="provider-btn provider-btn-sm"
							disabled={selectedModels.size === 0 || savingModels}
							onClick={onSaveModels}
						>
							{savingModels ? "Saving\u2026" : "Save"}
						</button>
						<button
							type="button"
							className="provider-btn provider-btn-secondary provider-btn-sm"
							onClick={onCancelConfigure}
							disabled={savingModels}
						>
							Cancel
						</button>
					</div>
					{savingModels ? (
						<div className="text-xs text-[var(--muted)] mt-1">
							Saving credentials and validating selected models{"\u2026"}
						</div>
					) : null}
				</div>
			) : null}
			{isOAuth ? (
				<div className="flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3">
					{oauthInfo?.status === "device" ? (
						<div className="text-sm text-[var(--text)]">
							Open{" "}
							<a href={oauthInfo.uri} target="_blank" className="text-[var(--accent)] underline">
								{oauthInfo.uri}
							</a>{" "}
							and enter code:<strong className="font-mono ml-1">{oauthInfo.code}</strong>
						</div>
					) : (
						<div className="text-sm text-[var(--muted)]">Waiting for authentication{"\u2026"}</div>
					)}
					{oauthInfo?.status === "device" ? null : (
						<>
							<div className="text-xs text-[var(--muted)]">
								If localhost callback fails, paste the redirect URL (or code#state) below.
							</div>
							<input
								type="text"
								className="provider-key-input w-full"
								placeholder="http://localhost:1455/auth/callback?code=...&state=..."
								value={oauthCallbackInput}
								onInput={(event) => setOauthCallbackInput((event.target as HTMLInputElement).value)}
								disabled={oauthSubmitting}
							/>
							<button
								className="provider-btn provider-btn-secondary provider-btn-sm self-start"
								onClick={() => onSubmitOAuthCallback(provider.name)}
								disabled={oauthSubmitting}
							>
								{oauthSubmitting ? "Submitting..." : "Submit Callback"}
							</button>
						</>
					)}
					{error ? <ErrorPanel message={error} /> : null}
					<button className="provider-btn provider-btn-secondary provider-btn-sm self-start" onClick={onCancelOAuth}>
						Cancel
					</button>
				</div>
			) : null}
			{isLocal ? (
				<div className="flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3">
					{sysInfo ? (
						<div className="flex flex-col gap-3">
							<div className="flex gap-3 text-xs text-[var(--muted)]">
								<span>RAM: {sysInfo.totalRamGb}GB</span>
								<span>Tier: {sysInfo.memoryTier}</span>
								{sysInfo.hasGpu ? <span className="text-[var(--ok)]">GPU available</span> : null}
							</div>
							{sysInfo.isAppleSilicon && (sysInfo.availableBackends || []).length > 0 ? (
								<div className="flex flex-col gap-2">
									<div className="text-xs font-medium text-[var(--text-strong)]">Backend</div>
									<div className="flex flex-col gap-2">
										{(sysInfo.availableBackends || []).map((b) => (
											<div
												key={b.id}
												className={`backend-card ${b.id === selectedBackend ? "selected" : ""} ${b.available ? "" : "disabled"}`}
												onClick={() => {
													if (b.available) setSelectedBackend(b.id);
												}}
											>
												<div className="flex flex-wrap items-center justify-between gap-2">
													<span className="text-sm font-medium text-[var(--text)]">{b.name}</span>
													<div className="flex flex-wrap gap-2 justify-end">
														{b.id === sysInfo.recommendedBackend && b.available ? (
															<span className="recommended-badge">Recommended</span>
														) : null}
														{b.available ? null : <span className="tier-badge">Not installed</span>}
													</div>
												</div>
												<div className="text-xs text-[var(--muted)] mt-1">{b.description}</div>
											</div>
										))}
									</div>
								</div>
							) : null}
							<div className="text-xs font-medium text-[var(--text-strong)]">Select a model</div>
							<div className="flex flex-col gap-2">
								{localModels.filter((m) => m.backend === selectedBackend).length === 0 ? (
									<div className="text-xs text-[var(--muted)] py-4 text-center">
										No models available for {selectedBackend}
									</div>
								) : (
									localModels
										.filter((m) => m.backend === selectedBackend)
										.map((mdl) => (
											<div key={mdl.id} className="model-card" onClick={() => onConfigureLocalModel(mdl)}>
												<div className="flex flex-wrap items-center justify-between gap-2">
													<span className="text-sm font-medium text-[var(--text)]">{mdl.displayName}</span>
													<div className="flex flex-wrap gap-2 justify-end">
														<span className="tier-badge">{mdl.minRamGb}GB</span>
														{mdl.suggested ? <span className="recommended-badge">Recommended</span> : null}
													</div>
												</div>
												<div className="text-xs text-[var(--muted)] mt-1">
													Context: {(mdl.contextWindow / 1000).toFixed(0)}k tokens
												</div>
											</div>
										))
								)}
							</div>
							{saving ? <div className="text-xs text-[var(--muted)]">Configuring{"\u2026"}</div> : null}
						</div>
					) : (
						<div className="text-xs text-[var(--muted)]">Loading system info{"\u2026"}</div>
					)}
					{error ? <ErrorPanel message={error} /> : null}
					<button className="provider-btn provider-btn-secondary provider-btn-sm self-start" onClick={onCancelLocal}>
						Cancel
					</button>
				</div>
			) : null}
		</div>
	);
}

// ── ProviderStep ─────────────────────────────────────────────

export function ProviderStep({ onNext, onBack }: { onNext: () => void; onBack?: (() => void) | null }): VNode {
	const [providers, setProviders] = useState<ProviderInfo[]>([]);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const [showAllProviders, setShowAllProviders] = useState(false);
	const [configuring, setConfiguring] = useState<string | null>(null);
	const [oauthProvider, setOauthProvider] = useState<string | null>(null);
	const [localProvider, setLocalProvider] = useState<string | null>(null);
	const [phase, setPhase] = useState("form");
	const [providerModels, setProviderModels] = useState<ModelSelectorRow[]>([]);
	const [selectedModels, setSelectedModels] = useState<Set<string>>(new Set());
	const [probeResults, setProbeResults] = useState<Map<string, string | ProbeResult>>(new Map());
	const [modelSearch, setModelSearch] = useState("");
	const [savingModels, setSavingModels] = useState(false);
	const [modelSelectProvider, setModelSelectProvider] = useState<string | null>(null);
	const [apiKey, setApiKey] = useState("");
	const [endpoint, setEndpoint] = useState("");
	const [model, setModel] = useState("");
	const [saving, setSaving] = useState(false);
	const [validationResults, setValidationResults] = useState<Record<string, ValidationResult>>({});
	const [oauthInfo, setOauthInfo] = useState<OAuthInfo | null>(null);
	const [oauthCallbackInput, setOauthCallbackInput] = useState("");
	const [oauthSubmitting, setOauthSubmitting] = useState(false);
	const oauthTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
	const [sysInfo, setSysInfo] = useState<SysInfo | null>(null);
	const [localModels, setLocalModels] = useState<LocalModel[]>([]);
	const [selectedBackend, setSelectedBackend] = useState<string | null>(null);

	function refreshProviders(): Promise<unknown> {
		return sendRpc<ProviderInfo[]>("providers.available", {}).then((res) => {
			if (res?.ok) setProviders(sortProviders(res.payload || []));
			return res;
		});
	}

	useEffect(() => {
		let cancelled = false;
		let attempts = 0;
		function loadProviders(): void {
			if (cancelled) return;
			sendRpc<ProviderInfo[]>("providers.available", {}).then((res) => {
				if (cancelled) return;
				if (res?.ok) {
					setProviders(sortProviders(res.payload || []));
					setLoading(false);
					return;
				}
				if (
					((res?.error as { code?: string })?.code === "UNAVAILABLE" ||
						(res?.error as { message?: string })?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
					attempts += 1;
					window.setTimeout(loadProviders, WS_RETRY_DELAY_MS);
					return;
				}
				setLoading(false);
			});
		}
		loadProviders();
		return () => {
			cancelled = true;
		};
	}, []);

	useEffect(() => {
		return () => {
			if (oauthTimerRef.current) {
				clearInterval(oauthTimerRef.current);
				oauthTimerRef.current = null;
			}
		};
	}, []);

	function closeAll(): void {
		setConfiguring(null);
		setOauthProvider(null);
		setLocalProvider(null);
		setModelSelectProvider(null);
		setPhase("form");
		setProviderModels([]);
		setSelectedModels(new Set());
		setProbeResults(new Map());
		setModelSearch("");
		setSavingModels(false);
		setApiKey("");
		setEndpoint("");
		setModel("");
		setError(null);
		setOauthInfo(null);
		setOauthCallbackInput("");
		setOauthSubmitting(false);
		setSysInfo(null);
		setLocalModels([]);
		if (oauthTimerRef.current) {
			clearInterval(oauthTimerRef.current);
			oauthTimerRef.current = null;
		}
	}

	async function loadModelsForProvider(providerName: string): Promise<ModelSelectorRow[]> {
		const modelsRes = await sendRpc<RawModelRow[]>("models.list", {});
		const allModels = modelsRes?.ok ? modelsRes.payload || [] : [];
		return allModels.filter((m) => modelBelongsToProvider(providerName, toModelSelectorRow(m))).map(toModelSelectorRow);
	}

	async function openModelSelectForConfiguredApiProvider(provider: ProviderInfo): Promise<boolean> {
		if (provider.authType !== "api-key" || !provider.configured) return false;
		const existingModels = await loadModelsForProvider(provider.name);
		if (existingModels.length === 0) return false;
		const saved = resolveSavedModelSelection(provider.models, existingModels);
		setModelSelectProvider(provider.name);
		setConfiguring(provider.name);
		setProviderModels(existingModels);
		setSelectedModels(saved);
		setPhase("selectModel");
		return true;
	}

	async function onStartConfigure(name: string): Promise<void> {
		closeAll();
		const p = providers.find((pr) => pr.name === name);
		if (!p) return;
		if (p.authType === "api-key") {
			setEndpoint(p.baseUrl || "");
			setModel(p.model || "");
			if (await openModelSelectForConfiguredApiProvider(p)) return;
			setConfiguring(name);
			setPhase("form");
		} else if (p.authType === "oauth") {
			startOAuth(p);
		} else if (p.authType === "local") {
			startLocal(p);
		}
	}

	function onSaveKey(e: Event): void {
		e.preventDefault();
		const p = providers.find((pr) => pr.name === configuring);
		if (!p) return;
		if (!(apiKey.trim() || p.keyOptional)) {
			setError("API key is required.");
			return;
		}
		if (BYOM_PROVIDERS.includes(p.name) && !model.trim()) {
			setError("Model ID is required.");
			return;
		}
		setError(null);
		setPhase("validating");
		const keyVal = apiKey.trim() || p.name;
		const endpointVal = endpoint.trim() || null;
		const modelVal = model.trim() || null;

		validateProviderKey(p.name, keyVal, endpointVal, modelVal)
			.then(async (result: { valid: boolean; error?: string; models?: ModelSelectorRow[] }) => {
				if (!result.valid) {
					setPhase("form");
					setError(result.error || "Validation failed.");
					return;
				}
				if (BYOM_PROVIDERS.includes(p.name)) {
					saveAndFinishByom(p.name, keyVal, endpointVal, modelVal);
					return;
				}
				const saveRes = await saveProviderKey(p.name, keyVal, endpointVal, modelVal);
				if (!saveRes?.ok) {
					setPhase("form");
					setError((saveRes?.error as { message?: string })?.message || "Failed to save credentials.");
					return;
				}
				setProviderModels(result.models || []);
				setPhase("selectModel");
			})
			.catch((err: Error) => {
				setPhase("form");
				setError(err?.message || "Validation failed.");
			});
	}

	function probeModelAsync(modelId: string): void {
		setProbeResults((prev) => {
			const next = new Map(prev);
			next.set(modelId, "probing");
			return next;
		});
		testModel(modelId).then((result: { ok: boolean; error?: string }) => {
			setProbeResults((prev) => {
				const next = new Map(prev);
				if (isModelServiceNotConfigured(result.error || "")) next.delete(modelId);
				else
					next.set(
						modelId,
						result.ok ? "ok" : { error: humanizeProbeError(result.error || "Unsupported") as string | undefined },
					);
				return next;
			});
		});
	}

	function onToggleModel(modelId: string): void {
		setSelectedModels((prev) => {
			const next = new Set(prev);
			if (next.has(modelId)) next.delete(modelId);
			else {
				next.add(modelId);
				probeModelAsync(modelId);
			}
			return next;
		});
	}

	async function onSaveSelectedModels(): Promise<boolean> {
		const providerName = modelSelectProvider || configuring;
		if (!providerName) return false;
		const modelIds = Array.from(selectedModels);
		setSavingModels(true);
		setError(null);
		try {
			if (!modelSelectProvider) {
				const p = providers.find((pr) => pr.name === providerName);
				const keyVal = apiKey.trim() || p?.name || "";
				const endpointVal = endpoint.trim() || null;
				const modelVal = model.trim() || (p?.keyOptional && modelIds.length > 0 ? modelIds[0] : null);
				const res = await saveProviderKey(providerName, keyVal, endpointVal, modelVal);
				if (!res?.ok) {
					setSavingModels(false);
					setError((res?.error as { message?: string })?.message || "Failed to save credentials.");
					return false;
				}
			}
			const res = await sendRpc("providers.save_models", { provider: providerName, models: modelIds });
			if (!res?.ok) {
				setSavingModels(false);
				setError((res?.error as { message?: string })?.message || "Failed to save model preferences.");
				return false;
			}
			if (modelIds.length > 0) localStorage.setItem("moltis-model", modelIds[0]);
			setValidationResults((prev) => ({ ...prev, [providerName]: { ok: true, message: null } }));
			closeAll();
			refreshProviders();
			return true;
		} catch (err) {
			setSavingModels(false);
			setError((err as Error)?.message || "Failed to save credentials.");
			return false;
		}
	}

	async function onContinue(): Promise<void> {
		const hasPendingModelSelection =
			phase === "selectModel" && (configuring || modelSelectProvider) && selectedModels.size > 0;
		if (hasPendingModelSelection) {
			const saved = await onSaveSelectedModels();
			if (!saved) return;
		}
		onNext();
	}

	function saveAndFinishByom(
		providerName: string,
		keyVal: string,
		endpointVal: string | null,
		modelVal: string | null,
	): void {
		saveProviderKey(providerName, keyVal, endpointVal, modelVal)
			.then(async (res: { ok?: boolean; error?: { message?: string } } | null) => {
				if (!res?.ok) {
					setPhase("form");
					setError(res?.error?.message || "Failed to save credentials.");
					return;
				}
				if (modelVal) {
					const testResult = await testModel(modelVal);
					const modelServiceUnavailable = !testResult.ok && isModelServiceNotConfigured(testResult.error || "");
					if (!(testResult.ok || modelServiceUnavailable)) {
						setPhase("form");
						setError(testResult.error || "Model test failed.");
						return;
					}
					await sendRpc("providers.save_models", { provider: providerName, models: [modelVal] });
					localStorage.setItem("moltis-model", modelVal);
				}
				setValidationResults((prev) => ({ ...prev, [providerName]: { ok: true, message: null } }));
				setConfiguring(null);
				setPhase("form");
				setProviderModels([]);
				setSelectedModels(new Set());
				setProbeResults(new Map());
				setModelSearch("");
				setApiKey("");
				setEndpoint("");
				setModel("");
				setError(null);
				refreshProviders();
			})
			.catch((err: Error) => {
				setPhase("form");
				setError(err?.message || "Failed to save credentials.");
			});
	}

	function startOAuth(p: ProviderInfo): void {
		setOauthProvider(p.name);
		setOauthInfo({ status: "starting" });
		setOauthCallbackInput("");
		setOauthSubmitting(false);
		startProviderOAuth(p.name).then(
			(result: { status: string; authUrl?: string; verificationUrl?: string; userCode?: string; error?: string }) => {
				if (result.status === "already") onOAuthAuthenticated(p.name);
				else if (result.status === "browser") {
					window.open(result.authUrl, "_blank");
					setOauthInfo({ status: "waiting" });
					pollOAuth(p);
				} else if (result.status === "device") {
					setOauthInfo({ status: "device", uri: result.verificationUrl, code: result.userCode });
					pollOAuth(p);
				} else {
					setError(result.error || "Failed to start OAuth");
					setOauthProvider(null);
					setOauthInfo(null);
					setOauthCallbackInput("");
					setOauthSubmitting(false);
				}
			},
		);
	}

	async function onOAuthAuthenticated(providerName: string): Promise<void> {
		const provModels = await loadModelsForProvider(providerName);
		setOauthProvider(null);
		setOauthInfo(null);
		setOauthCallbackInput("");
		setOauthSubmitting(false);
		if (provModels.length > 0) {
			setModelSelectProvider(providerName);
			setConfiguring(providerName);
			setProviderModels(provModels);
			setSelectedModels(new Set());
			setPhase("selectModel");
		} else setValidationResults((prev) => ({ ...prev, [providerName]: { ok: true, message: null } }));
		refreshProviders();
	}

	function pollOAuth(p: ProviderInfo): void {
		let attempts = 0;
		if (oauthTimerRef.current) clearInterval(oauthTimerRef.current);
		oauthTimerRef.current = setInterval(() => {
			attempts++;
			if (attempts > 60) {
				clearInterval(oauthTimerRef.current!);
				oauthTimerRef.current = null;
				setError("OAuth timed out.");
				setOauthProvider(null);
				setOauthInfo(null);
				setOauthCallbackInput("");
				setOauthSubmitting(false);
				return;
			}
			sendRpc<{ authenticated?: boolean }>("providers.oauth.status", { provider: p.name }).then((res) => {
				if (res?.ok && res.payload?.authenticated) {
					clearInterval(oauthTimerRef.current!);
					oauthTimerRef.current = null;
					onOAuthAuthenticated(p.name);
				}
			});
		}, 2000);
	}

	function cancelOAuth(): void {
		if (oauthTimerRef.current) {
			clearInterval(oauthTimerRef.current);
			oauthTimerRef.current = null;
		}
		setOauthProvider(null);
		setOauthInfo(null);
		setOauthCallbackInput("");
		setOauthSubmitting(false);
		setError(null);
	}

	function submitOAuthCallback(providerName: string): void {
		const callback = oauthCallbackInput.trim();
		if (!callback) {
			setError("Paste the callback URL (or code#state) to continue.");
			return;
		}
		setOauthSubmitting(true);
		setError(null);
		completeProviderOAuth(providerName, callback)
			.then((res: { ok?: boolean; error?: { message?: string } } | null) => {
				if (res?.ok) {
					if (oauthTimerRef.current) {
						clearInterval(oauthTimerRef.current);
						oauthTimerRef.current = null;
					}
					onOAuthAuthenticated(providerName);
					return;
				}
				setError(res?.error?.message || "Failed to complete OAuth callback.");
			})
			.catch((err: Error) => {
				setError(err?.message || "Failed to complete OAuth callback.");
			})
			.finally(() => {
				setOauthSubmitting(false);
			});
	}

	function startLocal(p: ProviderInfo): void {
		setLocalProvider(p.name);
		sendRpc<SysInfo>("providers.local.system_info", {}).then((sysRes) => {
			if (!sysRes?.ok) {
				setError((sysRes?.error as { message?: string })?.message || "Failed to get system info");
				setLocalProvider(null);
				return;
			}
			setSysInfo(sysRes.payload!);
			setSelectedBackend(sysRes.payload?.recommendedBackend || "GGUF");
			sendRpc<{ recommended?: LocalModel[] }>("providers.local.models", {}).then((modelsRes) => {
				if (modelsRes?.ok) setLocalModels(modelsRes.payload?.recommended || []);
			});
		});
	}

	function configureLocalModel(mdl: LocalModel): void {
		const provName = localProvider;
		setSaving(true);
		setError(null);
		sendRpc("providers.local.configure", { modelId: mdl.id, backend: selectedBackend }).then((res) => {
			setSaving(false);
			if (res?.ok) {
				setLocalProvider(null);
				setSysInfo(null);
				setLocalModels([]);
				setValidationResults((prev) => ({ ...prev, [provName!]: { ok: true, message: null } }));
				refreshProviders();
			} else setError((res?.error as { message?: string })?.message || "Failed to configure model");
		});
	}

	function cancelLocal(): void {
		setLocalProvider(null);
		setSysInfo(null);
		setLocalModels([]);
		setError(null);
	}

	if (loading) return <div className="text-sm text-[var(--muted)]">{t("onboarding:provider.loadingLlms")}</div>;

	const configuredProviders = providers.filter((p) => p.configured);
	const recommendedProviders = providers.filter((p) => RECOMMENDED_PROVIDERS.has(p.name));
	const otherProviders = providers.filter((p) => !RECOMMENDED_PROVIDERS.has(p.name));
	const otherIsActive = otherProviders.some(
		(p) => configuring === p.name || oauthProvider === p.name || localProvider === p.name,
	);
	const showOther = showAllProviders || otherIsActive;

	function renderProviderRow(p: ProviderInfo): VNode {
		return (
			<OnboardingProviderRow
				key={p.name}
				provider={p}
				configuring={configuring}
				phase={configuring === p.name ? phase : "form"}
				providerModels={configuring === p.name ? providerModels : []}
				selectedModels={configuring === p.name ? selectedModels : new Set()}
				probeResults={configuring === p.name ? probeResults : new Map()}
				modelSearch={configuring === p.name ? modelSearch : ""}
				setModelSearch={setModelSearch}
				oauthProvider={oauthProvider}
				oauthInfo={oauthInfo}
				oauthCallbackInput={oauthCallbackInput}
				setOauthCallbackInput={setOauthCallbackInput}
				oauthSubmitting={oauthSubmitting}
				localProvider={localProvider}
				sysInfo={sysInfo}
				localModels={localModels}
				selectedBackend={selectedBackend}
				setSelectedBackend={setSelectedBackend}
				apiKey={apiKey}
				setApiKey={setApiKey}
				endpoint={endpoint}
				setEndpoint={setEndpoint}
				model={model}
				setModel={setModel}
				saving={saving}
				savingModels={savingModels}
				error={configuring === p.name || oauthProvider === p.name || localProvider === p.name ? error : null}
				validationResult={validationResults[p.name] || null}
				onStartConfigure={onStartConfigure}
				onCancelConfigure={closeAll}
				onSaveKey={onSaveKey}
				onToggleModel={onToggleModel}
				onSaveModels={onSaveSelectedModels}
				onSubmitOAuthCallback={submitOAuthCallback}
				onCancelOAuth={cancelOAuth}
				onConfigureLocalModel={configureLocalModel}
				onCancelLocal={cancelLocal}
			/>
		);
	}

	return (
		<div className="flex flex-col gap-4">
			<div className="flex items-baseline justify-between gap-2">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">{t("onboarding:provider.addLlms")}</h2>
				<a
					href="https://docs.moltis.org/choosing-a-provider.html"
					target="_blank"
					rel="noopener noreferrer"
					className="text-xs text-[var(--accent)] hover:underline shrink-0"
				>
					Help me choose
				</a>
			</div>
			<p className="text-xs text-[var(--muted)] leading-relaxed">
				Configure one or more LLM providers to power your agent. You can add more later in Settings.
			</p>
			{configuredProviders.length > 0 ? (
				<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 flex flex-col gap-2">
					<div className="text-xs text-[var(--muted)]">Detected LLM providers</div>
					<div className="flex flex-wrap gap-2">
						{configuredProviders.map((p) => (
							<span key={p.name} className="provider-item-badge configured">
								{p.displayName}
							</span>
						))}
					</div>
				</div>
			) : null}
			<div className="flex flex-col gap-2">
				<div className="text-xs font-medium text-[var(--text)] uppercase tracking-wide">Recommended</div>
				{recommendedProviders.map(renderProviderRow)}
			</div>
			{otherProviders.length > 0 ? (
				<div className="flex flex-col gap-2">
					<button
						type="button"
						className="text-xs text-[var(--muted)] hover:text-[var(--text)] cursor-pointer bg-transparent border-none text-left flex items-center gap-1"
						onClick={() => setShowAllProviders((v) => !v)}
					>
						<span className={`inline-block transition-transform ${showOther ? "rotate-90" : ""}`}>{"\u25B6"}</span>
						All providers ({otherProviders.length} more)
					</button>
					{showOther ? otherProviders.map(renderProviderRow) : null}
				</div>
			) : null}
			{error && !configuring && !oauthProvider && !localProvider ? <ErrorPanel message={error} /> : null}
			<div className="flex flex-wrap items-center gap-3 mt-1">
				<button className="provider-btn provider-btn-secondary" onClick={onBack || undefined}>
					{t("common:actions.back")}
				</button>
				<button className="provider-btn" onClick={onContinue} disabled={phase === "validating" || savingModels}>
					{t("common:actions.continue")}
				</button>
				<button
					className="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
					onClick={onNext}
				>
					{t("common:actions.skip")}
				</button>
			</div>
		</div>
	);
}

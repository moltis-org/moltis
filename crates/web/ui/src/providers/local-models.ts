// ── Local/Ollama model management ────────────────────────────

import { onEvent } from "../events";
import { sendRpc } from "../helpers";
import { fetchModels } from "../models";
import * as S from "../state";
import type { RpcResponse } from "../types";
import { closeProviderModal, els, openProviderModal } from "./shared";
import type {
	BackendInfo,
	HfSearchResult,
	LocalLlmDownloadPayload,
	LocalLlmLifecyclePayload,
	LocalModelInfo,
	ModelSelectorWrapper,
	ModelStateEntry,
	ModelsData,
	ProviderInfo,
	SystemInfo,
} from "./types";

// Store the selected backend for model configuration
let selectedBackend: string | null = null;

// ── Model lifecycle state tracking ──────────────────────────

/** Cached model lifecycle states, keyed by model_id. */
const modelStates: Record<string, ModelStateEntry> = {};

/** Subscribe to lifecycle events. Call once on page load. */
export function initModelLifecycleTracking(): void {
	onEvent("local-llm.lifecycle", (payload: unknown) => {
		const p = payload as LocalLlmLifecyclePayload;
		if (!p.modelId) return;

		const existing = modelStates[p.modelId];
		if (existing) {
			existing.is_loaded = p.state === "loaded";
			existing.memory_bytes = p.modelSizeBytes ?? existing.memory_bytes;
		}
	});

	// Fetch initial state
	refreshModelStates();
}

/** Fetch current model states from the gateway. */
async function refreshModelStates(): Promise<void> {
	const res = await sendRpc<ModelStateEntry[]>("providers.local.model_states", {});
	if (res?.ok && Array.isArray(res.payload)) {
		for (const entry of res.payload) {
			modelStates[entry.model_id] = entry;
		}
	}
}

/** Get the cached lifecycle state for a model. */
export function getModelState(modelId: string): ModelStateEntry | undefined {
	return modelStates[modelId];
}

export function showLocalModelFlow(provider: ProviderInfo): void {
	const m = els();
	m.title.textContent = provider.displayName;
	m.body.textContent = "Loading system info...";

	// Fetch system info first
	sendRpc<SystemInfo>("providers.local.system_info", {}).then((sysRes: RpcResponse<SystemInfo>) => {
		if (!sysRes?.ok) {
			m.body.textContent = sysRes?.error?.message || "Failed to get system info";
			return;
		}
		const sysInfo = sysRes.payload as SystemInfo;

		// Fetch available models
		sendRpc<ModelsData>("providers.local.models", {}).then((modelsRes: RpcResponse<ModelsData>) => {
			if (!modelsRes?.ok) {
				m.body.textContent = modelsRes?.error?.message || "Failed to get models";
				return;
			}
			const modelsData = modelsRes.payload as ModelsData;
			renderLocalModelSelection(provider, sysInfo, modelsData);
		});
	});
}

function renderLocalModelSelection(provider: ProviderInfo, sysInfo: SystemInfo, modelsData: ModelsData): void {
	const m = els();
	m.body.textContent = "";

	// Initialize selected backend to recommended
	selectedBackend = sysInfo.recommendedBackend || "GGUF";

	const wrapper = document.createElement("div") as ModelSelectorWrapper;
	wrapper.className = "provider-key-form";

	// System info section
	const sysSection = document.createElement("div");
	sysSection.className = "flex flex-col gap-2 mb-4";

	const sysTitle = document.createElement("div");
	sysTitle.className = "text-xs font-medium text-[var(--text-strong)]";
	sysTitle.textContent = "System Info";
	sysSection.appendChild(sysTitle);

	const sysDetails = document.createElement("div");
	sysDetails.className = "flex gap-3 text-xs text-[var(--muted)]";

	const ramSpan = document.createElement("span");
	ramSpan.textContent = `RAM: ${sysInfo.totalRamGb}GB`;
	sysDetails.appendChild(ramSpan);

	const tierSpan = document.createElement("span");
	tierSpan.textContent = `Tier: ${sysInfo.memoryTier}`;
	sysDetails.appendChild(tierSpan);

	if (sysInfo.hasGpu) {
		const gpuSpan = document.createElement("span");
		gpuSpan.className = "text-[var(--ok)]";
		gpuSpan.textContent = "GPU available";
		sysDetails.appendChild(gpuSpan);
	}

	sysSection.appendChild(sysDetails);
	wrapper.appendChild(sysSection);

	// Backend selector (show on Apple Silicon where both GGUF and MLX are options)
	const backends = sysInfo.availableBackends || [];
	if (sysInfo.isAppleSilicon && backends.length > 0) {
		const backendSection = document.createElement("div");
		backendSection.className = "flex flex-col gap-2 mb-4";

		const backendLabel = document.createElement("div");
		backendLabel.className = "text-xs font-medium text-[var(--text-strong)]";
		backendLabel.textContent = "Inference Backend";
		backendSection.appendChild(backendLabel);

		const backendCards = document.createElement("div");
		backendCards.className = "flex flex-col gap-2";

		// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: backend card rendering with many conditions
		backends.forEach((b: BackendInfo) => {
			const card = document.createElement("div");
			card.className = "backend-card";
			if (!b.available) card.className += " disabled";
			if (b.id === selectedBackend) card.className += " selected";
			card.dataset.backendId = b.id;

			const header = document.createElement("div");
			header.className = "flex items-center justify-between";

			const name = document.createElement("span");
			name.className = "backend-name text-sm font-medium text-[var(--text)]";
			name.textContent = b.name;
			header.appendChild(name);

			const badges = document.createElement("div");
			badges.className = "flex gap-2";

			if (b.id === sysInfo.recommendedBackend && b.available) {
				const recBadge = document.createElement("span");
				recBadge.className = "recommended-badge";
				recBadge.textContent = "Recommended";
				badges.appendChild(recBadge);
			}

			if (!b.available) {
				const unavailBadge = document.createElement("span");
				unavailBadge.className = "tier-badge";
				unavailBadge.textContent = "Not installed";
				badges.appendChild(unavailBadge);
			}

			header.appendChild(badges);
			card.appendChild(header);

			const desc = document.createElement("div");
			desc.className = "text-xs text-[var(--muted)] mt-1";
			desc.textContent = b.description;
			card.appendChild(desc);

			// Show install instructions for unavailable backends
			if (!b.available && b.id === "MLX") {
				const cmds = b.installCommands || ["pip install mlx-lm"];
				const tpl = S.$<HTMLTemplateElement>("tpl-install-hint")!;
				const hintEl = (tpl.content.cloneNode(true) as DocumentFragment).firstElementChild as HTMLElement;
				const labelEl = hintEl.querySelector("[data-install-label]") as HTMLElement;
				const container = hintEl.querySelector("[data-install-commands]") as HTMLElement;

				labelEl.textContent = cmds.length === 1 ? "Install with:" : "Install with any of:";

				const cmdTpl = S.$<HTMLTemplateElement>("tpl-install-cmd")!;
				cmds.forEach((c: string) => {
					const cmdEl = (cmdTpl.content.cloneNode(true) as DocumentFragment).firstElementChild as HTMLElement;
					cmdEl.textContent = c;
					container.appendChild(cmdEl);
				});

				card.appendChild(hintEl);
			}

			if (b.available) {
				card.addEventListener("click", () => {
					// Deselect all cards
					backendCards.querySelectorAll(".backend-card").forEach((c: Element) => {
						c.classList.remove("selected");
					});
					// Select this card
					card.classList.add("selected");
					selectedBackend = b.id;
					// Re-render models for new backend
					if (wrapper._renderModelsForBackend) {
						wrapper._renderModelsForBackend(b.id);
					}
					// Update filename input visibility
					if (wrapper._updateFilenameVisibility) {
						wrapper._updateFilenameVisibility(b.id);
					}
				});
			}

			backendCards.appendChild(card);
		});

		backendSection.appendChild(backendCards);
		wrapper.appendChild(backendSection);
	} else if (sysInfo.backendNote) {
		// Non-Apple Silicon - just show info
		const backendDiv = document.createElement("div");
		backendDiv.className = "text-xs text-[var(--muted)] mb-4";
		// Safe: backendNote comes from server system info, not user input
		backendDiv.textContent = `Backend: ${sysInfo.backendNote}`;
		wrapper.appendChild(backendDiv);
	}

	// Models section
	const modelsTitle = document.createElement("div");
	modelsTitle.className = "text-xs font-medium text-[var(--text-strong)] mb-2";
	modelsTitle.textContent = "Select a Model";
	wrapper.appendChild(modelsTitle);

	const modelsList = document.createElement("div");
	modelsList.className = "flex flex-col gap-2";
	modelsList.id = "local-model-list";

	// Helper to render models filtered by backend
	function renderModelsForBackend(backend: string): void {
		modelsList.textContent = "";
		const recommended = modelsData.recommended || [];
		const filtered = recommended.filter((mdl: LocalModelInfo) => mdl.backend === backend);
		if (filtered.length === 0) {
			const empty = document.createElement("div");
			empty.className = "text-xs text-[var(--muted)] py-4 text-center";
			empty.textContent = `No models available for ${backend}`;
			modelsList.appendChild(empty);
			return;
		}
		filtered.forEach((model: LocalModelInfo) => {
			const card = createModelCard(model, provider, sysInfo.totalRamGb);
			modelsList.appendChild(card);
		});
	}

	// Initial render with selected backend
	renderModelsForBackend(selectedBackend);

	// Store render function for backend card click handlers
	wrapper._renderModelsForBackend = renderModelsForBackend;

	wrapper.appendChild(modelsList);

	// HuggingFace search section
	const searchSection = document.createElement("div");
	searchSection.className = "flex flex-col gap-2 mt-4 pt-4 border-t border-[var(--border)]";

	const searchLabel = document.createElement("div");
	searchLabel.className = "text-xs font-medium text-[var(--text-strong)]";
	searchLabel.textContent = "Search HuggingFace";
	searchSection.appendChild(searchLabel);

	const searchRow = document.createElement("div");
	searchRow.className = "flex gap-2";

	const searchInput = document.createElement("input");
	searchInput.type = "text";
	searchInput.placeholder = "Search models...";
	searchInput.className = "provider-input flex-1";
	searchRow.appendChild(searchInput);

	const searchBtn = document.createElement("button");
	searchBtn.className = "provider-btn provider-btn-secondary";
	searchBtn.textContent = "Search";
	searchRow.appendChild(searchBtn);

	searchSection.appendChild(searchRow);

	const searchResults = document.createElement("div");
	searchResults.className = "flex flex-col gap-2 max-h-48 overflow-y-auto";
	searchResults.id = "hf-search-results";
	searchSection.appendChild(searchResults);

	// Search handler
	const doSearch = async (): Promise<void> => {
		const query = searchInput.value.trim();
		if (!query) return;
		searchBtn.disabled = true;
		searchBtn.textContent = "Searching...";
		searchResults.textContent = "";
		const res = await sendRpc<{ results: HfSearchResult[] }>("providers.local.search_hf", {
			query: query,
			backend: selectedBackend,
			limit: 15,
		});
		searchBtn.disabled = false;
		searchBtn.textContent = "Search";
		if (!(res?.ok && (res.payload as { results?: HfSearchResult[] })?.results?.length)) {
			const noResults = document.createElement("div");
			noResults.className = "text-xs text-[var(--muted)] py-2";
			noResults.textContent = "No results found";
			searchResults.appendChild(noResults);
			return;
		}
		(res.payload as { results: HfSearchResult[] }).results.forEach((result: HfSearchResult) => {
			const card = createHfSearchResultCard(result, provider);
			searchResults.appendChild(card);
		});
	};

	searchBtn.addEventListener("click", doSearch);
	searchInput.addEventListener("keydown", (e: KeyboardEvent) => {
		if (e.key === "Enter" && !e.isComposing) doSearch();
	});

	// Auto-search with debounce when user stops typing
	let searchTimeout: ReturnType<typeof setTimeout> | null = null;
	searchInput.addEventListener("input", () => {
		if (searchTimeout) clearTimeout(searchTimeout);
		const query = searchInput.value.trim();
		if (query.length >= 2) {
			searchTimeout = setTimeout(doSearch, 500);
		}
	});

	wrapper.appendChild(searchSection);

	// Custom repo section
	const customSection = document.createElement("div");
	customSection.className = "flex flex-col gap-2 mt-4 pt-4 border-t border-[var(--border)]";

	const customLabel = document.createElement("div");
	customLabel.className = "text-xs font-medium text-[var(--text-strong)]";
	customLabel.textContent = "Or enter HuggingFace repo URL";
	customSection.appendChild(customLabel);

	const customRow = document.createElement("div");
	customRow.className = "flex gap-2";

	const customInput = document.createElement("input");
	customInput.type = "text";
	customInput.placeholder = selectedBackend === "MLX" ? "mlx-community/Model-Name" : "TheBloke/Model-GGUF";
	customInput.className = "provider-input flex-1";
	customRow.appendChild(customInput);

	const customBtn = document.createElement("button");
	customBtn.className = "provider-btn";
	customBtn.textContent = "Use";
	customRow.appendChild(customBtn);

	customSection.appendChild(customRow);

	// GGUF filename input (only for GGUF backend)
	const filenameRow = document.createElement("div");
	filenameRow.className = "flex gap-2";
	filenameRow.style.display = selectedBackend === "GGUF" ? "flex" : "none";

	const filenameInput = document.createElement("input");
	filenameInput.type = "text";
	filenameInput.placeholder = "model-file.gguf (required for GGUF)";
	filenameInput.className = "provider-input flex-1";
	filenameRow.appendChild(filenameInput);

	customSection.appendChild(filenameRow);

	// Update filename visibility when backend changes
	wrapper._updateFilenameVisibility = (backend: string): void => {
		filenameRow.style.display = backend === "GGUF" ? "flex" : "none";
		customInput.placeholder = backend === "MLX" ? "mlx-community/Model-Name" : "TheBloke/Model-GGUF";
	};

	// Custom repo handler
	customBtn.addEventListener("click", async () => {
		const repo = customInput.value.trim();
		if (!repo) return;

		const params: Record<string, string | null> = {
			hfRepo: repo,
			backend: selectedBackend,
		};
		if (selectedBackend === "GGUF") {
			const filename = filenameInput.value.trim();
			if (!filename) {
				filenameInput.focus();
				return;
			}
			params.hfFilename = filename;
		}

		customBtn.disabled = true;
		customBtn.textContent = "Configuring...";
		const res = await sendRpc<{ modelId: string }>("providers.local.configure_custom", params);
		customBtn.disabled = false;
		customBtn.textContent = "Use";

		if (res?.ok) {
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			showModelDownloadProgress({ id: (res.payload as { modelId: string }).modelId, displayName: repo }, provider);
		} else {
			const err = res?.error?.message || "Failed to configure model";
			const errEl = document.createElement("div");
			errEl.className = "text-xs text-[var(--error)] py-2";
			errEl.textContent = err;
			searchResults.textContent = "";
			searchResults.appendChild(errEl);
		}
	});

	wrapper.appendChild(customSection);

	// Back button
	const btns = document.createElement("div");
	btns.className = "btn-row mt-4";

	const backBtn = document.createElement("button");
	backBtn.className = "provider-btn provider-btn-secondary";
	backBtn.textContent = "Back";
	backBtn.addEventListener("click", openProviderModal);
	btns.appendChild(backBtn);
	wrapper.appendChild(btns);

	m.body.appendChild(wrapper);
}

// Create a card for HuggingFace search result
function createHfSearchResultCard(model: HfSearchResult, provider: ProviderInfo): HTMLElement {
	const card = document.createElement("div");
	card.className = "model-card";

	const header = document.createElement("div");
	header.className = "flex items-center justify-between";

	const name = document.createElement("span");
	name.className = "text-sm font-medium text-[var(--text)]";
	name.textContent = model.displayName;
	header.appendChild(name);

	const stats = document.createElement("div");
	stats.className = "flex gap-2 text-xs text-[var(--muted)]";
	if (model.downloads) {
		const dl = document.createElement("span");
		dl.textContent = `\u2193${formatDownloads(model.downloads)}`;
		stats.appendChild(dl);
	}
	if (model.likes) {
		const likes = document.createElement("span");
		likes.textContent = `\u2665${model.likes}`;
		stats.appendChild(likes);
	}
	header.appendChild(stats);

	card.appendChild(header);

	const repo = document.createElement("div");
	repo.className = "text-xs text-[var(--muted)] mt-1";
	repo.textContent = model.id;
	card.appendChild(repo);

	card.addEventListener("click", async () => {
		// Prevent multiple clicks
		if (card.dataset.configuring) return;
		card.dataset.configuring = "true";

		const params: Record<string, string> = {
			hfRepo: model.id,
			backend: model.backend,
		};
		// For GGUF, we'd need to fetch the file list - for now, prompt user
		if (model.backend === "GGUF") {
			const filename = prompt("Enter the GGUF filename (e.g., model-q4_k_m.gguf):");
			if (!filename) {
				delete card.dataset.configuring;
				return;
			}
			params.hfFilename = filename;
		}
		card.style.opacity = "0.5";
		card.style.pointerEvents = "none";

		// Show configuring state in modal
		const modalEls = els();
		modalEls.body.textContent = "";
		const statusWrapper = document.createElement("div");
		statusWrapper.className = "provider-key-form";
		const statusText = document.createElement("div");
		statusText.className = "text-sm text-[var(--text)]";
		statusText.textContent = `Configuring ${model.displayName}...`;
		statusWrapper.appendChild(statusText);
		modalEls.body.appendChild(statusWrapper);

		const res = await sendRpc<{ modelId: string }>("providers.local.configure_custom", params);
		if (res?.ok) {
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			showModelDownloadProgress(
				{ id: (res.payload as { modelId: string }).modelId, displayName: model.displayName },
				provider,
			);
		} else {
			const err = res?.error?.message || "Failed to configure model";
			statusText.className = "text-sm text-[var(--error)]";
			statusText.textContent = err;
		}
	});

	return card;
}

// Format download count (e.g., 1234567 -> "1.2M")
function formatDownloads(n: number): string {
	if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
	if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
	return n.toString();
}

function createModelCard(model: LocalModelInfo, provider: ProviderInfo, totalRamGb: number): HTMLElement {
	const card = document.createElement("div");
	card.className = "model-card";
	const detectedRamGb = Number.isFinite(totalRamGb) ? totalRamGb : 0;
	const hasEnoughRam = detectedRamGb >= model.minRamGb;

	const header = document.createElement("div");
	header.className = "flex items-center justify-between";

	const name = document.createElement("span");
	name.className = "text-sm font-medium text-[var(--text)]";
	name.textContent = model.displayName;
	header.appendChild(name);

	const badges = document.createElement("div");
	badges.className = "flex gap-2";

	const ramBadge = document.createElement("span");
	ramBadge.className = "tier-badge";
	ramBadge.textContent = `${model.minRamGb}GB`;
	badges.appendChild(ramBadge);

	if (model.suggested && hasEnoughRam) {
		const suggestedBadge = document.createElement("span");
		suggestedBadge.className = "recommended-badge";
		suggestedBadge.textContent = "Recommended";
		badges.appendChild(suggestedBadge);
	}

	if (!hasEnoughRam) {
		const insufficientBadge = document.createElement("span");
		insufficientBadge.className = "tier-badge";
		insufficientBadge.textContent = "Insufficient RAM";
		badges.appendChild(insufficientBadge);
	}

	header.appendChild(badges);
	card.appendChild(header);

	const meta = document.createElement("div");
	meta.className = "text-xs text-[var(--muted)] mt-1";
	meta.textContent = `Context: ${(model.contextWindow / 1000).toFixed(0)}k tokens`;
	card.appendChild(meta);

	if (!hasEnoughRam) {
		card.classList.add("disabled");
		const warning = document.createElement("div");
		warning.className = "text-xs text-[var(--error)] mt-1";
		warning.textContent = `You do not have enough RAM for this model (${detectedRamGb}GB detected, ${model.minRamGb}GB required).`;
		card.appendChild(warning);
		return card;
	}

	card.addEventListener("click", () => selectLocalModel(model, provider));

	return card;
}

export function showModelDownloadProgress(model: { id: string; displayName: string }, provider: ProviderInfo): void {
	const m = els();
	m.modal.classList.remove("hidden");
	m.body.textContent = "";

	const wrapper = document.createElement("div");
	wrapper.className = "provider-key-form";

	const status = document.createElement("div");
	status.className = "text-sm text-[var(--text)]";
	status.textContent = `Configuring ${model.displayName}...`;
	wrapper.appendChild(status);

	const progress = document.createElement("div");
	progress.className = "download-progress mt-4";

	const progressBar = document.createElement("div");
	progressBar.className = "download-progress-bar";
	progressBar.style.width = "0%";
	progress.appendChild(progressBar);

	const progressText = document.createElement("div");
	progressText.className = "text-xs text-[var(--muted)] mt-2";
	progress.appendChild(progressText);

	wrapper.appendChild(progress);
	m.body.appendChild(wrapper);

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: download progress handler with many states
	const off = onEvent("local-llm.download", (payload: unknown) => {
		const p = payload as LocalLlmDownloadPayload;
		if (p.modelId !== model.id) return;

		if (p.error) {
			status.textContent = p.error;
			status.className = "text-sm text-[var(--error)]";
			off();
			return;
		}

		if (p.complete) {
			status.textContent = `${model.displayName} downloaded successfully!`;
			status.className = "provider-status";
			progressBar.style.width = "100%";
			progressText.textContent = "";
			off();
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			setTimeout(closeProviderModal, 1500);
			return;
		}

		if (p.progress != null) {
			progressBar.style.width = `${p.progress.toFixed(1)}%`;
			status.textContent = `Downloading ${model.displayName}...`;
		}
		if (p.downloaded != null) {
			const downloadedMb = (p.downloaded / (1024 * 1024)).toFixed(1);
			if (p.total != null) {
				const totalMb = (p.total / (1024 * 1024)).toFixed(1);
				progressText.textContent = `${downloadedMb} MB / ${totalMb} MB`;
			} else {
				progressText.textContent = `${downloadedMb} MB downloaded`;
			}
		}
	});

	pollLocalStatus(model, provider, status, progress, off);
}

function selectLocalModel(model: LocalModelInfo, provider: ProviderInfo): void {
	const m = els();
	m.body.textContent = "";

	const wrapper = document.createElement("div");
	wrapper.className = "provider-key-form";

	const status = document.createElement("div");
	status.className = "text-sm text-[var(--text)]";
	status.textContent = `Configuring ${model.displayName}...`;
	wrapper.appendChild(status);

	const progress = document.createElement("div");
	progress.className = "download-progress mt-4";

	const progressBar = document.createElement("div");
	progressBar.className = "download-progress-bar";
	progressBar.style.width = "0%";
	progress.appendChild(progressBar);

	const progressText = document.createElement("div");
	progressText.className = "text-xs text-[var(--muted)] mt-2";
	progress.appendChild(progressText);

	wrapper.appendChild(progress);
	m.body.appendChild(wrapper);

	// Subscribe to download progress events
	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: download progress handler with many states
	const off = onEvent("local-llm.download", (payload: unknown) => {
		const p = payload as LocalLlmDownloadPayload;
		if (p.modelId !== model.id) return;

		if (p.error) {
			status.textContent = p.error;
			status.className = "text-sm text-[var(--error)]";
			off();
			return;
		}

		if (p.complete) {
			status.textContent = `${model.displayName} downloaded successfully!`;
			status.className = "provider-status";
			progressBar.style.width = "100%";
			progressText.textContent = "";
			off();
			fetchModels();
			if (S.refreshProvidersPage) S.refreshProvidersPage();
			setTimeout(closeProviderModal, 1500);
			return;
		}

		// Update progress
		if (p.progress != null) {
			progressBar.style.width = `${p.progress.toFixed(1)}%`;
			status.textContent = `Downloading ${model.displayName}...`;
		}
		if (p.downloaded != null) {
			const downloadedMb = (p.downloaded / (1024 * 1024)).toFixed(1);
			if (p.total != null) {
				const totalMb = (p.total / (1024 * 1024)).toFixed(1);
				progressText.textContent = `${downloadedMb} MB / ${totalMb} MB`;
			} else {
				progressText.textContent = `${downloadedMb} MB downloaded`;
			}
		}
	});

	sendRpc("providers.local.configure", { modelId: model.id, backend: selectedBackend }).then((res: RpcResponse) => {
		if (!res?.ok) {
			status.textContent = res?.error?.message || "Failed to configure model";
			status.className = "text-sm text-[var(--error)]";
			off(); // Unsubscribe from events
			return;
		}

		// Start polling for status as a fallback (in case WebSocket events are missed)
		pollLocalStatus(model, provider, status, progress, off);
	});
}

function pollLocalStatus(
	model: { id: string; displayName: string },
	_provider: ProviderInfo,
	statusEl: HTMLElement,
	progressEl: HTMLElement,
	offEvent: (() => void) | null,
): void {
	let attempts = 0;
	const maxAttempts = 300; // 10 minutes with 2s interval
	let completed = false;
	const timer = setInterval(() => {
		if (completed) {
			clearInterval(timer);
			return;
		}
		attempts++;
		if (attempts > maxAttempts) {
			clearInterval(timer);
			if (offEvent) offEvent();
			statusEl.textContent = "Configuration timed out. Please try again.";
			statusEl.className = "text-sm text-[var(--error)]";
			return;
		}

		// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: status polling with many state transitions
		sendRpc<{ status: string; error?: string }>("providers.local.status", {}).then(
			(res: RpcResponse<{ status: string; error?: string }>) => {
				if (!res?.ok) return;
				const st = res.payload as { status: string; error?: string };

				if (st.status === "ready" || st.status === "loaded") {
					completed = true;
					clearInterval(timer);
					if (offEvent) offEvent();
					statusEl.textContent = `${model.displayName} configured successfully!`;
					statusEl.className = "provider-status";
					progressEl.style.display = "none";
					fetchModels();
					if (S.refreshProvidersPage) S.refreshProvidersPage();
					setTimeout(closeProviderModal, 1500);
				} else if (st.status === "error") {
					completed = true;
					clearInterval(timer);
					if (offEvent) offEvent();
					statusEl.textContent = st.error || "Configuration failed";
					statusEl.className = "text-sm text-[var(--error)]";
				}
				// Don't update progress here - let WebSocket events handle it
			},
		);
	}, 2000);
}

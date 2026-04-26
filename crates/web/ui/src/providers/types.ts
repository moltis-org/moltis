// ── Shared types for provider sub-modules ────────────────────

export interface ProviderModalElements {
	modal: HTMLElement;
	body: HTMLElement;
	title: HTMLElement;
	close: HTMLElement;
}

export interface ProviderInfo {
	name: string;
	displayName: string;
	authType: string;
	keyOptional?: boolean;
	configured?: boolean;
	isCustom?: boolean;
	uiOrder?: number;
	defaultBaseUrl?: string;
	models?: string[];
}

export interface ModelEntry {
	id: string;
	displayName: string;
	provider?: string;
	supportsTools?: boolean;
	createdAt?: number;
}

export interface ValidationProgressState {
	progress: HTMLElement;
	progressBar: HTMLElement;
	progressText: HTMLElement;
	value: number;
}

export interface ValidationProgressUpdate {
	value: number;
	message: string;
}

export interface ValidationEventPayload {
	requestId?: string;
	phase?: string;
	message?: string;
	modelCount?: number;
	totalAttempts?: number;
	attempt?: number;
	modelId?: string;
}

export interface AddCustomPayload {
	providerName: string;
	displayName: string;
}

export interface SystemInfo {
	totalRamGb: number;
	memoryTier: string;
	hasGpu: boolean;
	isAppleSilicon: boolean;
	recommendedBackend: string;
	availableBackends: BackendInfo[];
	backendNote?: string;
}

export interface BackendInfo {
	id: string;
	name: string;
	description: string;
	available: boolean;
	installCommands?: string[];
}

export interface ModelsData {
	recommended: LocalModelInfo[];
}

export interface LocalModelInfo {
	id: string;
	displayName: string;
	backend: string;
	minRamGb: number;
	contextWindow: number;
	suggested?: boolean;
}

export interface HfSearchResult {
	id: string;
	displayName: string;
	downloads?: number;
	likes?: number;
	backend: string;
}

export interface LocalLlmDownloadPayload {
	modelId?: string;
	error?: string;
	complete?: boolean;
	progress?: number;
	downloaded?: number;
	total?: number;
}

export type { LocalLlmLifecyclePayload } from "../types/ws-events";

export interface ModelStateEntry {
	model_id: string;
	is_loaded: boolean;
	memory_bytes: number;
	last_activity: number;
	idle_timeout_secs: number | null;
}

export interface ProbeResult {
	error?: string;
	timeout?: boolean;
}

// Model selector wrapper with attached properties
export interface ModelSelectorWrapper extends HTMLElement {
	_errorArea?: HTMLElement;
	_resetSelection?: () => void;
	_renderModelsForBackend?: (backend: string) => void;
	_updateFilenameVisibility?: (backend: string) => void;
}

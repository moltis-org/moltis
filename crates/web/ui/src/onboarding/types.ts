// ── Shared types for onboarding sub-modules ──────────────────

export interface ProviderInfo {
	name: string;
	displayName: string;
	authType: string;
	configured: boolean;
	keyOptional?: boolean;
	defaultBaseUrl?: string;
	baseUrl?: string;
	model?: string;
	models?: string[];
	uiOrder?: number;
	[key: string]: unknown;
}

export interface ModelSelectorRow {
	id: string;
	displayName: string;
	provider?: string;
	supportsTools?: boolean;
	createdAt?: number;
	recommended?: boolean;
}

export interface ValidationResult {
	ok: boolean;
	message: string | null;
}

export interface OAuthInfo {
	status: string;
	uri?: string;
	code?: string;
}

export interface SysInfo {
	totalRamGb: number;
	memoryTier: string;
	hasGpu: boolean;
	isAppleSilicon: boolean;
	recommendedBackend: string;
	availableBackends?: BackendInfo[];
}

export interface BackendInfo {
	id: string;
	name: string;
	description: string;
	available: boolean;
}

export interface LocalModel {
	id: string;
	displayName: string;
	backend: string;
	minRamGb: number;
	contextWindow: number;
	suggested?: boolean;
}

export interface IdentityInfo {
	user_name?: string;
	name?: string;
	emoji?: string;
	theme?: string;
	[key: string]: unknown;
}

export interface KeyHelp {
	text: string;
	url?: string;
	label?: string;
}

export interface ProbeResult {
	error?: string;
}

export interface RawModelRow {
	id: string;
	displayName?: string;
	provider?: string;
	supportsTools?: boolean;
	createdAt?: number;
}

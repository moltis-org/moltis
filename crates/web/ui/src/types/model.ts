// ── Model types ─────────────────────────────────────��──────────
//
// Mirrors the JSON shape produced by `LiveModelService::list()` in
// `crates/chat/src/models.rs`. The JSON is built manually with
// camelCase keys (not derived from serde).

/**
 * Model info as returned by the `models.list` and `models.list_all` RPCs.
 */
export interface ModelInfo {
	id: string;
	provider: string;
	displayName: string;
	supportsTools: boolean;
	supportsVision: boolean;
	supportsReasoning: boolean;
	preferred: boolean;
	recommended: boolean;
	createdAt?: number | null;
	/** Present only in `models.list_all` responses. */
	disabled?: boolean;
	/** Present in `models.list` (always false) and `models.list_all`. */
	unsupported: boolean;
	unsupportedReason?: string | null;
	unsupportedProvider?: string | null;
	unsupportedUpdatedAt?: number | null;
}

/** Parsed reasoning-suffix result. */
export interface ReasoningSuffix {
	baseId: string;
	effort: string;
}

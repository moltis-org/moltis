// ── Global window augmentations ─────────────────────────────
//
// Ambient declarations for custom properties attached to `window`
// across the moltis web UI. This file has no imports/exports so
// its declarations are visible to every compilation unit, including
// standalone entry points like login-app.tsx.

interface MoltisStores {
	sessionStore: typeof import("../stores/session-store").sessionStore;
	modelStore: typeof import("../stores/model-store");
	projectStore: typeof import("../stores/project-store");
}

interface Window {
	/** Server-injected data (gon pattern). See gon.ts for typed access. */
	__MOLTIS__?: Partial<import("./gon").GonData>;
	/** Suppress the next password-changed WebSocket redirect. */
	__moltisSuppressNextPasswordChangedRedirect?: boolean;
	/** Exposed stores for E2E test access. */
	__moltis_stores?: MoltisStores;
	/** Exposed state module for E2E test WS connection checks. */
	__moltis_state?: Record<string, unknown>;
	/** Exposed bundled modules for E2E test dynamic imports. */
	__moltis_modules?: Record<string, Record<string, unknown>>;
}

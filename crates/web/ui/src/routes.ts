// ── Central route definitions ────────────────────────────────
//
// All SPA paths are defined once in Rust (SpaRoutes) and injected
// via gon. This module re-exports them so JS never hardcodes paths.

import * as gon from "./gon";
import type { SpaRoutes } from "./types/gon";

const r: Partial<SpaRoutes> = gon.get("routes") || {};
export const routes: Partial<SpaRoutes> = r;

export function settingsPath(id: string): string {
	return `${r.settings}/${id}`;
}

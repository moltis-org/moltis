// ── Central route definitions ────────────────────────────────
//
// All SPA paths are defined once in Rust (SpaRoutes) and injected
// via gon. This module re-exports them so JS never hardcodes paths.

import * as gon from "./gon.js";

var r = gon.get("routes") || {};
export var routes = r;

export function settingsPath(id) {
	return `${r.settings}/${id}`;
}

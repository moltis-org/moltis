// ── Server-injected data (gon pattern) ────────────────────
//
// The server injects `window.__MOLTIS__ = { ... }` into every
// page <head> before any module script runs.  This module
// provides typed access, runtime updates, and a refresh
// mechanism that re-fetches the data from `/api/gon`.
//
// Register listeners with `onChange(key, fn)` to react when
// a key is updated (via `set()` or `refresh()`).

var gon = window.__MOLTIS__ || {};
var listeners = {};

export function get(key) {
	return gon[key] ?? null;
}

export function set(key, value) {
	gon[key] = value;
	notify(key, value);
}

export function onChange(key, fn) {
	if (!listeners[key]) listeners[key] = [];
	listeners[key].push(fn);
}

export function refresh() {
	return fetch(`/api/gon?_=${Date.now()}`, {
		cache: "no-store",
		headers: {
			"Cache-Control": "no-cache",
			Pragma: "no-cache",
		},
	})
		.then((r) => (r.ok ? r.json() : null))
		.then((data) => {
			if (!data) return;
			for (var key of Object.keys(data)) {
				gon[key] = data[key];
				notify(key, data[key]);
			}
		});
}

function notify(key, value) {
	for (var fn of listeners[key] || []) fn(value);
}

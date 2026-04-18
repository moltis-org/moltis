// ── Server-injected data (gon pattern) ────────────────────
//
// The server injects `window.__MOLTIS__ = { ... }` into every
// page <head> before any module script runs.  This module
// provides typed access, runtime updates, and a refresh
// mechanism that re-fetches the data from `/api/gon`.
//
// Register listeners with `onChange(key, fn)` to react when
// a key is updated (via `set()` or `refresh()`).

import type { GonData, GonKey } from "./types/gon";

declare global {
	interface Window {
		__MOLTIS__?: Partial<GonData>;
	}
}

type GonListener<K extends GonKey = GonKey> = (value: GonData[K]) => void;

const gon: Partial<GonData> = window.__MOLTIS__ || {};
const listeners: Partial<Record<GonKey, GonListener[]>> = {};

export function get<K extends GonKey>(key: K): GonData[K] | null {
	return (gon[key] as GonData[K]) ?? null;
}

export function set<K extends GonKey>(key: K, value: GonData[K] | null): void {
	(gon as Record<string, unknown>)[key] = value;
	notify(key, value as GonData[K]);
}

export function onChange<K extends GonKey>(key: K, fn: GonListener<K>): void {
	if (!listeners[key]) listeners[key] = [];
	(listeners[key] as GonListener<K>[]).push(fn);
}

export function offChange<K extends GonKey>(key: K, fn: GonListener<K>): void {
	const arr = listeners[key] as GonListener<K>[] | undefined;
	if (!arr) return;
	const idx = arr.indexOf(fn);
	if (idx !== -1) arr.splice(idx, 1);
}

export function refresh(): Promise<void> {
	return fetch(`/api/gon?_=${Date.now()}`, {
		cache: "no-store",
		headers: {
			"Cache-Control": "no-cache",
			Pragma: "no-cache",
		},
	})
		.then((r) => (r.ok ? (r.json() as Promise<Partial<GonData>>) : null))
		.then((data) => {
			if (!data) return;
			for (const key of Object.keys(data) as GonKey[]) {
				(gon as Record<string, unknown>)[key] = data[key];
				notify(key, data[key] as GonData[typeof key]);
			}
		});
}

function notify<K extends GonKey>(key: K, value: GonData[K]): void {
	for (const fn of (listeners[key] as GonListener<K>[] | undefined) || []) fn(value);
}

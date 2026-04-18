// ── Session history cache (in-memory) ─────────────────────────
//
// Stores per-session chat history so re-selecting a session can render
// immediately. Histories are patched incrementally from websocket events and
// refreshed authoritatively from sessions.switch responses.

import type { HistoryMessage } from "../types";

const historyByKey = new Map<string, HistoryMessage[]>();
const revisionByKey = new Map<string, number>();
const bytesByKey = new Map<string, number>();
const lastAccessByKey = new Map<string, number>();
let totalBytes = 0;
const encoder: TextEncoder | null = typeof TextEncoder === "function" ? new TextEncoder() : null;

const MAX_TOTAL_HISTORY_BYTES = 12 * 1024 * 1024;
const MAX_SESSION_HISTORY_BYTES = 2 * 1024 * 1024;
const MIN_SESSION_HISTORY_MESSAGES = 80;
const TRIM_STEP_MESSAGES = 25;

function deepClone<T>(value: T): T {
	if (value === undefined) return undefined as T;
	if (typeof structuredClone === "function") {
		try {
			return structuredClone(value);
		} catch (_e) {
			// Fall through to JSON clone.
		}
	}
	return JSON.parse(JSON.stringify(value));
}

function toValidIndex(value: unknown): number | null {
	if (value === null || value === undefined) return null;
	const parsed = Number(value);
	if (!Number.isInteger(parsed) || parsed < 0) return null;
	return parsed;
}

function messageHistoryIndex(msg: HistoryMessage | null | undefined): number | null {
	if (!(msg && typeof msg === "object")) return null;
	const direct = toValidIndex(msg.historyIndex);
	if (direct !== null) return direct;
	return toValidIndex(msg.messageIndex);
}

function bumpRevision(key: string): void {
	revisionByKey.set(key, (revisionByKey.get(key) || 0) + 1);
}

function touchHistoryKey(key: string): void {
	lastAccessByKey.set(key, Date.now());
}

function estimateHistoryBytes(history: HistoryMessage[]): number {
	try {
		const serialized = JSON.stringify(history || []);
		if (!serialized) return 0;
		if (encoder) return encoder.encode(serialized).length;
		return serialized.length;
	} catch (_e) {
		return 0;
	}
}

function updateHistorySize(key: string, nextBytes: number): void {
	const prev = bytesByKey.get(key) || 0;
	bytesByKey.set(key, nextBytes);
	totalBytes += nextBytes - prev;
	if (totalBytes < 0) totalBytes = 0;
}

function dropHistoryKey(key: string): void {
	const prev = bytesByKey.get(key) || 0;
	historyByKey.delete(key);
	revisionByKey.delete(key);
	bytesByKey.delete(key);
	lastAccessByKey.delete(key);
	totalBytes -= prev;
	if (totalBytes < 0) totalBytes = 0;
}

function trimSessionHistoryInPlace(list: HistoryMessage[]): number {
	let bytes = estimateHistoryBytes(list);
	while (list.length > MIN_SESSION_HISTORY_MESSAGES && list.length > 1 && bytes > MAX_SESSION_HISTORY_BYTES) {
		const removable = list.length - MIN_SESSION_HISTORY_MESSAGES;
		const trimCount = Math.min(TRIM_STEP_MESSAGES, removable);
		list.splice(0, trimCount);
		bytes = estimateHistoryBytes(list);
	}

	while (list.length > 1 && bytes > MAX_SESSION_HISTORY_BYTES) {
		list.shift();
		bytes = estimateHistoryBytes(list);
	}

	return bytes;
}

function evictGlobalHistoryBudget(preferredKey: string): void {
	while (totalBytes > MAX_TOTAL_HISTORY_BYTES && historyByKey.size > 0) {
		let victim: string | null = null;
		let oldest = Number.POSITIVE_INFINITY;
		for (const [key, ts] of lastAccessByKey.entries()) {
			if (key === preferredKey && historyByKey.size > 1) continue;
			if (ts < oldest) {
				oldest = ts;
				victim = key;
			}
		}
		if (!victim) {
			victim = historyByKey.keys().next().value ?? null;
		}
		if (!victim) break;
		dropHistoryKey(victim);
	}
}

function enforceHistoryBudgets(key: string): void {
	const list = historyByKey.get(key);
	if (!list) return;
	const bytes = trimSessionHistoryInPlace(list);
	updateHistorySize(key, bytes);
	touchHistoryKey(key);
	evictGlobalHistoryBudget(key);
}

function normalizeMessage(message: unknown, fallbackIndex?: number | null): HistoryMessage {
	let next: HistoryMessage = (deepClone(message) as HistoryMessage) || {};
	if (!(next && typeof next === "object")) {
		next = { role: "notice", content: String(message || "") };
	}
	const idx = toValidIndex(fallbackIndex);
	const msgIdx = idx === null ? messageHistoryIndex(next) : idx;
	if (msgIdx !== null) next.historyIndex = msgIdx;
	return next;
}

function upsertWithoutIndex(list: HistoryMessage[], next: HistoryMessage): void {
	if (next.role === "tool_result" && next.tool_call_id) {
		const existingToolIdx = list.findIndex(
			(msg) => msg?.role === "tool_result" && msg?.tool_call_id && msg.tool_call_id === next.tool_call_id,
		);
		if (existingToolIdx >= 0) {
			list[existingToolIdx] = next;
			return;
		}
	}
	if (next.role === "assistant" && next.run_id) {
		const existingRunIdx = list.findIndex(
			(msg) => msg?.role === "assistant" && msg?.run_id && msg.run_id === next.run_id,
		);
		if (existingRunIdx >= 0) {
			list[existingRunIdx] = next;
			return;
		}
	}
	list.push(next);
}

function upsertByIndex(list: HistoryMessage[], next: HistoryMessage, historyIndex: number): void {
	const existingIdx = list.findIndex((msg) => messageHistoryIndex(msg) === historyIndex);
	if (existingIdx >= 0) {
		list[existingIdx] = next;
		return;
	}
	const insertAt = list.findIndex((msg) => {
		const other = messageHistoryIndex(msg);
		if (other === null) return true;
		return other > historyIndex;
	});
	if (insertAt === -1) {
		list.push(next);
		return;
	}
	list.splice(insertAt, 0, next);
}

export function getHistoryRevision(key: string): number {
	return revisionByKey.get(key) || 0;
}

export function hasSessionHistory(key: string): boolean {
	return historyByKey.has(key);
}

export function getSessionHistory(key: string): HistoryMessage[] | null {
	const history = historyByKey.get(key) || null;
	if (history) touchHistoryKey(key);
	return history;
}

export function replaceSessionHistory(key: string, history: unknown[]): HistoryMessage[] {
	const next = Array.isArray(history) ? history.map((msg) => normalizeMessage(msg)) : [];
	historyByKey.set(key, next);
	bumpRevision(key);
	enforceHistoryBudgets(key);
	return next;
}

export function upsertSessionHistoryMessage(
	key: string,
	message: unknown,
	historyIndex?: number | null,
): HistoryMessage {
	let list = historyByKey.get(key);
	if (!list) {
		list = [];
		historyByKey.set(key, list);
	}
	const next = normalizeMessage(message, historyIndex);
	const idx = messageHistoryIndex(next);
	if (idx !== null) {
		upsertByIndex(list, next, idx);
	} else {
		upsertWithoutIndex(list, next);
	}
	bumpRevision(key);
	enforceHistoryBudgets(key);
	return next;
}

export function clearSessionHistory(key?: string): void {
	if (key === undefined) {
		historyByKey.clear();
		revisionByKey.clear();
		bytesByKey.clear();
		lastAccessByKey.clear();
		totalBytes = 0;
		return;
	}
	dropHistoryKey(key);
}

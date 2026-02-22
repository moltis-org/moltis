// ── Session history cache (in-memory) ─────────────────────────
//
// Stores per-session chat history so re-selecting a session can render
// immediately. Histories are patched incrementally from websocket events and
// refreshed authoritatively from sessions.switch responses.

var historyByKey = new Map();
var revisionByKey = new Map();

function deepClone(value) {
	if (value === undefined) return undefined;
	if (typeof structuredClone === "function") {
		try {
			return structuredClone(value);
		} catch (_e) {
			// Fall through to JSON clone.
		}
	}
	return JSON.parse(JSON.stringify(value));
}

function toValidIndex(value) {
	if (value === null || value === undefined) return null;
	var parsed = Number(value);
	if (!Number.isInteger(parsed) || parsed < 0) return null;
	return parsed;
}

function messageHistoryIndex(msg) {
	if (!(msg && typeof msg === "object")) return null;
	var direct = toValidIndex(msg.historyIndex);
	if (direct !== null) return direct;
	return toValidIndex(msg.messageIndex);
}

function bumpRevision(key) {
	revisionByKey.set(key, (revisionByKey.get(key) || 0) + 1);
}

function normalizeMessage(message, fallbackIndex) {
	var next = deepClone(message) || {};
	if (!(next && typeof next === "object")) {
		next = { role: "notice", content: String(message || "") };
	}
	var idx = toValidIndex(fallbackIndex);
	if (idx === null) idx = messageHistoryIndex(next);
	if (idx !== null) next.historyIndex = idx;
	return next;
}

function upsertWithoutIndex(list, next) {
	if (next.role === "tool_result" && next.tool_call_id) {
		var existingToolIdx = list.findIndex(
			(msg) => msg?.role === "tool_result" && msg?.tool_call_id && msg.tool_call_id === next.tool_call_id,
		);
		if (existingToolIdx >= 0) {
			list[existingToolIdx] = next;
			return;
		}
	}
	if (next.role === "assistant" && next.run_id) {
		var existingRunIdx = list.findIndex(
			(msg) => msg?.role === "assistant" && msg?.run_id && msg.run_id === next.run_id,
		);
		if (existingRunIdx >= 0) {
			list[existingRunIdx] = next;
			return;
		}
	}
	list.push(next);
}

function upsertByIndex(list, next, historyIndex) {
	var existingIdx = list.findIndex((msg) => messageHistoryIndex(msg) === historyIndex);
	if (existingIdx >= 0) {
		list[existingIdx] = next;
		return;
	}
	var insertAt = list.findIndex((msg) => {
		var other = messageHistoryIndex(msg);
		if (other === null) return true;
		return other > historyIndex;
	});
	if (insertAt === -1) {
		list.push(next);
		return;
	}
	list.splice(insertAt, 0, next);
}

export function getHistoryRevision(key) {
	return revisionByKey.get(key) || 0;
}

export function hasSessionHistory(key) {
	return historyByKey.has(key);
}

export function getSessionHistory(key) {
	return historyByKey.get(key) || null;
}

export function replaceSessionHistory(key, history) {
	var next = Array.isArray(history) ? history.map((msg) => normalizeMessage(msg)) : [];
	historyByKey.set(key, next);
	bumpRevision(key);
	return next;
}

export function upsertSessionHistoryMessage(key, message, historyIndex) {
	var list = historyByKey.get(key);
	if (!list) {
		list = [];
		historyByKey.set(key, list);
	}
	var next = normalizeMessage(message, historyIndex);
	var idx = messageHistoryIndex(next);
	if (idx !== null) {
		upsertByIndex(list, next, idx);
	} else {
		upsertWithoutIndex(list, next);
	}
	bumpRevision(key);
	return next;
}

export function clearSessionHistory(key) {
	if (key === undefined) {
		historyByKey.clear();
		revisionByKey.clear();
		return;
	}
	historyByKey.delete(key);
	revisionByKey.delete(key);
}

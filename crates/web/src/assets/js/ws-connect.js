// ── Shared WebSocket connection with JSON-RPC handshake and reconnect ──
import { localizeRpcError, nextId, sendRpc } from "./helpers.js";
import { getPreferredLocale } from "./i18n.js";
import * as S from "./state.js";

var reconnectTimer = null;
var lastOpts = null;

/** Registry of server-request handlers keyed by method name (v4 bidir RPC). */
var serverRequestHandlers = {};

function resolveLocale() {
	return getPreferredLocale();
}

/**
 * Register a handler for server-initiated RPC requests (v4 bidirectional RPC).
 * @param {string} method — method name (e.g. "node.invoke")
 * @param {(params: object) => Promise<object>} handler — returns result or throws
 * @returns {() => void} unregister function
 */
export function onServerRequest(method, handler) {
	serverRequestHandlers[method] = handler;
	return function off() {
		delete serverRequestHandlers[method];
	};
}

/**
 * Open a WebSocket, perform the protocol handshake, route RPC responses to
 * `S.pending`, and auto-reconnect on close.
 *
 * @param {Object} opts
 * @param {(frame: object) => void} [opts.onFrame]       — non-RPC frames (events)
 * @param {(hello: object) => void} [opts.onConnected]    — after successful handshake
 * @param {(frame: object) => void} [opts.onHandshakeFailed]
 * @param {(wasConnected: boolean) => void} [opts.onDisconnected]
 * @param {{ factor?: number, max?: number }} [opts.backoff] — default {1.5, 5000}
 */
export function connectWs(opts) {
	lastOpts = opts;
	var backoff = Object.assign({ factor: 1.5, max: 5000 }, opts.backoff);
	var proto = location.protocol === "https:" ? "wss:" : "ws:";
	var ws = new WebSocket(`${proto}//${location.host}/ws/chat`);
	S.setWs(ws);

	ws.onopen = () => {
		var id = nextId();
		S.pending[id] = (frame) => {
			var hello = frame?.ok && frame.payload;
			if (hello?.type === "hello-ok") {
				S.setConnected(true);
				S.setReconnectDelay(1000);
				if (opts.onConnected) opts.onConnected(hello);
			} else {
				S.setConnected(false);
				if (opts.onHandshakeFailed) opts.onHandshakeFailed(frame);
				else ws.close();
			}
		};
		ws.send(
			JSON.stringify({
				type: "req",
				id: id,
				method: "connect",
				params: {
					protocol: { min: 3, max: 4 },
					client: {
						id: "web-chat-ui",
						version: "0.1.0",
						platform: "browser",
						mode: "operator",
					},
					locale: resolveLocale(),
					timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
				},
			}),
		);
	};

	ws.onmessage = (evt) => {
		var frame;
		try {
			frame = JSON.parse(evt.data);
		} catch {
			return;
		}
		if (frame?.type === "res" && frame.error) {
			frame.error = localizeRpcError(frame.error);
		}
		if (frame.type === "res" && frame.id && S.pending[frame.id]) {
			S.pending[frame.id](frame);
			delete S.pending[frame.id];
			return;
		}
		// Handle server-initiated RPC requests (v4 bidirectional RPC).
		if (frame.type === "req" && frame.id && frame.method) {
			handleServerRequest(ws, frame);
			return;
		}
		if (opts.onFrame) opts.onFrame(frame);
	};

	ws.onclose = () => {
		var wasConnected = S.connected;
		S.setConnected(false);
		for (var id in S.pending) {
			S.pending[id]({ ok: false, error: { message: "WebSocket disconnected" } });
			delete S.pending[id];
		}
		if (opts.onDisconnected) opts.onDisconnected(wasConnected);
		scheduleReconnect(() => connectWs(opts), backoff);
	};

	ws.onerror = () => {
		/* handled by onclose */
	};
}

/** Handle server-initiated RPC request (v4). */
function handleServerRequest(ws, frame) {
	var handler = serverRequestHandlers[frame.method];
	if (!handler) {
		// Unknown method — send error response.
		ws.send(
			JSON.stringify({
				type: "res",
				id: frame.id,
				ok: false,
				error: { code: "UNKNOWN_METHOD", message: `no handler for ${frame.method}` },
			}),
		);
		return;
	}
	Promise.resolve()
		.then(() => handler(frame.params || {}))
		.then((result) => {
			ws.send(JSON.stringify({ type: "res", id: frame.id, ok: true, payload: result || {} }));
		})
		.catch((err) => {
			ws.send(
				JSON.stringify({
					type: "res",
					id: frame.id,
					ok: false,
					error: { code: "INTERNAL", message: String(err?.message || err) },
				}),
			);
		});
}

/**
 * Subscribe to events after handshake. Called from websocket.js.
 * @param {string[]} events — event names to subscribe to
 */
export function subscribeEvents(events) {
	return sendRpc("subscribe", { events: events });
}

function scheduleReconnect(reconnect, backoff) {
	if (reconnectTimer) return;
	reconnectTimer = setTimeout(() => {
		reconnectTimer = null;
		S.setReconnectDelay(Math.min(S.reconnectDelay * backoff.factor, backoff.max));
		reconnect();
	}, S.reconnectDelay);
}

/** Force an immediate reconnect (e.g. on tab visibility change). */
export function forceReconnect(opts) {
	var resolved = opts || lastOpts;
	if (!resolved || S.connected) return;
	clearTimeout(reconnectTimer);
	reconnectTimer = null;
	S.setReconnectDelay(1000);
	connectWs(resolved);
}

// ── Shared WebSocket connection with JSON-RPC handshake and reconnect ──
import { nextId } from "./helpers.js";
import * as S from "./state.js";

var reconnectTimer = null;

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
	var backoff = Object.assign({ factor: 1.5, max: 5000 }, opts.backoff);
	var proto = location.protocol === "https:" ? "wss:" : "ws:";
	var ws = new WebSocket(`${proto}//${location.host}/ws`);
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
					minProtocol: 3,
					maxProtocol: 3,
					client: {
						id: "web-chat-ui",
						version: "0.1.0",
						platform: "browser",
						mode: "operator",
					},
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
		if (frame.type === "res" && frame.id && S.pending[frame.id]) {
			S.pending[frame.id](frame);
			delete S.pending[frame.id];
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
	if (S.connected) return;
	clearTimeout(reconnectTimer);
	reconnectTimer = null;
	S.setReconnectDelay(1000);
	connectWs(opts);
}

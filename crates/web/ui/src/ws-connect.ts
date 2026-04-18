// ── Shared WebSocket connection with JSON-RPC handshake and reconnect ──
import { localizeRpcError, nextId, sendRpc } from "./helpers";
import { getPreferredLocale } from "./i18n";
import * as S from "./state";
import type { RpcResponse } from "./types";

let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let lastOpts: ConnectOptions | null = null;
let authRedirectPending = false;

/** Registry of server-request handlers keyed by method name (v4 bidir RPC). */
const serverRequestHandlers: Record<string, (params: Record<string, unknown>) => Promise<Record<string, unknown>>> = {};

function resolveLocale(): string {
	return getPreferredLocale();
}

function resetAuthRedirectGuard(): void {
	authRedirectPending = false;
}

window.addEventListener("moltis:auth-status-sync-complete", resetAuthRedirectGuard);

/** Backoff configuration for reconnect. */
interface BackoffConfig {
	factor: number;
	max: number;
}

/** Hello payload from the server after successful handshake. */
interface HelloPayload {
	type: string;
	server: {
		version: string;
		[key: string]: unknown;
	};
	[key: string]: unknown;
}

/** RPC frame received over the WebSocket. */
interface WsFrame {
	type: string;
	id?: string;
	method?: string;
	params?: Record<string, unknown>;
	ok?: boolean;
	payload?: HelloPayload | Record<string, unknown>;
	error?: {
		code?: string;
		message?: string;
		[key: string]: unknown;
	};
	event?: string;
	stream?: unknown;
	done?: unknown;
	channel?: unknown;
}



/** Options for connectWs. */
export interface ConnectOptions {
	onFrame?: (frame: WsFrame) => void;
	onConnected?: (hello: HelloPayload) => void | Promise<void>;
	onHandshakeFailed?: (frame: WsFrame) => void;
	onDisconnected?: (wasConnected: boolean) => void;
	backoff?: Partial<BackoffConfig>;
}

/**
 * Register a handler for server-initiated RPC requests (v4 bidirectional RPC).
 * @param method - method name (e.g. "node.invoke")
 * @param handler - returns result or throws
 * @returns unregister function
 */
export function onServerRequest(
	method: string,
	handler: (params: Record<string, unknown>) => Promise<Record<string, unknown>>,
): () => void {
	serverRequestHandlers[method] = handler;
	return function off(): void {
		delete serverRequestHandlers[method];
	};
}

/**
 * Open a WebSocket, perform the protocol handshake, route RPC responses to
 * `S.pending`, and auto-reconnect on close.
 */
export function connectWs(opts: ConnectOptions): void {
	lastOpts = opts;
	const backoff: BackoffConfig = Object.assign({ factor: 1.5, max: 5000 }, opts.backoff);
	const proto = location.protocol === "https:" ? "wss:" : "ws:";
	const ws = new WebSocket(`${proto}//${location.host}/ws/chat`);
	S.setWs(ws);

	ws.onopen = (): void => {
		const id = nextId();
		(S.pending as Record<string, (value: WsFrame) => void>)[id] = (frame: WsFrame): void => {
			const hello = frame?.ok && frame.payload;
			if (hello && (hello as HelloPayload).type === "hello-ok") {
				S.setConnected(true);
				S.setReconnectDelay(1000);
				if (opts.onConnected) opts.onConnected(hello as HelloPayload);
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

	ws.onmessage = (evt: MessageEvent): void => {
		let frame: WsFrame;
		try {
			frame = JSON.parse(evt.data as string);
		} catch {
			return;
		}
		if (frame?.type === "res" && frame.error) {
			frame.error = localizeRpcError(frame.error) as typeof frame.error;
			// When an RPC response indicates auth failure, trigger the
			// auth-status-changed flow so the UI redirects to login
			// instead of showing stale/broken data. Use a flag to
			// avoid dispatching multiple times when several RPCs fail.
			if (frame.error?.code === "UNAUTHORIZED" && !authRedirectPending) {
				authRedirectPending = true;
				window.dispatchEvent(new CustomEvent("moltis:auth-status-changed"));
			}
		}
		if (frame.type === "res" && frame.id && S.pending[frame.id]) {
			S.pending[frame.id](frame as unknown as RpcResponse);
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

	ws.onclose = (): void => {
		const wasConnected = S.connected;
		S.setConnected(false);
		for (const id in S.pending) {
			S.pending[id]({ ok: false, error: { code: "DISCONNECTED", message: "WebSocket disconnected" } });
			delete S.pending[id];
		}
		if (opts.onDisconnected) opts.onDisconnected(wasConnected);

		// If the WebSocket never opened, the server likely rejected the
		// upgrade (e.g. 401). Check auth status and redirect to login
		// instead of endlessly reconnecting.
		if (wasConnected) {
			scheduleReconnect(() => connectWs(opts), backoff);
		} else {
			checkAuthOrReconnect(opts, backoff);
		}
	};

	ws.onerror = (): void => {
		/* handled by onclose */
	};
}

/** Handle server-initiated RPC request (v4). */
function handleServerRequest(ws: WebSocket, frame: WsFrame): void {
	const handler = serverRequestHandlers[frame.method!];
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
		.catch((err: unknown) => {
			ws.send(
				JSON.stringify({
					type: "res",
					id: frame.id,
					ok: false,
					error: { code: "INTERNAL", message: String((err as Error)?.message || err) },
				}),
			);
		});
}

/**
 * Subscribe to events after handshake. Called from websocket.ts.
 */
export function subscribeEvents(events: string[]): Promise<unknown> {
	return sendRpc("subscribe", { events: events });
}

/**
 * When the WebSocket never opened, check `/api/auth/status` to see if
 * the failure was an auth rejection. Redirect to login/onboarding when
 * appropriate; otherwise fall back to normal reconnect.
 */
function checkAuthOrReconnect(opts: ConnectOptions, backoff: BackoffConfig): void {
	fetch("/api/auth/status")
		.then((r) => (r.ok ? (r.json() as Promise<Record<string, unknown>>) : null))
		.then((auth) => {
			if ((auth as Record<string, unknown> | null)?.setup_required) {
				window.location.assign("/onboarding");
			} else if (auth && !(auth as Record<string, unknown>).authenticated) {
				window.location.assign("/login");
			} else {
				scheduleReconnect(() => connectWs(opts), backoff);
			}
		})
		.catch(() => {
			// Auth check itself failed — fall back to normal reconnect.
			scheduleReconnect(() => connectWs(opts), backoff);
		});
}

function scheduleReconnect(reconnect: () => void, backoff: BackoffConfig): void {
	if (reconnectTimer) return;
	reconnectTimer = setTimeout(() => {
		reconnectTimer = null;
		S.setReconnectDelay(Math.min(S.reconnectDelay * backoff.factor, backoff.max));
		reconnect();
	}, S.reconnectDelay);
}

/** Force an immediate reconnect (e.g. on tab visibility change). */
export function forceReconnect(opts?: ConnectOptions): void {
	const resolved = opts || lastOpts;
	if (!resolved || S.connected) return;
	if (reconnectTimer) clearTimeout(reconnectTimer);
	reconnectTimer = null;
	S.setReconnectDelay(1000);
	connectWs(resolved);
}

// ── Shared WebSocket connection with JSON-RPC handshake and reconnect ──
import { localizeRpcError, nextId, sendRpc } from "./helpers";
import { getPreferredLocale } from "./i18n";
import * as S from "./state";
import type { RpcResponse } from "./types";
import type { WsFrame as EventWsFrame } from "./types/ws-events";

let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let lastOpts: ConnectOptions | null = null;
let authRedirectPending = false;

/** Server-request handler: receives arbitrary params, returns arbitrary result. */
type ServerRequestHandler = (params: Record<string, unknown>) => Promise<Record<string, unknown>>;

/** Registry of server-request handlers keyed by method name (v4 bidir RPC). */
const serverRequestHandlers: Record<string, ServerRequestHandler> = {};

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

/** Error detail inside a raw WebSocket frame (before localisation). */
interface WsFrameError {
	code?: string;
	message?: string;
}

/** RPC frame received over the WebSocket (superset of event WsFrame). */
interface WsRpcFrame {
	type: string;
	id?: string;
	method?: string;
	params?: Record<string, unknown>;
	ok?: boolean;
	payload?: HelloPayload | Record<string, unknown>;
	error?: WsFrameError;
	event?: string;
	stream?: unknown;
	done?: unknown;
	channel?: unknown;
}

/** Options for connectWs. */
export interface ConnectOptions {
	onFrame?: (frame: EventWsFrame) => void;
	onConnected?: (hello: HelloPayload) => void | Promise<void>;
	onHandshakeFailed?: (frame: WsRpcFrame) => void;
	onDisconnected?: (wasConnected: boolean) => void;
	backoff?: Partial<BackoffConfig>;
}

/**
 * Register a handler for server-initiated RPC requests (v4 bidirectional RPC).
 * @param method - method name (e.g. "node.invoke")
 * @param handler - returns result or throws
 * @returns unregister function
 */
export function onServerRequest(method: string, handler: ServerRequestHandler): () => void {
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
		// The handshake callback receives an RpcResponse from the pending map.
		// The payload is a HelloPayload on success.
		S.pending[id] = (res: RpcResponse): void => {
			if (res.ok && res.payload) {
				const hello = res.payload as HelloPayload;
				if (hello.type === "hello-ok") {
					S.setConnected(true);
					S.setReconnectDelay(1000);
					if (opts.onConnected) opts.onConnected(hello);
					return;
				}
			}
			S.setConnected(false);
			if (opts.onHandshakeFailed) {
				opts.onHandshakeFailed({
					type: "res",
					ok: res.ok,
					payload: res.payload as HelloPayload | Record<string, unknown>,
					error: res.error,
				});
			} else {
				ws.close();
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
		let frame: WsRpcFrame;
		try {
			frame = JSON.parse(evt.data as string);
		} catch {
			return;
		}
		if (frame?.type === "res" && frame.error) {
			frame.error = localizeRpcError(frame.error) as WsFrameError;
			// When an RPC response indicates auth failure, trigger the
			// auth-status-changed flow so the UI redirects to login
			// instead of showing stale/broken data. Use a flag to
			// avoid dispatching multiple times when several RPCs fail.
			if (frame.error?.code === "UNAUTHORIZED" && !authRedirectPending) {
				authRedirectPending = true;
				window.dispatchEvent(new CustomEvent("moltis:auth-status-changed"));
			}
		}
		if (frame.type === "res" && frame.id && Object.hasOwn(S.pending, frame.id)) {
			S.pending[frame.id]({
				ok: frame.ok ?? false,
				payload: frame.payload,
				error: frame.error as RpcResponse["error"],
			});
			delete S.pending[frame.id];
			return;
		}
		// Handle server-initiated RPC requests (v4 bidirectional RPC).
		if (frame.type === "req" && frame.id && frame.method) {
			handleServerRequest(ws, frame);
			return;
		}
		// Non-RPC frames are event broadcasts; cast to the event-specific shape.
		if (opts.onFrame) opts.onFrame(frame as unknown as EventWsFrame);
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
function handleServerRequest(ws: WebSocket, frame: WsRpcFrame): void {
	const method = frame.method ?? "";
	if (!Object.hasOwn(serverRequestHandlers, method)) {
		ws.send(
			JSON.stringify({
				type: "res",
				id: frame.id,
				ok: false,
				error: { code: "UNKNOWN_METHOD", message: `no handler for ${method}` },
			}),
		);
		return;
	}
	const handler = serverRequestHandlers[method];
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

/** Shape of the /api/auth/status JSON response. */
interface AuthStatusResponse {
	authenticated?: boolean;
	setup_required?: boolean;
}

/**
 * When the WebSocket never opened, check `/api/auth/status` to see if
 * the failure was an auth rejection. Redirect to login/onboarding when
 * appropriate; otherwise fall back to normal reconnect.
 */
function checkAuthOrReconnect(opts: ConnectOptions, backoff: BackoffConfig): void {
	fetch("/api/auth/status")
		.then((r) => (r.ok ? (r.json() as Promise<AuthStatusResponse>) : null))
		.then((auth) => {
			if (auth?.setup_required) {
				window.location.assign("/onboarding");
			} else if (auth && !auth.authenticated) {
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

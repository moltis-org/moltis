// ── RPC helper types ────────────────────────────────────────────
//
// Mirrors the WebSocket RPC frame shape used by `ws-connect.js` and
// `helpers.js`. The server sends `{ type: "res", id, ok, payload?, error? }`.

/** Error detail inside an RPC response. */
export interface RpcError {
	code: string;
	message: string;
}

/**
 * Generic RPC response envelope.
 *
 * Successful responses have `ok: true` and `payload` with the result.
 * Failed responses have `ok: false` and `error` with a code + message.
 */
export interface RpcResponse<T = unknown> {
	ok: boolean;
	payload?: T;
	error?: RpcError;
}

/**
 * Full RPC frame as received over the WebSocket.
 * Extends the response envelope with the wire-level fields.
 */
export interface RpcFrame<T = unknown> extends RpcResponse<T> {
	type: "res";
	id: string;
}

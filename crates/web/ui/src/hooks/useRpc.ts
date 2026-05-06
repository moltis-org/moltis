// ── useRpc hook ─────────────────────────────────────────────
//
// Encapsulates the useEffect + sendRpc + loading/error/data state
// pattern that repeats across every settings section and page.
//
// Usage:
//   const { data, loading, error, refetch } = useRpc("models.list", {});
//   if (loading) return <Loading />;
//   if (error) return <StatusMessage error={error} />;
//   return <div>{data?.map(...)}</div>;

import { useCallback, useEffect, useState } from "preact/hooks";
import { sendRpc } from "../helpers";
import type { RpcMethod, RpcMethodMap } from "../types/rpc-methods";

interface UseRpcResult<T> {
	/** Response payload (undefined until first successful load) */
	data: T | undefined;
	/** True while the RPC call is in flight */
	loading: boolean;
	/** Error message if the call failed */
	error: string | null;
	/** Re-fetch the data */
	refetch: () => void;
}

/**
 * Fetch data from an RPC method on mount (and when params change).
 *
 * @param method - The RPC method name (type-checked against RpcMethodMap)
 * @param params - Parameters to send with the RPC call
 * @param opts - Options: `skip` to conditionally skip the fetch
 */
export function useRpc<M extends RpcMethod>(
	method: M,
	params: Record<string, unknown>,
	opts?: { skip?: boolean },
): UseRpcResult<RpcMethodMap[M]> {
	const [data, setData] = useState<RpcMethodMap[M] | undefined>(undefined);
	const [loading, setLoading] = useState(!opts?.skip);
	const [error, setError] = useState<string | null>(null);

	// Stable serialized key for params to avoid infinite re-fetch loops
	const paramsKey = JSON.stringify(params);

	const doFetch = useCallback(() => {
		if (opts?.skip) return;
		setLoading(true);
		setError(null);
		sendRpc(method, params).then((res) => {
			if (res?.ok) {
				setData(res.payload as RpcMethodMap[M]);
			} else {
				setError(res?.error?.message ?? "Request failed");
			}
			setLoading(false);
		});
		// eslint-disable-next-line react-hooks/exhaustive-deps
	}, [method, paramsKey, opts?.skip]);

	useEffect(() => {
		doFetch();
	}, [doFetch]);

	return { data, loading, error, refetch: doFetch };
}

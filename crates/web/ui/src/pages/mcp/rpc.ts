import { sendRpc } from "../../helpers";
import type { RpcResponse } from "../../types";

const REPOSITORY_RPC_TIMEOUT_MS = 120_000;

export async function mcpRepositoryRpc<T>(method: string, params: unknown): Promise<T> {
	const timeoutMs = method.startsWith("mcp.repositories.") ? REPOSITORY_RPC_TIMEOUT_MS : undefined;
	const response: RpcResponse<T> = await sendRpc<T>(method, params, timeoutMs);
	if (!response.ok) throw new Error(response.error?.message || `${method} failed`);
	return response.payload as T;
}

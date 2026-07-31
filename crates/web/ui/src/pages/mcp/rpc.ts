import { sendRpc } from "../../helpers";
import type { RpcResponse } from "../../types";

export async function mcpRepositoryRpc<T>(method: string, params: unknown): Promise<T> {
	const response: RpcResponse<T> = await sendRpc<T>(method, params);
	if (!response.ok) throw new Error(response.error?.message || `${method} failed`);
	return response.payload as T;
}

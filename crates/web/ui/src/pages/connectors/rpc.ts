import { sendRpc } from "../../helpers";
import type {
	ChannelType,
	ConnectorDatasetConfig,
	ConnectorProjections,
	HimalayaBackend,
	JsonValue,
} from "../../types/connector";
import type { RpcMethodMap } from "../../types/rpc-methods";

export interface ConnectorRpcParams {
	"connectors.available": Record<string, never>;
	"connectors.accounts.list": Record<string, never>;
	"connectors.accounts.add":
		| {
				kind: "caldav";
				name: string;
				serverUrl: string;
				username: string;
				password: string;
				timeoutSeconds: number;
				allowInsecureHttp: boolean;
				allowPrivateNetwork: boolean;
				enabled: boolean;
		  }
		| {
				kind: "channel_history";
				name: string;
				channelType: ChannelType;
				channelAccountId: string;
				enabled: boolean;
		  }
		| { kind: "gmail"; name: string; enabled: boolean }
		| {
				kind: "himalaya";
				name: string;
				himalayaAccountName: string;
				himalayaBackend: HimalayaBackend;
				enabled: boolean;
		  };
	"connectors.accounts.update":
		| {
				id: string;
				name: string;
				serverUrl: string;
				username: string;
				password?: string;
				timeoutSeconds: number;
				allowInsecureHttp: boolean;
				allowPrivateNetwork: boolean;
				enabled: boolean;
		  }
		| { id: string; name: string; enabled: boolean };
	"connectors.accounts.remove": { id: string };
	"connectors.accounts.test": { id: string };
	"connectors.channel_sources.list": Record<string, never>;
	"connectors.datasets.list": Record<string, never>;
	"connectors.datasets.compile": {
		accountId: string;
		datasetId?: string;
		instruction: string;
		overrides?: JsonValue;
	};
	"connectors.datasets.add": {
		accountId: string;
		instruction: string;
		name: string;
		config: ConnectorDatasetConfig;
		scheduleMinutes?: number | null;
		projections: ConnectorProjections;
		enabled: boolean;
	};
	"connectors.datasets.update": {
		id: string;
		instruction: string;
		name: string;
		config: ConnectorDatasetConfig;
		scheduleMinutes?: number | null;
		projections: ConnectorProjections;
		enabled: boolean;
	};
	"connectors.datasets.remove": { id: string };
	"connectors.datasets.sync": { id: string };
	"connectors.runs.list": { datasetId: string; limit: number };
	"connectors.items.query": {
		datasetId: string;
		limit: number;
		offset: number;
		includeDeleted: boolean;
		text?: string;
	};
}

export type ConnectorRpcMethod = keyof ConnectorRpcParams;

const CONNECTOR_RPC_TIMEOUT_MS: Partial<Record<ConnectorRpcMethod, number>> = {
	"connectors.accounts.test": 5 * 60_000,
	"connectors.datasets.compile": 90_000,
	"connectors.datasets.sync": 5 * 60_000,
};

export async function connectorRpc<M extends ConnectorRpcMethod>(
	method: M,
	params: ConnectorRpcParams[M],
): Promise<RpcMethodMap[M]> {
	const response = await sendRpc<RpcMethodMap[M]>(method, params, { timeoutMs: CONNECTOR_RPC_TIMEOUT_MS[method] });
	if (response.ok && response.payload !== undefined) return response.payload;
	throw new Error(response.error?.serverMessage || response.error?.message || `Connector request failed: ${method}`);
}

/** External chat agent discovered by the gateway. */
export interface ExternalAgentInfo {
	kind: string;
	name: string;
	installed: boolean;
	isAcp: boolean;
	version?: string | null;
}

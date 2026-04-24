// ── Tools section ─────────────────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { sendRpc } from "../../helpers";
import type { RpcResponse } from "./_shared";

interface ToolEntry {
	name?: string;
	description?: string;
}

interface ToolGroup {
	label: string;
	tools: ToolEntry[];
}

interface ToolsContextData {
	session?: { model?: string; provider?: string; label?: string };
	execution?: { mode?: string; promptSymbol?: string };
	sandbox?: { enabled?: boolean; backend?: string };
	tools?: ToolEntry[];
	supportsTools?: boolean;
	mcpDisabled?: boolean;
}

interface NodeInventoryEntry {
	platform?: string;
	[key: string]: unknown;
}

interface RemoteExecSummary {
	pairedNodes: number;
	sshTargets: number;
}

function pluralizeToolsCount(count: number, noun: string): string {
	return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

export function toolsOverviewCategory(name: string | undefined): string {
	if (typeof name !== "string" || !name) return "Core";
	if (name.startsWith("mcp__")) return "MCP";
	if (name === "exec" || name.startsWith("node") || name.startsWith("sandbox") || name.includes("checkpoint")) {
		return "Execution";
	}
	if (name.startsWith("session") || name.startsWith("sessions_")) return "Sessions";
	if (name.startsWith("memory") || name.includes("memory")) return "Memory";
	if (name.startsWith("browser") || name.startsWith("web_") || name.includes("screenshot") || name.includes("fetch")) {
		return "Web & Browser";
	}
	if (name.startsWith("skill") || name.includes("skill")) return "Skills";
	return "Core";
}

export function groupToolsForOverview(tools: ToolEntry[]): ToolGroup[] {
	const grouped = new Map<string, ToolEntry[]>();
	(tools || []).forEach((tool) => {
		const category = toolsOverviewCategory(tool?.name);
		if (!grouped.has(category)) grouped.set(category, []);
		grouped.get(category)?.push(tool);
	});
	const order = ["Execution", "Sessions", "Memory", "Web & Browser", "Skills", "MCP", "Core"];
	return order
		.filter((label) => grouped.has(label))
		.map((label) => ({
			label,
			tools:
				grouped
					.get(label)
					?.slice()
					.sort((left, right) => String(left?.name || "").localeCompare(String(right?.name || ""))) ?? [],
		}));
}

function summarizeRemoteExecInventory(entries: NodeInventoryEntry[]): RemoteExecSummary {
	const summary: RemoteExecSummary = { pairedNodes: 0, sshTargets: 0 };
	(entries || []).forEach((entry) => {
		if (!entry || typeof entry !== "object") return;
		if (entry.platform === "ssh") {
			summary.sshTargets += 1;
			return;
		}
		summary.pairedNodes += 1;
	});
	return summary;
}

export function ToolsSection(): VNode {
	const [loadingTools, setLoadingTools] = useState(true);
	const [toolData, setToolData] = useState<ToolsContextData | null>(null);
	const [nodeInventory, setNodeInventory] = useState<NodeInventoryEntry[]>([]);
	const [toolsErr, setToolsErr] = useState<string | null>(null);

	function loadToolsOverview(): void {
		setLoadingTools(true);
		setToolsErr(null);
		Promise.allSettled([sendRpc("chat.context", {}), sendRpc("node.list", {})])
			.then((results) => {
				const contextResult = results[0];
				if (contextResult.status !== "fulfilled" || !(contextResult.value as RpcResponse)?.ok) {
					const errValue = contextResult.status === "fulfilled" ? (contextResult.value as RpcResponse) : null;
					throw new Error(errValue?.error?.message || "Failed to load tools overview.");
				}
				const nextToolData = ((contextResult.value as RpcResponse).payload || {}) as ToolsContextData;
				const nodesResult = results[1];
				const nextNodeInventory =
					nodesResult.status === "fulfilled" &&
					(nodesResult.value as RpcResponse)?.ok &&
					Array.isArray((nodesResult.value as RpcResponse).payload)
						? ((nodesResult.value as RpcResponse).payload as NodeInventoryEntry[])
						: [];
				setToolData(nextToolData);
				setNodeInventory(nextNodeInventory);
				setLoadingTools(false);
			})
			.catch((error: Error) => {
				setLoadingTools(false);
				setToolsErr(error.message);
			});
	}

	useEffect(() => {
		loadToolsOverview();
	}, []);

	const data = toolData || ({} as ToolsContextData);
	const session = data.session || {};
	const execution = data.execution || {};
	const sandbox = data.sandbox || {};
	const tools: ToolEntry[] = Array.isArray(data.tools) ? data.tools : [];
	const toolGroups = groupToolsForOverview(tools);
	const remoteExecInventory = summarizeRemoteExecInventory(nodeInventory);
	const routeDetails: string[] = [];
	routeDetails.push(execution.mode === "sandbox" ? "sandboxed commands" : "host commands");
	if (remoteExecInventory.pairedNodes > 0) {
		routeDetails.push(pluralizeToolsCount(remoteExecInventory.pairedNodes, "paired node"));
	}
	if (remoteExecInventory.sshTargets > 0) {
		routeDetails.push(pluralizeToolsCount(remoteExecInventory.sshTargets, "SSH target"));
	}
	if (remoteExecInventory.pairedNodes === 0 && remoteExecInventory.sshTargets === 0) {
		routeDetails.push("local only");
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div className="flex items-start justify-between gap-3 flex-wrap max-w-[1100px]">
				<div className="min-w-0">
					<h2 className="text-lg font-medium text-[var(--text-strong)]">Tools</h2>
					<p className="text-xs text-[var(--muted)] mt-1 max-w-[900px] leading-relaxed">
						This page shows the effective tool inventory for the active session and model. Change the current LLM,
						disable MCP for a session, or switch execution routes and the inventory here will change with it.
					</p>
				</div>
				<button
					type="button"
					className="provider-btn provider-btn-secondary"
					onClick={loadToolsOverview}
					disabled={loadingTools}
				>
					{loadingTools ? "Refreshing\u2026" : "Refresh"}
				</button>
			</div>

			{toolsErr ? <div className="text-xs text-[var(--error)] max-w-[1100px]">{toolsErr}</div> : null}

			<div className="grid gap-4 md:grid-cols-2 max-w-[1100px]">
				<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
					<div className="text-xs uppercase tracking-wide text-[var(--muted)]">Tool Calling</div>
					<div className="mt-2 flex items-center gap-2 flex-wrap">
						<span className={`provider-item-badge ${data.supportsTools === false ? "warning" : "configured"}`}>
							{data.supportsTools === false ? "Disabled" : "Enabled"}
						</span>
						<span className="text-sm font-medium text-[var(--text)]">
							{tools.length} registered tool{tools.length === 1 ? "" : "s"}
						</span>
					</div>
					<div className="text-xs text-[var(--muted)] mt-2 leading-relaxed">
						{data.supportsTools === false
							? "The current model is chat-only, so the agent cannot call tools in this session."
							: "Built-in, MCP, and runtime-routed tools available to the active model."}
					</div>
				</div>

				<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
					<div className="text-xs uppercase tracking-wide text-[var(--muted)]">Active Model</div>
					<div className="mt-2 text-sm font-medium text-[var(--text)] break-words">
						{session.model || "Default model selection"}
					</div>
					<div className="text-xs text-[var(--muted)] mt-2 leading-relaxed">
						{session.provider ? `Provider: ${session.provider}` : "Provider selected automatically."}
						{session.label ? ` Session: ${session.label}.` : ""}
					</div>
				</div>

				<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4">
					<div className="text-xs uppercase tracking-wide text-[var(--muted)]">Execution Routes</div>
					<div className="mt-2 text-sm font-medium text-[var(--text)]">{routeDetails.join(" \u00b7 ")}</div>
					<div className="text-xs text-[var(--muted)] mt-2 leading-relaxed">
						{sandbox.enabled ? `Sandbox backend: ${sandbox.backend || "configured"}. ` : ""}
						{execution.promptSymbol ? `Prompt symbol: ${execution.promptSymbol}. ` : ""}
						The <code className="text-[var(--text)]">exec</code> tool uses these routes rather than exposing SSH as a
						separate command runner.
					</div>
				</div>
			</div>

			{data.supportsTools === false ? (
				<div className="rounded border border-[var(--warn)] bg-[var(--surface2)] p-3 max-w-[1100px]">
					<div className="text-xs text-[var(--muted)] leading-relaxed">
						Tools are unavailable because the current model does not support tool calling. Switch to a tool-capable
						model in <strong className="text-[var(--text)]">Settings {"\u2192"} LLMs</strong> and refresh this page.
					</div>
				</div>
			) : null}

			<div className="rounded border border-[var(--border)] bg-[var(--surface)] p-4 max-w-[1100px]">
				<div className="flex items-center justify-between gap-2 flex-wrap">
					<h3 className="text-sm font-medium text-[var(--text-strong)] m-0">Registered Tools</h3>
					<span className="provider-item-badge muted">{tools.length}</span>
				</div>
				{toolGroups.length > 0 ? (
					<div className="mt-3 flex flex-col gap-3">
						{toolGroups.map((group) => (
							<div key={group.label}>
								<div className="text-xs uppercase tracking-wide text-[var(--muted)] mb-2">
									{group.label} {"\u00b7"} {group.tools.length}
								</div>
								<div className="flex flex-col gap-2">
									{group.tools.map((tool) => (
										<div key={tool.name} className="rounded border border-[var(--border)] bg-[var(--surface2)] p-3">
											<div className="flex items-center justify-between gap-2 flex-wrap">
												<div className="text-xs font-medium text-[var(--text)] break-words">{tool.name}</div>
												{tool.name?.startsWith("mcp__") ? (
													<span className="provider-item-badge configured">MCP</span>
												) : null}
											</div>
											<div className="text-xs text-[var(--muted)] mt-1 leading-relaxed">
												{tool.description || "No description provided."}
											</div>
										</div>
									))}
								</div>
							</div>
						))}
					</div>
				) : (
					<div className="text-xs text-[var(--muted)] mt-3">No tools are currently exposed to this session.</div>
				)}
			</div>
		</div>
	);
}

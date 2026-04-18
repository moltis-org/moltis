// ── RunDetail Preact component ───────────────────────────
//
// Expand/collapse panel showing tool calls and message flow
// for a specific agent run. Lazy-loads data via RPC.

import type { VNode } from "preact";
import { useCallback, useState } from "preact/hooks";
import { TabBar } from "../components/forms";
import { sendRpc } from "../helpers";

// ── Types ────────────────────────────────────────────────────

interface RunMessage {
	role: string;
	content?: string;
	model?: string;
	provider?: string;
	inputTokens?: number;
	outputTokens?: number;
	tool_name?: string;
	success?: boolean;
	arguments?: Record<string, unknown>;
	error?: string;
}

interface RunSummary {
	userMessages?: number;
	toolCalls?: number;
	assistantMessages?: number;
}

interface RunData {
	summary?: RunSummary;
	messages?: RunMessage[];
}

interface TabProps {
	data: RunData | null;
}

export interface RunDetailProps {
	sessionKey: string;
	runId: string;
}

// ── Sub-components ───────────────────────────────────────────

const RUN_DETAIL_TABS = [
	{ id: "overview", label: "Overview" },
	{ id: "actions", label: "Actions" },
	{ id: "messages", label: "Messages" },
];

function OverviewTab({ data }: TabProps): VNode | null {
	if (!data) return null;
	const summary = data.summary || {};
	const messages = data.messages || [];
	let model: string | null = null;
	let provider: string | null = null;
	let totalInput = 0;
	let totalOutput = 0;
	for (const m of messages) {
		if (m.role === "assistant") {
			if (m.model) model = m.model;
			if (m.provider) provider = m.provider;
			totalInput += m.inputTokens || 0;
			totalOutput += m.outputTokens || 0;
		}
	}
	return (
		<div className="flex flex-col gap-1 text-xs">
			<div className="flex gap-4">
				<span className="text-[var(--muted)]">User messages:</span>
				<span className="font-medium">{summary.userMessages || 0}</span>
			</div>
			<div className="flex gap-4">
				<span className="text-[var(--muted)]">Tool calls:</span>
				<span className="font-medium">{summary.toolCalls || 0}</span>
			</div>
			<div className="flex gap-4">
				<span className="text-[var(--muted)]">Assistant messages:</span>
				<span className="font-medium">{summary.assistantMessages || 0}</span>
			</div>
			{model ? (
				<div className="flex gap-4">
					<span className="text-[var(--muted)]">Model:</span>
					<span className="font-medium">
						{provider ? `${provider} / ` : ""}
						{model}
					</span>
				</div>
			) : null}
			{totalInput + totalOutput > 0 ? (
				<div className="flex gap-4">
					<span className="text-[var(--muted)]">Tokens:</span>
					<span className="font-medium">
						{totalInput} in / {totalOutput} out
					</span>
				</div>
			) : null}
		</div>
	);
}

function ActionsTab({ data }: TabProps): VNode | null {
	if (!data) return null;
	const toolResults = (data.messages || []).filter((m) => m.role === "tool_result");
	if (toolResults.length === 0) return <div className="text-xs text-[var(--muted)]">No tool calls in this run.</div>;
	return (
		<div className="flex flex-col gap-2">
			{toolResults.map((tr, i) => (
				<div key={i} className="border border-[var(--border)] rounded-md p-2 bg-[var(--surface)] text-xs">
					<div className="flex items-center gap-2">
						<span className="font-semibold">{tr.tool_name || "unknown"}</span>
						<span className={tr.success ? "text-green-500" : "text-red-500"}>{tr.success ? "ok" : "error"}</span>
					</div>
					{tr.arguments ? (
						<pre className="mt-1 font-mono whitespace-pre-wrap break-words text-[var(--muted)]">
							{JSON.stringify(tr.arguments, null, 2)}
						</pre>
					) : null}
					{tr.error ? <div className="mt-1 text-red-500">{tr.error}</div> : null}
				</div>
			))}
		</div>
	);
}

function MessagesTab({ data }: TabProps): VNode | null {
	if (!data) return null;
	const messages = data.messages || [];
	if (messages.length === 0) return <div className="text-xs text-[var(--muted)]">No messages.</div>;
	return (
		<div className="flex flex-col gap-1">
			{messages.map((m, i) => (
				<div key={i} className="border-b border-[var(--border)] pb-1 text-xs">
					<span className="font-semibold uppercase text-[var(--muted)]" style={{ fontSize: "10px" }}>
						{m.role}
					</span>
					<span className="text-[var(--muted)] ml-1">#{i}</span>
					{typeof m.content === "string" && m.content ? (
						<div className="mt-0.5 font-mono whitespace-pre-wrap break-words max-h-32 overflow-auto">
							{m.content.length > 500 ? `${m.content.slice(0, 500)}\u2026` : m.content}
						</div>
					) : null}
				</div>
			))}
		</div>
	);
}

// ── Main component ───────────────────────────────────────────

export function RunDetail({ sessionKey, runId }: RunDetailProps): VNode {
	const [expanded, setExpanded] = useState(false);
	const [data, setData] = useState<RunData | null>(null);
	const [loading, setLoading] = useState(false);
	const [activeTab, setActiveTab] = useState<string>("overview");

	const toggle = useCallback(() => {
		const next = !expanded;
		setExpanded(next);
		if (next && !data && !loading) {
			setLoading(true);
			sendRpc<RunData>("sessions.run_detail", { sessionKey, runId }).then((res) => {
				setLoading(false);
				if (res?.ok && res.payload) {
					setData(res.payload);
				}
			});
		}
	}, [expanded, data, loading, sessionKey, runId]);

	return (
		<div className="mt-1">
			<button
				className="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
				onClick={toggle}
			>
				{expanded ? "\u25bc" : "\u25b6"} Run details
			</button>
			{expanded ? (
				<div className="mt-2 border border-[var(--border)] rounded-md p-3 bg-[var(--bg)]">
					{loading ? (
						<div className="text-xs text-[var(--muted)]">Loading\u2026</div>
					) : (
						<div>
							<TabBar tabs={RUN_DETAIL_TABS} active={activeTab} onChange={setActiveTab} />
							{activeTab === "overview" && <OverviewTab data={data} />}
							{activeTab === "actions" && <ActionsTab data={data} />}
							{activeTab === "messages" && <MessagesTab data={data} />}
						</div>
					)}
				</div>
			) : null}
		</div>
	);
}

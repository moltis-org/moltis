// ── RunDetail Preact component ───────────────────────────
//
// Expand/collapse panel showing tool calls and message flow
// for a specific agent run. Lazy-loads data via RPC.

import { html } from "htm/preact";
import { useCallback, useState } from "preact/hooks";
import { sendRpc } from "../helpers.js";

var TABS = ["overview", "actions", "messages"];

function TabButton({ label, active, onClick }) {
	return html`<button
		class="text-xs px-2 py-1 rounded-md transition-colors cursor-pointer bg-transparent border ${
			active
				? "border-[var(--accent)] text-[var(--accent)] font-semibold"
				: "border-[var(--border)] text-[var(--muted)]"
		}"
		onClick=${onClick}
	>
		${label}
	</button>`;
}

function OverviewTab({ data }) {
	if (!data) return null;
	var summary = data.summary || {};
	var messages = data.messages || [];
	var model = null;
	var provider = null;
	var totalInput = 0;
	var totalOutput = 0;
	for (var m of messages) {
		if (m.role === "assistant") {
			if (m.model) model = m.model;
			if (m.provider) provider = m.provider;
			totalInput += m.inputTokens || 0;
			totalOutput += m.outputTokens || 0;
		}
	}
	return html`<div class="flex flex-col gap-1 text-xs">
		<div class="flex gap-4">
			<span class="text-[var(--muted)]">User messages:</span>
			<span class="font-medium">${summary.userMessages || 0}</span>
		</div>
		<div class="flex gap-4">
			<span class="text-[var(--muted)]">Tool calls:</span>
			<span class="font-medium">${summary.toolCalls || 0}</span>
		</div>
		<div class="flex gap-4">
			<span class="text-[var(--muted)]">Assistant messages:</span>
			<span class="font-medium">${summary.assistantMessages || 0}</span>
		</div>
		${
			model
				? html`<div class="flex gap-4">
					<span class="text-[var(--muted)]">Model:</span>
					<span class="font-medium">${provider ? `${provider} / ` : ""}${model}</span>
				</div>`
				: null
		}
		${
			totalInput + totalOutput > 0
				? html`<div class="flex gap-4">
					<span class="text-[var(--muted)]">Tokens:</span>
					<span class="font-medium">${totalInput} in / ${totalOutput} out</span>
				</div>`
				: null
		}
	</div>`;
}

function ActionsTab({ data }) {
	if (!data) return null;
	var toolResults = (data.messages || []).filter((m) => m.role === "tool_result");
	if (toolResults.length === 0) return html`<div class="text-xs text-[var(--muted)]">No tool calls in this run.</div>`;
	return html`<div class="flex flex-col gap-2">
		${toolResults.map(
			(tr) =>
				html`<div
					class="border border-[var(--border)] rounded-md p-2 bg-[var(--surface)] text-xs"
				>
					<div class="flex items-center gap-2">
						<span class="font-semibold">${tr.tool_name || "unknown"}</span>
						<span class="${tr.success ? "text-green-500" : "text-red-500"}"
							>${tr.success ? "ok" : "error"}</span
						>
					</div>
					${
						tr.arguments
							? html`<pre
								class="mt-1 font-mono whitespace-pre-wrap break-words text-[var(--muted)]"
							>
${JSON.stringify(tr.arguments, null, 2)}</pre
							>`
							: null
					}
					${tr.error ? html`<div class="mt-1 text-red-500">${tr.error}</div>` : null}
				</div>`,
		)}
	</div>`;
}

function MessagesTab({ data }) {
	if (!data) return null;
	var messages = data.messages || [];
	if (messages.length === 0) return html`<div class="text-xs text-[var(--muted)]">No messages.</div>`;
	return html`<div class="flex flex-col gap-1">
		${messages.map(
			(m, i) =>
				html`<div class="border-b border-[var(--border)] pb-1 text-xs">
					<span
						class="font-semibold uppercase text-[var(--muted)]"
						style="font-size:10px"
						>${m.role}</span
					>
					<span class="text-[var(--muted)] ml-1">#${i}</span>
					${
						typeof m.content === "string" && m.content
							? html`<div
								class="mt-0.5 font-mono whitespace-pre-wrap break-words max-h-32 overflow-auto"
							>
								${m.content.length > 500 ? `${m.content.slice(0, 500)}\u2026` : m.content}
							</div>`
							: null
					}
				</div>`,
		)}
	</div>`;
}

export function RunDetail({ sessionKey, runId }) {
	var [expanded, setExpanded] = useState(false);
	var [data, setData] = useState(null);
	var [loading, setLoading] = useState(false);
	var [activeTab, setActiveTab] = useState("overview");

	var toggle = useCallback(() => {
		var next = !expanded;
		setExpanded(next);
		if (next && !data && !loading) {
			setLoading(true);
			sendRpc("sessions.run_detail", { sessionKey, runId }).then((res) => {
				setLoading(false);
				if (res?.ok && res.payload) {
					setData(res.payload);
				}
			});
		}
	}, [expanded, data, loading, sessionKey, runId]);

	return html`<div class="mt-1">
		<button
			class="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
			onClick=${toggle}
		>
			${expanded ? "\u25bc" : "\u25b6"} Run details
		</button>
		${
			expanded
				? html`<div
					class="mt-2 border border-[var(--border)] rounded-md p-3 bg-[var(--bg)]"
				>
					${
						loading
							? html`<div class="text-xs text-[var(--muted)]">Loading\u2026</div>`
							: html`<div>
								<div class="flex gap-1 mb-2">
									${TABS.map(
										(t) =>
											html`<${TabButton}
												label=${t.charAt(0).toUpperCase() + t.slice(1)}
												active=${activeTab === t}
												onClick=${() => setActiveTab(t)}
											/>`,
									)}
								</div>
								${activeTab === "overview" && html`<${OverviewTab} data=${data} />`}
								${activeTab === "actions" && html`<${ActionsTab} data=${data} />`}
								${activeTab === "messages" && html`<${MessagesTab} data=${data} />`}
							</div>`
					}
				</div>`
				: null
		}
	</div>`;
}

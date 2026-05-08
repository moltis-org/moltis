// ── Network Audit page (Preact toolbar + imperative entry area) ──

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef } from "preact/hooks";
import { sendRpc } from "../helpers";
import * as S from "../state";
import { ComboSelect } from "../ui";

interface AuditEntry {
	timestamp: string;
	protocol: string;
	action: string;
	domain: string;
	port: number;
	method?: string;
	url?: string;
	bytes_sent: number;
	bytes_received: number;
	duration_ms: number;
	error?: string;
}

interface ComboOption {
	value: string;
	label: string;
}

const paused = signal(false);
const domainFilter = signal("");
const protocolFilter = signal("");
const actionFilter = signal("");
const entryCount = signal(0);
const maxEntries = 2000;

function actionColor(action: string): string {
	if (action === "allowed" || action === "approved_by_user") return "var(--ok, #22c55e)";
	if (action === "denied") return "var(--error, #ef4444)";
	if (action === "timeout") return "var(--warn, #f59e0b)";
	return "var(--text)";
}

function actionBg(action: string): string {
	if (action === "denied") return "rgba(239,68,68,0.08)";
	if (action === "timeout") return "rgba(245,158,11,0.06)";
	return "transparent";
}

function formatBytes(n: number): string {
	if (n < 1024) return `${n}B`;
	if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
	return `${(n / (1024 * 1024)).toFixed(1)}MB`;
}

function renderEntry(entry: AuditEntry): HTMLDivElement {
	const row = document.createElement("div");
	row.className = "logs-row";
	row.style.background = actionBg(entry.action);

	// Timestamp
	const ts = document.createElement("span");
	ts.className = "logs-ts";
	const d = new Date(entry.timestamp);
	ts.textContent =
		d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }) +
		"." +
		String(d.getMilliseconds()).padStart(3, "0");

	// Protocol badge
	const proto = document.createElement("span");
	proto.className = "logs-level";
	proto.style.color = "var(--accent, #3b82f6)";
	proto.textContent = entry.protocol === "http_connect" ? "CONNECT" : "HTTP";

	// Action badge
	const act = document.createElement("span");
	act.className = "logs-level";
	act.style.color = actionColor(entry.action);
	act.textContent =
		entry.action === "approved_by_user"
			? "\u2713"
			: entry.action === "allowed"
				? "\u2713"
				: entry.action === "denied"
					? "\u2717"
					: "\u29D6";

	// Domain
	const dom = document.createElement("span");
	dom.className = "logs-target";
	dom.textContent = `${entry.domain}:${entry.port}`;

	// Details
	const details = document.createElement("span");
	details.className = "logs-msg";
	const parts: string[] = [];
	if (entry.method) parts.push(entry.method);
	if (entry.url) parts.push(entry.url);
	parts.push(`${formatBytes(entry.bytes_sent)}\u2191`);
	parts.push(`${formatBytes(entry.bytes_received)}\u2193`);
	parts.push(`${entry.duration_ms}ms`);
	if (entry.error) parts.push(`ERR: ${entry.error}`);
	details.textContent = parts.join("  ");

	row.appendChild(ts);
	row.appendChild(proto);
	row.appendChild(act);
	row.appendChild(dom);
	row.appendChild(details);
	return row;
}

function Toolbar(): VNode {
	const domainRef = useRef<HTMLInputElement>(null);
	const filterTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

	function debouncedDomain(): void {
		if (filterTimer.current != null) clearTimeout(filterTimer.current);
		filterTimer.current = setTimeout(() => {
			domainFilter.value = domainRef.current?.value || "";
		}, 300);
	}

	const protocolOptions: ComboOption[] = [
		{ value: "http_connect", label: "CONNECT" },
		{ value: "http_forward", label: "HTTP" },
	];

	const actionOptions: ComboOption[] = [
		{ value: "allowed", label: "Allowed" },
		{ value: "denied", label: "Denied" },
		{ value: "approved_by_user", label: "Approved" },
		{ value: "timeout", label: "Timeout" },
	];

	return (
		<div className="logs-toolbar">
			<input
				ref={domainRef}
				type="text"
				placeholder={"Filter domain\u2026"}
				className="logs-input"
				style={{ width: "180px" }}
				onInput={debouncedDomain}
			/>
			<div className="logs-level-filter">
				<ComboSelect
					options={protocolOptions}
					value={protocolFilter.value}
					onChange={(v: string) => {
						protocolFilter.value = v;
					}}
					placeholder="All protocols"
					searchable={false}
				/>
			</div>
			<div className="logs-level-filter">
				<ComboSelect
					options={actionOptions}
					value={actionFilter.value}
					onChange={(v: string) => {
						actionFilter.value = v;
					}}
					placeholder="All actions"
					searchable={false}
				/>
			</div>
			<button
				className="logs-btn"
				onClick={() => {
					paused.value = !paused.value;
				}}
				style={paused.value ? { borderColor: "var(--warn)" } : undefined}
			>
				{paused.value ? "Resume" : "Pause"}
			</button>
			<button
				className="logs-btn"
				onClick={() => {
					const area = document.getElementById("networkAuditArea");
					if (area) area.textContent = "";
					entryCount.value = 0;
				}}
			>
				Clear
			</button>
			<span className="logs-count">{entryCount.value} entries</span>
		</div>
	);
}

function NetworkAuditPage(): VNode {
	const areaRef = useRef<HTMLDivElement>(null);

	function appendEntry(entry: AuditEntry): void {
		const area = areaRef.current;
		if (!area) return;
		const row = renderEntry(entry);
		area.appendChild(row);
		entryCount.value++;
		while (area.childNodes.length > maxEntries) {
			area.removeChild(area.firstChild!);
			entryCount.value--;
		}
		if (!paused.value) {
			const atBottom = area.scrollHeight - area.scrollTop - area.clientHeight < 60;
			if (atBottom) area.scrollTop = area.scrollHeight;
		}
	}

	function matchesFilter(entry: AuditEntry): boolean {
		const dVal = domainFilter.value.trim().toLowerCase();
		if (dVal && entry.domain.toLowerCase().indexOf(dVal) === -1) return false;
		if (protocolFilter.value && entry.protocol !== protocolFilter.value) return false;
		if (actionFilter.value && entry.action !== actionFilter.value) return false;
		return true;
	}

	function refetch(): void {
		const area = areaRef.current;
		if (area) area.textContent = "";
		entryCount.value = 0;
		sendRpc("network.audit.list", {
			domain: domainFilter.value.trim() || undefined,
			protocol: protocolFilter.value || undefined,
			action: actionFilter.value || undefined,
			limit: 500,
		}).then((res) => {
			if (!res?.ok) return;
			const entries: AuditEntry[] = (res.payload as { entries?: AuditEntry[] })?.entries || [];
			let i = 0;
			const batchSize = 100;
			function renderBatch(): void {
				const end = Math.min(i + batchSize, entries.length);
				while (i < end) appendEntry(entries[i++]);
				if (i < entries.length) requestAnimationFrame(renderBatch);
				else if (areaRef.current) areaRef.current.scrollTop = areaRef.current.scrollHeight;
			}
			renderBatch();
		});
	}

	useEffect(() => {
		refetch();
		S.setNetworkAuditEventHandler((entry: unknown) => {
			if (paused.value) return;
			const e = entry as AuditEntry;
			if (!matchesFilter(e)) return;
			appendEntry(e);
		});
		return () => S.setNetworkAuditEventHandler(null);
	}, []);

	useEffect(() => {
		refetch();
	}, [domainFilter.value, protocolFilter.value, actionFilter.value]);

	return (
		<>
			<Toolbar />
			<div ref={areaRef} id="networkAuditArea" className="logs-area" />
		</>
	);
}

let _container: HTMLElement | null = null;

export function initNetworkAudit(container: HTMLElement): void {
	_container = container;
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	paused.value = false;
	domainFilter.value = "";
	protocolFilter.value = "";
	actionFilter.value = "";
	entryCount.value = 0;
	render(<NetworkAuditPage />, container);
}

export function teardownNetworkAudit(): void {
	S.setNetworkAuditEventHandler(null);
	if (_container) render(null, _container);
	_container = null;
}

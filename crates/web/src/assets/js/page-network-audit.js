// ── Network Audit page (Preact toolbar + imperative entry area) ──

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef } from "preact/hooks";
import { sendRpc } from "./helpers.js";
import * as S from "./state.js";
import { ComboSelect } from "./ui.js";

var paused = signal(false);
var domainFilter = signal("");
var protocolFilter = signal("");
var actionFilter = signal("");
var entryCount = signal(0);
var maxEntries = 2000;

function actionColor(action) {
	if (action === "allowed" || action === "approved_by_user") return "var(--ok, #22c55e)";
	if (action === "denied") return "var(--error, #ef4444)";
	if (action === "timeout") return "var(--warn, #f59e0b)";
	return "var(--text)";
}

function actionBg(action) {
	if (action === "denied") return "rgba(239,68,68,0.08)";
	if (action === "timeout") return "rgba(245,158,11,0.06)";
	return "transparent";
}

function formatBytes(n) {
	if (n < 1024) return `${n}B`;
	if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
	return `${(n / (1024 * 1024)).toFixed(1)}MB`;
}

function renderEntry(entry) {
	var row = document.createElement("div");
	row.className = "logs-row";
	row.style.background = actionBg(entry.action);

	// Timestamp
	var ts = document.createElement("span");
	ts.className = "logs-ts";
	var d = new Date(entry.timestamp);
	ts.textContent =
		d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }) +
		"." +
		String(d.getMilliseconds()).padStart(3, "0");

	// Protocol badge
	var proto = document.createElement("span");
	proto.className = "logs-level";
	proto.style.color = "var(--accent, #3b82f6)";
	proto.textContent = entry.protocol === "http_connect" ? "CONNECT" : "HTTP";

	// Action badge
	var act = document.createElement("span");
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
	var dom = document.createElement("span");
	dom.className = "logs-target";
	dom.textContent = `${entry.domain}:${entry.port}`;

	// Details
	var details = document.createElement("span");
	details.className = "logs-msg";
	var parts = [];
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

function Toolbar() {
	var domainRef = useRef(null);
	var filterTimer = useRef(null);

	function debouncedDomain() {
		clearTimeout(filterTimer.current);
		filterTimer.current = setTimeout(() => {
			domainFilter.value = domainRef.current?.value || "";
		}, 300);
	}

	return html`<div class="logs-toolbar">
		<input ref=${domainRef} type="text" placeholder="Filter domain\u2026"
			class="logs-input" style="width:180px;"
			onInput=${debouncedDomain} />
		<div class="logs-level-filter">
			<${ComboSelect}
				options=${[{ value: "http_connect", label: "CONNECT" }, { value: "http_forward", label: "HTTP" }]}
				value=${protocolFilter.value}
				onChange=${(v) => {
					protocolFilter.value = v;
				}}
				placeholder="All protocols"
				searchable=${false}
			/>
		</div>
		<div class="logs-level-filter">
			<${ComboSelect}
				options=${[
					{ value: "allowed", label: "Allowed" },
					{ value: "denied", label: "Denied" },
					{ value: "approved_by_user", label: "Approved" },
					{ value: "timeout", label: "Timeout" },
				]}
				value=${actionFilter.value}
				onChange=${(v) => {
					actionFilter.value = v;
				}}
				placeholder="All actions"
				searchable=${false}
			/>
		</div>
		<button class="logs-btn" onClick=${() => {
			paused.value = !paused.value;
		}}
			style=${paused.value ? "border-color:var(--warn);" : ""}>
			${paused.value ? "Resume" : "Pause"}
		</button>
		<button class="logs-btn" onClick=${() => {
			var area = document.getElementById("networkAuditArea");
			if (area) area.textContent = "";
			entryCount.value = 0;
		}}>Clear</button>
		<span class="logs-count">${entryCount.value} entries</span>
	</div>`;
}

function NetworkAuditPage() {
	var areaRef = useRef(null);

	function appendEntry(entry) {
		var area = areaRef.current;
		if (!area) return;
		var row = renderEntry(entry);
		area.appendChild(row);
		entryCount.value++;
		while (area.childNodes.length > maxEntries) {
			area.removeChild(area.firstChild);
			entryCount.value--;
		}
		if (!paused.value) {
			var atBottom = area.scrollHeight - area.scrollTop - area.clientHeight < 60;
			if (atBottom) area.scrollTop = area.scrollHeight;
		}
	}

	function matchesFilter(entry) {
		var dVal = domainFilter.value.trim().toLowerCase();
		if (dVal && entry.domain.toLowerCase().indexOf(dVal) === -1) return false;
		if (protocolFilter.value && entry.protocol !== protocolFilter.value) return false;
		if (actionFilter.value && entry.action !== actionFilter.value) return false;
		return true;
	}

	function refetch() {
		var area = areaRef.current;
		if (area) area.textContent = "";
		entryCount.value = 0;
		sendRpc("network.audit.list", {
			domain: domainFilter.value.trim() || undefined,
			protocol: protocolFilter.value || undefined,
			action: actionFilter.value || undefined,
			limit: 500,
		}).then((res) => {
			if (!res?.ok) return;
			var entries = res.payload?.entries || [];
			var i = 0;
			var batchSize = 100;
			function renderBatch() {
				var end = Math.min(i + batchSize, entries.length);
				while (i < end) appendEntry(entries[i++]);
				if (i < entries.length) requestAnimationFrame(renderBatch);
				else if (areaRef.current) areaRef.current.scrollTop = areaRef.current.scrollHeight;
			}
			renderBatch();
		});
	}

	useEffect(() => {
		refetch();
		S.setNetworkAuditEventHandler((entry) => {
			if (paused.value) return;
			if (!matchesFilter(entry)) return;
			appendEntry(entry);
		});
		return () => S.setNetworkAuditEventHandler(null);
	}, []);

	useEffect(() => {
		refetch();
	}, [domainFilter.value, protocolFilter.value, actionFilter.value]);

	return html`
		<${Toolbar} />
		<div ref=${areaRef} id="networkAuditArea" class="logs-area" />
	`;
}

var _container = null;

export function initNetworkAudit(container) {
	_container = container;
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	paused.value = false;
	domainFilter.value = "";
	protocolFilter.value = "";
	actionFilter.value = "";
	entryCount.value = 0;
	render(html`<${NetworkAuditPage} />`, container);
}

export function teardownNetworkAudit() {
	S.setNetworkAuditEventHandler(null);
	if (_container) render(null, _container);
	_container = null;
}

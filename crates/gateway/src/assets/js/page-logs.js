// ── Logs page (Preact toolbar + imperative log area) ─────────

import { signal } from "@preact/signals";
import { html } from "htm/preact";
import { render } from "preact";
import { useEffect, useRef } from "preact/hooks";
import { sendRpc } from "./helpers.js";
import { registerPage } from "./router.js";
import * as S from "./state.js";

var paused = signal(false);
var levelFilter = signal("");
var targetFilter = signal("");
var searchFilter = signal("");
var entryCount = signal(0);
var maxEntries = 2000;

function levelColor(level) {
	var l = level.toUpperCase();
	if (l === "ERROR") return "var(--error)";
	if (l === "WARN") return "var(--warn)";
	if (l === "DEBUG") return "var(--muted)";
	if (l === "TRACE") return "color-mix(in oklab, var(--muted) 60%, transparent)";
	return "var(--text)";
}

function levelBg(level) {
	var l = level.toUpperCase();
	if (l === "ERROR") return "rgba(239,68,68,0.08)";
	if (l === "WARN") return "rgba(245,158,11,0.06)";
	return "transparent";
}

function renderEntry(entry) {
	var row = document.createElement("div");
	row.className = "logs-row";
	row.style.background = levelBg(entry.level);
	var ts = document.createElement("span");
	ts.className = "logs-ts";
	var d = new Date(entry.ts);
	ts.textContent =
		d.toLocaleTimeString([], {
			hour: "2-digit",
			minute: "2-digit",
			second: "2-digit",
		}) +
		"." +
		String(d.getMilliseconds()).padStart(3, "0");
	var lvl = document.createElement("span");
	lvl.className = "logs-level";
	lvl.style.color = levelColor(entry.level);
	lvl.textContent = entry.level.toUpperCase().substring(0, 5);
	var tgt = document.createElement("span");
	tgt.className = "logs-target";
	tgt.textContent = entry.target;
	var msg = document.createElement("span");
	msg.className = "logs-msg";
	msg.textContent = entry.message;
	if (entry.fields && Object.keys(entry.fields).length > 0) {
		msg.textContent +=
			" " +
			Object.keys(entry.fields)
				.map((k) => `${k}=${entry.fields[k]}`)
				.join(" ");
	}
	row.appendChild(ts);
	row.appendChild(lvl);
	row.appendChild(tgt);
	row.appendChild(msg);
	return row;
}

function Toolbar() {
	var targetRef = useRef(null);
	var searchRef = useRef(null);
	var filterTimer = useRef(null);

	function debouncedUpdate(setter, ref) {
		return () => {
			clearTimeout(filterTimer.current);
			filterTimer.current = setTimeout(() => {
				setter(ref.current?.value || "");
			}, 300);
		};
	}

	return html`<div class="logs-toolbar">
    <select class="logs-select" value=${levelFilter.value}
      onChange=${(e) => {
				levelFilter.value = e.target.value;
			}}>
      <option value="">All levels</option>
      <option value="trace">TRACE</option>
      <option value="debug">DEBUG</option>
      <option value="info">INFO</option>
      <option value="warn">WARN</option>
      <option value="error">ERROR</option>
    </select>
    <input ref=${targetRef} type="text" placeholder="Filter target\u2026"
      class="logs-input" style="width:140px;"
      onInput=${debouncedUpdate((v) => {
				targetFilter.value = v;
			}, targetRef)} />
    <input ref=${searchRef} type="text" placeholder="Search\u2026"
      class="logs-input" style="width:160px;"
      onInput=${debouncedUpdate((v) => {
				searchFilter.value = v;
			}, searchRef)} />
    <button class="logs-btn" onClick=${() => {
			paused.value = !paused.value;
		}}
      style=${paused.value ? "border-color:var(--warn);" : ""}>
      ${paused.value ? "Resume" : "Pause"}
    </button>
    <button class="logs-btn" onClick=${() => {
			var area = document.getElementById("logsArea");
			if (area) area.textContent = "";
			entryCount.value = 0;
		}}>Clear</button>
    <a href="/api/logs/download" class="logs-btn" download="moltis-logs.jsonl"
      style="text-decoration:none;text-align:center;">Download</a>
    <span class="logs-count">${entryCount.value} entries</span>
  </div>`;
}

function LogsPage() {
	var logAreaRef = useRef(null);

	function appendEntry(entry) {
		var area = logAreaRef.current;
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
		if (levelFilter.value) {
			var levels = ["trace", "debug", "info", "warn", "error"];
			if (levels.indexOf(entry.level.toLowerCase()) < levels.indexOf(levelFilter.value)) return false;
		}
		var tgtVal = targetFilter.value.trim();
		if (tgtVal && entry.target.indexOf(tgtVal) === -1) return false;
		var searchVal = searchFilter.value.trim().toLowerCase();
		if (
			searchVal &&
			entry.message.toLowerCase().indexOf(searchVal) === -1 &&
			entry.target.toLowerCase().indexOf(searchVal) === -1
		)
			return false;
		return true;
	}

	function refetch() {
		var area = logAreaRef.current;
		if (area) area.textContent = "";
		entryCount.value = 0;
		sendRpc("logs.list", {
			level: levelFilter.value || undefined,
			target: targetFilter.value.trim() || undefined,
			search: searchFilter.value.trim() || undefined,
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
				else if (logAreaRef.current) logAreaRef.current.scrollTop = logAreaRef.current.scrollHeight;
			}
			renderBatch();
		});
	}

	useEffect(() => {
		refetch();
		S.setLogsEventHandler((entry) => {
			if (paused.value) return;
			if (!matchesFilter(entry)) return;
			appendEntry(entry);
		});
		return () => S.setLogsEventHandler(null);
	}, []);

	// Re-fetch when filters change
	useEffect(() => {
		refetch();
	}, [levelFilter.value, targetFilter.value, searchFilter.value]);

	return html`
    <${Toolbar} />
    <div ref=${logAreaRef} id="logsArea" class="logs-area" />
  `;
}

registerPage(
	"/logs",
	function initLogs(container) {
		container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
		paused.value = false;
		levelFilter.value = "";
		targetFilter.value = "";
		searchFilter.value = "";
		entryCount.value = 0;
		render(html`<${LogsPage} />`, container);
	},
	function teardownLogs() {
		S.setLogsEventHandler(null);
		var container = S.$("pageContent");
		if (container) render(null, container);
	},
);

// ── Logs page (Preact toolbar + imperative log area) ─────────

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef } from "preact/hooks";
import { sendRpc } from "../helpers";
import { t } from "../i18n";
import * as S from "../state";
import { ComboSelect } from "../ui";

interface LogEntry {
	ts: string;
	level: string;
	target: string;
	message: string;
	fields?: Record<string, unknown>;
}

interface LogsStatusPayload {
	enabled_levels?: Record<string, boolean>;
}

interface ComboOption {
	value: string;
	label: string;
}

const paused = signal(false);
const levelFilter = signal("");
const targetFilter = signal("");
const searchFilter = signal("");
const entryCount = signal(0);
const debugEnabled = signal(false);
const traceEnabled = signal(false);
const maxEntries = 2000;

function levelColor(level: string): string {
	const l = level.toUpperCase();
	if (l === "ERROR") return "var(--error)";
	if (l === "WARN") return "var(--warn)";
	if (l === "DEBUG") return "var(--muted)";
	if (l === "TRACE") return "color-mix(in oklab, var(--muted) 60%, transparent)";
	return "var(--text)";
}

function levelBg(level: string): string {
	const l = level.toUpperCase();
	if (l === "ERROR") return "rgba(239,68,68,0.08)";
	if (l === "WARN") return "rgba(245,158,11,0.06)";
	return "transparent";
}

function renderEntry(entry: LogEntry): HTMLDivElement {
	const row = document.createElement("div");
	row.className = "logs-row";
	row.style.background = levelBg(entry.level);
	const ts = document.createElement("span");
	ts.className = "logs-ts";
	const d = new Date(entry.ts);
	ts.textContent =
		d.toLocaleTimeString([], {
			hour: "2-digit",
			minute: "2-digit",
			second: "2-digit",
		}) +
		"." +
		String(d.getMilliseconds()).padStart(3, "0");
	const lvl = document.createElement("span");
	lvl.className = "logs-level";
	lvl.style.color = levelColor(entry.level);
	lvl.textContent = entry.level.toUpperCase().substring(0, 5);
	const tgt = document.createElement("span");
	tgt.className = "logs-target";
	tgt.textContent = entry.target;
	const msg = document.createElement("span");
	msg.className = "logs-msg";
	msg.textContent = entry.message;
	if (entry.fields && Object.keys(entry.fields).length > 0) {
		msg.textContent +=
			" " +
			Object.keys(entry.fields)
				.map((k) => `${k}=${entry.fields?.[k]}`)
				.join(" ");
	}
	row.appendChild(ts);
	row.appendChild(lvl);
	row.appendChild(tgt);
	row.appendChild(msg);
	return row;
}

function levelFilterOptions(): ComboOption[] {
	const options: ComboOption[] = [];
	if (traceEnabled.value) options.push({ value: "trace", label: t("logs:levels.trace") });
	if (debugEnabled.value) options.push({ value: "debug", label: t("logs:levels.debug") });
	options.push({ value: "info", label: t("logs:levels.info") });
	options.push({ value: "warn", label: t("logs:levels.warn") });
	options.push({ value: "error", label: t("logs:levels.error") });
	return options;
}

function Toolbar(): VNode {
	const targetRef = useRef<HTMLInputElement>(null);
	const searchRef = useRef<HTMLInputElement>(null);
	const filterTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

	function debouncedUpdate(setter: (v: string) => void, ref: { current: HTMLInputElement | null }): () => void {
		return () => {
			if (filterTimer.current != null) clearTimeout(filterTimer.current);
			filterTimer.current = setTimeout(() => {
				setter(ref.current?.value || "");
			}, 300);
		};
	}

	return (
		<div className="logs-toolbar">
			<div className="logs-level-filter">
				<ComboSelect
					options={levelFilterOptions()}
					value={levelFilter.value}
					onChange={(value: string) => {
						levelFilter.value = value;
					}}
					placeholder={t("logs:toolbar.allLevels")}
					searchable={false}
				/>
			</div>
			<input
				ref={targetRef}
				type="text"
				placeholder={t("logs:toolbar.filterTarget")}
				className="logs-input"
				style={{ width: "140px" }}
				onInput={debouncedUpdate((v) => {
					targetFilter.value = v;
				}, targetRef)}
			/>
			<input
				ref={searchRef}
				type="text"
				placeholder={t("common:actions.search")}
				className="logs-input"
				style={{ width: "160px" }}
				onInput={debouncedUpdate((v) => {
					searchFilter.value = v;
				}, searchRef)}
			/>
			<button
				className="logs-btn"
				onClick={() => {
					paused.value = !paused.value;
				}}
				style={paused.value ? { borderColor: "var(--warn)" } : undefined}
			>
				{paused.value ? t("logs:toolbar.resume") : t("logs:toolbar.pause")}
			</button>
			<button
				className="logs-btn"
				onClick={() => {
					const area = document.getElementById("logsArea");
					if (area) area.textContent = "";
					entryCount.value = 0;
				}}
			>
				{t("logs:toolbar.clear")}
			</button>
			<a
				href="/api/logs/download"
				className="logs-btn"
				download="moltis-logs.jsonl"
				style={{ textDecoration: "none", textAlign: "center" }}
			>
				{t("logs:toolbar.download")}
			</a>
			<span className="logs-count">
				{entryCount.value} {t("logs:toolbar.entries")}
			</span>
		</div>
	);
}

function LogsPage(): VNode {
	const logAreaRef = useRef<HTMLDivElement>(null);

	function updateEnabledLevels(statusPayload: LogsStatusPayload): void {
		const enabled = statusPayload?.enabled_levels || {};
		traceEnabled.value = !!enabled.trace;
		debugEnabled.value = !!enabled.debug || traceEnabled.value;
		if (levelFilter.value === "trace" && !traceEnabled.value) levelFilter.value = "";
		if (levelFilter.value === "debug" && !debugEnabled.value) levelFilter.value = "";
	}

	function appendEntry(entry: LogEntry): void {
		const area = logAreaRef.current;
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

	function matchesFilter(entry: LogEntry): boolean {
		if (levelFilter.value) {
			const levels = ["trace", "debug", "info", "warn", "error"];
			if (levels.indexOf(entry.level.toLowerCase()) < levels.indexOf(levelFilter.value)) return false;
		}
		const tgtVal = targetFilter.value.trim();
		if (tgtVal && entry.target.indexOf(tgtVal) === -1) return false;
		const searchVal = searchFilter.value.trim().toLowerCase();
		if (
			searchVal &&
			entry.message.toLowerCase().indexOf(searchVal) === -1 &&
			entry.target.toLowerCase().indexOf(searchVal) === -1
		)
			return false;
		return true;
	}

	function refetch(): void {
		const area = logAreaRef.current;
		if (area) area.textContent = "";
		entryCount.value = 0;
		sendRpc("logs.list", {
			level: levelFilter.value || undefined,
			target: targetFilter.value.trim() || undefined,
			search: searchFilter.value.trim() || undefined,
			limit: 500,
		}).then((res) => {
			if (!res?.ok) return;
			const entries: LogEntry[] = (res.payload as { entries?: LogEntry[] })?.entries || [];
			let i = 0;
			const batchSize = 100;
			function renderBatch(): void {
				const end = Math.min(i + batchSize, entries.length);
				while (i < end) appendEntry(entries[i++]);
				if (i < entries.length) requestAnimationFrame(renderBatch);
				else if (logAreaRef.current) logAreaRef.current.scrollTop = logAreaRef.current.scrollHeight;
			}
			renderBatch();
		});
	}

	useEffect(() => {
		sendRpc("logs.status", {})
			.then((res) => {
				if (!res?.ok) return;
				updateEnabledLevels((res.payload as LogsStatusPayload) || {});
			})
			.catch(() => undefined);
		refetch();
		S.setLogsEventHandler((entry: unknown) => {
			if (paused.value) return;
			const e = entry as LogEntry;
			if (!matchesFilter(e)) return;
			appendEntry(e);
		});
		return () => S.setLogsEventHandler(null);
	}, []);

	// Re-fetch when filters change
	useEffect(() => {
		refetch();
	}, [levelFilter.value, targetFilter.value, searchFilter.value]);

	return (
		<>
			<Toolbar />
			<div ref={logAreaRef} id="logsArea" className="logs-area" />
		</>
	);
}

let _logsContainer: HTMLElement | null = null;

export function initLogs(container: HTMLElement): void {
	_logsContainer = container;
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	paused.value = false;
	levelFilter.value = "";
	targetFilter.value = "";
	searchFilter.value = "";
	entryCount.value = 0;
	debugEnabled.value = false;
	traceEnabled.value = false;
	render(<LogsPage />, container);
}

export function teardownLogs(): void {
	S.setLogsEventHandler(null);
	if (_logsContainer) render(null, _logsContainer);
	_logsContainer = null;
}

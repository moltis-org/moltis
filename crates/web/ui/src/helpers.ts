// ── Helpers ──────────────────────────────────────────────────
import { hasTranslation, t } from "./i18n";
import * as S from "./state";
import type { RpcResponse } from "./types";

// Extend Window for webkitAudioContext (Safari)
declare global {
	interface Window {
		webkitAudioContext?: typeof AudioContext;
	}
}

interface CodeBlock {
	lang: string;
	code: string;
}

interface TableParseResult {
	html: string;
	next: number;
}

interface ParsedError {
	icon: string;
	title: string;
	detail: string;
	resetsAt: number | null;
	provider?: string;
}

interface StructuredError {
	icon?: string;
	title?: string;
	detail?: string;
	message?: string;
	provider?: string;
	resetsAt?: number | null;
	title_key?: string;
	detail_key?: string;
	title_params?: Record<string, unknown>;
	detail_params?: Record<string, unknown>;
}

interface RpcErrorObj {
	code?: string;
	message?: string;
	serverMessage?: string;
}

interface LocalizedRpcError extends RpcErrorObj {
	serverMessage?: string;
}

interface ApiPayload {
	error?: string | RpcErrorObj;
	code?: string;
	message?: string;
}

interface ToolCallArgs {
	command?: string;
	node?: string;
	url?: string;
	query?: string;
	action?: string;
	[key: string]: unknown;
}

interface MapLinks {
	url?: string;
	google_maps?: string;
	apple_maps?: string;
	openstreetmap?: string;
	[key: string]: unknown;
}

interface MapPoint {
	label?: string;
	latitude?: number;
	longitude?: number;
	map_links?: MapLinks;
}

interface AgentEntry {
	id?: string;
	name?: string;
	emoji?: string;
	is_default?: boolean;
	[key: string]: unknown;
}

interface AgentsListPayload {
	default_id?: string;
	agents?: AgentEntry[];
}

interface ParsedAgentsList {
	defaultId: string;
	agents: AgentEntry[];
}

interface CreateElAttrs {
	className?: string;
	textContent?: string;
	style?: string;
	[key: string]: string | undefined;
}

/**
 * Extract the highest version number from a model ID for sorting.
 * e.g. "gpt-5.4-mini" -> 5.4, "claude-opus-4-6-20260301" -> 20260301, "o4-mini" -> 4
 * For models with a date suffix the date itself becomes the sort key, which is
 * intentional -- newer dates rank higher.  Returns 0 when no number is found.
 */
export function modelVersionScore(id: string): number {
	const matches = (id || "").match(/\d+(?:\.\d+)?/g);
	if (!matches) return 0;
	let max = 0;
	for (const m of matches) {
		const v = Number.parseFloat(m);
		if (v > max) max = v;
	}
	return max;
}

function translatedOrFallback(
	key: string | undefined,
	opts: Record<string, unknown> | undefined,
	fallback: string,
): string {
	if (!key) return fallback;
	if (!hasTranslation(key, opts)) return fallback;
	const translated = t(key, opts);
	if (translated) return translated;
	return fallback;
}

export function nextId(): string {
	S.setReqId(S.reqId + 1);
	return `ui-${S.reqId}`;
}

export function esc(s: string): string {
	return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function stripAnsi(text: string | null | undefined): string {
	const input = String(text || "");
	let out = "";
	for (let i = 0; i < input.length; i++) {
		if (input.charCodeAt(i) === 27 && input[i + 1] === "[") {
			i += 2;
			while (i < input.length) {
				const ch = input[i];
				if (ch >= "@" && ch <= "~") break;
				i++;
			}
			continue;
		}
		out += input[i];
	}
	return out;
}

function splitPipeCells(line: string): string[] {
	let plain = stripAnsi(line).trim();
	if (plain.startsWith("|")) plain = plain.slice(1);
	if (plain.endsWith("|")) plain = plain.slice(0, -1);
	return plain.split("|").map((cell) => cell.trim());
}

function normalizeTableRow(cells: string[], columnCount: number): string[] {
	const row = cells.slice(0, columnCount);
	while (row.length < columnCount) row.push("");
	return row;
}

function buildTableHtml(headerCells: string[], bodyRows: string[][]): string {
	const columnCount = headerCells.length;
	const headerRow = normalizeTableRow(headerCells, columnCount);
	const bodyHtml = bodyRows
		.map((row) => normalizeTableRow(row, columnCount))
		.map((row) => `<tr>${row.map((cell) => `<td>${cell}</td>`).join("")}</tr>`)
		.join("");
	const thead = `<thead><tr>${headerRow.map((cell) => `<th>${cell}</th>`).join("")}</tr></thead>`;
	const tbody = bodyRows.length > 0 ? `<tbody>${bodyHtml}</tbody>` : "";
	return `<div class="msg-table-wrap"><table class="msg-table">${thead}${tbody}</table></div>`;
}

function isMarkdownPipeRow(line: string): boolean {
	if (!stripAnsi(line).includes("|")) return false;
	return splitPipeCells(line).length >= 2;
}

function isMarkdownSeparatorRow(line: string, expectedCols: number): boolean {
	const cells = splitPipeCells(line);
	if (cells.length !== expectedCols) return false;
	return cells.every((cell) => /^:?-{3,}:?$/.test(cell));
}

function parseMarkdownTable(lines: string[], start: number): TableParseResult | null {
	if (start + 1 >= lines.length) return null;
	if (!isMarkdownPipeRow(lines[start])) return null;
	const headerCells = splitPipeCells(lines[start]);
	if (headerCells.length < 2) return null;
	if (!isMarkdownSeparatorRow(lines[start + 1], headerCells.length)) return null;

	const bodyRows: string[][] = [];
	let next = start + 2;
	while (next < lines.length) {
		const candidate = lines[next];
		if (!candidate.trim()) break;
		if (!isMarkdownPipeRow(candidate)) break;
		bodyRows.push(splitPipeCells(candidate));
		next++;
	}
	return {
		html: buildTableHtml(headerCells, bodyRows),
		next: next,
	};
}

function isAsciiBorderRow(line: string): boolean {
	return /^\+(?:[-=]+\+)+$/.test(stripAnsi(line).trim());
}

function isAsciiPipeRow(line: string): boolean {
	return /^\|.*\|$/.test(stripAnsi(line).trim());
}

function parseAsciiTable(lines: string[], start: number): TableParseResult | null {
	if (!isAsciiBorderRow(lines[start])) return null;
	let next = start + 1;
	const rows: string[][] = [];

	while (next < lines.length) {
		const line = lines[next];
		if (isAsciiBorderRow(line)) {
			next++;
			continue;
		}
		if (!isAsciiPipeRow(line)) break;
		rows.push(splitPipeCells(line));
		next++;
	}
	if (rows.length === 0) return null;

	return {
		html: buildTableHtml(rows[0], rows.slice(1)),
		next: next,
	};
}

function renderTables(s: string): string {
	const lines = s.split("\n");
	const out: string[] = [];
	for (let i = 0; i < lines.length; ) {
		const markdownTable = parseMarkdownTable(lines, i);
		if (markdownTable) {
			out.push(markdownTable.html);
			i = markdownTable.next;
			continue;
		}

		const asciiTable = parseAsciiTable(lines, i);
		if (asciiTable) {
			out.push(asciiTable.html);
			i = asciiTable.next;
			continue;
		}

		out.push(lines[i]);
		i++;
	}
	return out.join("\n");
}

export function renderMarkdown(raw: string): string {
	let s = esc(raw);
	const codeBlocks: CodeBlock[] = [];
	s = s.replace(/```(\w*)\n([\s\S]*?)```/g, (_: string, lang: string, code: string) => {
		codeBlocks.push({ lang: lang, code: code });
		return `@@MOLTIS_CODE_BLOCK_${codeBlocks.length - 1}@@`;
	});
	s = renderTables(s);
	s = s.replace(/`([^`]+)`/g, "<code>$1</code>");
	s = s.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
	s = s.replace(/@@MOLTIS_CODE_BLOCK_(\d+)@@/g, (_: string, idx: string) => {
		const block = codeBlocks[Number(idx)];
		if (!block) return "";
		const langAttr = block.lang ? ` data-lang="${block.lang}"` : "";
		const badge = block.lang ? `<div class="code-lang-badge">${block.lang}</div>` : "";
		return `<pre class="code-block">${badge}<code${langAttr}>${block.code}</code></pre>`;
	});
	return s;
}

export function sendRpc<T = unknown>(method: string, params: unknown): Promise<RpcResponse<T>> {
	return new Promise((resolve) => {
		if (!S.ws || S.ws.readyState !== WebSocket.OPEN) {
			resolve({
				ok: false,
				error: {
					code: "UNAVAILABLE",
					message: localizedRpcErrorMessage({
						code: "UNAVAILABLE",
						message: "WebSocket not connected",
					}),
				},
			});
			return;
		}
		const id = nextId();
		S.pending[id] = resolve as (value: RpcResponse) => void;
		S.ws.send(JSON.stringify({ type: "req", id: id, method: method, params: params }));
	});
}

export function localizedRpcErrorMessage(error: RpcErrorObj | null | undefined): string {
	if (!error) return t("errors:generic.title");
	if (error.code) {
		const key = `errors:codes.${error.code}`;
		const translated = t(key);
		if (translated && translated !== key) {
			return translated;
		}
	}
	return error.message || t("errors:generic.title");
}

export function localizedApiErrorMessage(payload: ApiPayload | null | undefined, fallbackMessage?: string): string {
	if (payload && typeof payload.error === "object") {
		return localizedRpcErrorMessage(payload.error);
	}
	if (payload?.code) {
		const key = `errors:codes.${payload.code}`;
		const translated = t(key);
		if (translated && translated !== key) {
			return translated;
		}
	}
	if (payload && typeof payload.error === "string" && payload.error.trim()) {
		return payload.error;
	}
	if (payload && typeof payload.message === "string" && payload.message.trim()) {
		return payload.message;
	}
	return fallbackMessage || t("errors:generic.title");
}

export function localizeRpcError(error: RpcErrorObj | null | undefined): LocalizedRpcError | null | undefined {
	if (!error) return error;
	// When the server provides a specific message (not just an error code),
	// preserve it as `serverMessage` so callers like model probes can show
	// the precise backend reason instead of a generic locale string.
	const message = localizedRpcErrorMessage(error);
	if (error.message === message) return error;
	return Object.assign({}, error, { message: message, serverMessage: error.message });
}

export function localizeStructuredError(error: StructuredError | null | undefined): StructuredError | null | undefined {
	if (!error) return error;
	const title = translatedOrFallback(error.title_key, error.title_params, error.title || t("errors:generic.title"));
	const detail = translatedOrFallback(error.detail_key, error.detail_params, error.detail || "");
	if (title === error.title && detail === (error.detail || "")) return error;
	return Object.assign({}, error, { title: title, detail: detail });
}

export function formatTokens(n: number): string {
	if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
	if (n >= 1000) return `${(n / 1000).toFixed(1)}K`;
	return String(n);
}

export function formatAssistantTokenUsage(
	inputTokens: number | undefined,
	outputTokens: number | undefined,
	cacheReadTokens?: number | undefined,
): string {
	const input = Number(inputTokens || 0);
	const output = Number(outputTokens || 0);
	const cached = Number(cacheReadTokens || 0);
	let inputText = `${formatTokens(input)} in`;
	if (cached > 0) {
		inputText += ` (${formatTokens(cached)} cached)`;
	}
	return `${inputText} / ${formatTokens(output)} out`;
}

const TOKEN_SPEED_SLOW_TPS = 10;
const TOKEN_SPEED_FAST_TPS = 25;

export function tokenSpeedPerSecond(outputTokens: number | undefined, durationMs: number | undefined): number | null {
	const out = Number(outputTokens || 0);
	const ms = Number(durationMs || 0);
	if (!(out > 0 && ms > 0)) return null;
	const speed = (out * 1000) / ms;
	return Number.isFinite(speed) && speed > 0 ? speed : null;
}

export function formatTokenSpeed(outputTokens: number | undefined, durationMs: number | undefined): string | null {
	const speed = tokenSpeedPerSecond(outputTokens, durationMs);
	if (speed == null) return null;
	if (speed >= 100) return `${speed.toFixed(0)} tok/s`;
	if (speed >= 10) return `${speed.toFixed(1)} tok/s`;
	return `${speed.toFixed(2)} tok/s`;
}

export function tokenSpeedTone(outputTokens: number | undefined, durationMs: number | undefined): string | null {
	const speed = tokenSpeedPerSecond(outputTokens, durationMs);
	if (speed == null) return null;
	if (speed < TOKEN_SPEED_SLOW_TPS) return "slow";
	if (speed >= TOKEN_SPEED_FAST_TPS) return "fast";
	return "normal";
}

export function formatBytes(b: number): string {
	if (b >= 1024) return `${(b / 1024).toFixed(1)} KB`;
	return `${b} B`;
}

function getResetsAtMs(errObj: Record<string, unknown>): number | null {
	return (errObj.resetsAt as number | null) || (errObj.resets_at ? (errObj.resets_at as number) * 1000 : null);
}

function classifyStructuredError(errObj: Record<string, unknown>, resetsAt: number | null): ParsedError | null {
	if (!(errObj.title_key || errObj.detail_key)) return null;
	const result = localizeStructuredError({
		icon: (errObj.icon as string) || "\u26A0\uFE0F",
		title: (errObj.title as string) || t("errors:generic.title"),
		detail: (errObj.detail as string) || (errObj.message as string) || "",
		provider: errObj.provider as string | undefined,
		resetsAt: resetsAt,
		title_key: errObj.title_key as string | undefined,
		detail_key: errObj.detail_key as string | undefined,
		title_params: errObj.title_params as Record<string, unknown> | undefined,
		detail_params: errObj.detail_params as Record<string, unknown> | undefined,
	});
	if (!result) return null;
	return {
		icon: result.icon || "\u26A0\uFE0F",
		title: result.title || t("errors:generic.title"),
		detail: result.detail || "",
		provider: result.provider,
		resetsAt: result.resetsAt ?? null,
	};
}

function classifyUsageLimitError(errObj: Record<string, unknown>, resetsAt: number | null): ParsedError | null {
	if (
		!(errObj.type === "usage_limit_reached" || (errObj.message && String(errObj.message).indexOf("usage limit") !== -1))
	) {
		return null;
	}
	return {
		icon: "",
		title: t("errors:usageLimitReached.title"),
		detail: t("errors:usageLimitReached.detail", { planType: (errObj.plan_type as string) || "current" }),
		resetsAt: resetsAt,
	};
}

function classifyRateLimitError(errObj: Record<string, unknown>, resetsAt: number | null): ParsedError | null {
	if (
		!(errObj.type === "rate_limit_exceeded" || (errObj.message && String(errObj.message).indexOf("rate limit") !== -1))
	) {
		return null;
	}
	return {
		icon: "\u26A0\uFE0F",
		title: t("errors:rateLimited.title"),
		detail: (errObj.message as string) || t("errors:rateLimited.detail"),
		resetsAt: resetsAt,
	};
}

function classifyJsonErrorObj(errObj: Record<string, unknown>): ParsedError | null {
	const resetsAt = getResetsAtMs(errObj);
	return (
		classifyStructuredError(errObj, resetsAt) ||
		classifyUsageLimitError(errObj, resetsAt) ||
		classifyRateLimitError(errObj, resetsAt) ||
		(errObj.message
			? { icon: "\u26A0\uFE0F", title: t("errors:generic.title"), detail: errObj.message as string, resetsAt: null }
			: null)
	);
}

function parseJsonError(message: string): ParsedError | null {
	const jsonMatch = message.match(/\{[\s\S]*\}$/);
	if (!jsonMatch) return null;
	try {
		const err = JSON.parse(jsonMatch[0]) as Record<string, unknown>;
		return classifyJsonErrorObj((err.error as Record<string, unknown>) || err);
	} catch (_e) {
		/* fall through */
	}
	return null;
}

function parseHttpStatusError(message: string): ParsedError | null {
	const statusMatch = message.match(/HTTP (\d{3})/);
	const code = statusMatch ? parseInt(statusMatch[1], 10) : 0;
	if (code === 401 || code === 403)
		return {
			icon: "\uD83D\uDD12",
			title: t("errors:authError.title"),
			detail: t("errors:authError.detail"),
			resetsAt: null,
		};
	if (code === 429)
		return {
			icon: "",
			title: t("errors:rateLimited.title"),
			detail: t("errors:rateLimited.detailShort"),
			resetsAt: null,
		};
	if (code >= 500)
		return {
			icon: "\uD83D\uDEA8",
			title: t("errors:serverError.title"),
			detail: t("errors:serverError.detail"),
			resetsAt: null,
		};
	return null;
}

export function parseErrorMessage(message: string): ParsedError {
	return (
		parseJsonError(message) ||
		parseHttpStatusError(message) || {
			icon: "\u26A0\uFE0F",
			title: t("errors:generic.title"),
			detail: message,
			resetsAt: null,
		}
	);
}

export function updateCountdown(el: HTMLElement, resetsAtMs: number): boolean {
	const now = Date.now();
	const diff = resetsAtMs - now;
	if (diff <= 0) {
		el.textContent = t("errors:countdown.resetReady");
		el.className = "error-countdown reset-ready";
		return true;
	}
	const hours = Math.floor(diff / 3600000);
	const mins = Math.floor((diff % 3600000) / 60000);
	const parts: string[] = [];
	if (hours > 0) parts.push(`${hours}h`);
	parts.push(`${mins}m`);
	el.textContent = t("errors:countdown.resetsIn", { time: parts.join(" ") });
	return false;
}

/** Build a short summary string for a tool call card. */
export function toolCallSummary(
	name: string | undefined,
	args: ToolCallArgs | undefined,
	executionMode?: string,
): string {
	if (!args) return name || "tool";
	switch (name) {
		case "exec": {
			const command = args.command || "exec";
			const nodeRef = typeof args.node === "string" ? args.node.trim() : "";
			if (!nodeRef) return command;
			if (nodeRef.startsWith("ssh:target:")) {
				return `${command} [SSH target]`;
			}
			if (nodeRef.startsWith("ssh:")) {
				return `${command} [SSH: ${nodeRef.slice(4)}]`;
			}
			if (nodeRef.includes("@")) {
				return `${command} [SSH: ${nodeRef}]`;
			}
			return `${command} [node: ${nodeRef}]`;
		}
		case "web_fetch":
			return `web_fetch ${args.url || ""}`.trim();
		case "web_search":
			return `web_search "${args.query || ""}"`;
		case "browser": {
			const action = args.action || "browser";
			const mode = executionMode ? ` (${executionMode})` : "";
			const url = args.url ? ` ${args.url}` : "";
			return `browser ${action}${mode}${url}`.trim();
		}
		default:
			return name || "tool";
	}
}

/**
 * Render a screenshot thumbnail with lightbox and download into `container`.
 * @param container - parent element to append into
 * @param imgSrc - image URL (data URI or HTTP URL)
 * @param scale - HiDPI scale factor
 */
export function renderScreenshot(container: HTMLElement, imgSrc: string, scale?: number): void {
	if (!scale) scale = 1;
	const imgContainer = document.createElement("div");
	imgContainer.className = "screenshot-container";
	const img = document.createElement("img");
	img.src = imgSrc;
	img.className = "screenshot-thumbnail";
	img.alt = "Browser screenshot";
	img.title = "Click to view full size";

	const effectiveScale = scale;
	img.onload = (): void => {
		if (effectiveScale > 1) {
			const logicalWidth = img.naturalWidth / effectiveScale;
			const logicalHeight = img.naturalHeight / effectiveScale;
			img.style.aspectRatio = `${logicalWidth} / ${logicalHeight}`;
		}
	};

	const downloadScreenshot = (e: Event): void => {
		e.stopPropagation();
		const link = document.createElement("a");
		link.href = imgSrc;
		link.download = `screenshot-${Date.now()}.png`;
		link.click();
	};

	img.onclick = (): void => {
		const overlay = document.createElement("div");
		overlay.className = "screenshot-lightbox";

		const lightboxContent = document.createElement("div");
		lightboxContent.className = "screenshot-lightbox-content";

		const header = document.createElement("div");
		header.className = "screenshot-lightbox-header";
		header.onclick = (e: Event): void => e.stopPropagation();

		const closeBtn = document.createElement("button");
		closeBtn.className = "screenshot-lightbox-close";
		closeBtn.textContent = "\u2715";
		closeBtn.title = "Close (Esc)";
		closeBtn.onclick = (): void => overlay.remove();

		const downloadBtn = document.createElement("button");
		downloadBtn.className = "screenshot-download-btn";
		downloadBtn.textContent = "\u2B07 Download";
		downloadBtn.onclick = downloadScreenshot;

		header.appendChild(closeBtn);
		header.appendChild(downloadBtn);

		const scrollContainer = document.createElement("div");
		scrollContainer.className = "screenshot-lightbox-scroll";
		scrollContainer.onclick = (e: Event): void => e.stopPropagation();

		const fullImg = document.createElement("img");
		fullImg.src = img.src;
		fullImg.className = "screenshot-lightbox-img";

		fullImg.onload = (): void => {
			const logicalWidth = fullImg.naturalWidth / effectiveScale;
			const logicalHeight = fullImg.naturalHeight / effectiveScale;
			const viewportWidth = window.innerWidth - 80;
			const displayWidth = Math.min(logicalWidth, viewportWidth);
			fullImg.style.width = `${displayWidth}px`;
			const displayHeight = (displayWidth / logicalWidth) * logicalHeight;
			fullImg.style.height = `${displayHeight}px`;
		};

		scrollContainer.appendChild(fullImg);
		lightboxContent.appendChild(header);
		lightboxContent.appendChild(scrollContainer);
		overlay.appendChild(lightboxContent);

		overlay.onclick = (): void => overlay.remove();
		const closeOnEscape = (e: KeyboardEvent): void => {
			if (e.key === "Escape") {
				overlay.remove();
				document.removeEventListener("keydown", closeOnEscape);
			}
		};
		document.addEventListener("keydown", closeOnEscape);
		document.body.appendChild(overlay);
	};

	const thumbDownloadBtn = document.createElement("button");
	thumbDownloadBtn.className = "screenshot-download-btn-small";
	thumbDownloadBtn.textContent = "\u2B07";
	thumbDownloadBtn.title = "Download screenshot";
	thumbDownloadBtn.onclick = downloadScreenshot;

	imgContainer.appendChild(img);
	imgContainer.appendChild(thumbDownloadBtn);
	container.appendChild(imgContainer);
}

// ── Document card ───────────────────────────────────────────

/**
 * Return an icon string for a given MIME type / filename extension.
 */
function documentIcon(mimeType?: string, filename?: string): string {
	const ext = (filename || "").split(".").pop()?.toLowerCase() || "";
	if (mimeType === "application/pdf" || ext === "pdf") return "\uD83D\uDCC4"; // 📄
	if (mimeType === "application/zip" || mimeType === "application/gzip" || ext === "zip" || ext === "gz")
		return "\uD83D\uDCE6"; // 📦
	if (/spreadsheet|csv|xls/.test(mimeType || "") || /^(csv|xls|xlsx)$/.test(ext)) return "\uD83D\uDCCA"; // 📊
	if (/wordprocessing|msword|rtf/.test(mimeType || "") || /^(doc|docx|rtf)$/.test(ext)) return "\uD83D\uDCC3"; // 📃
	if (/presentation|ppt/.test(mimeType || "") || /^(ppt|pptx)$/.test(ext)) return "\uD83D\uDCCA"; // 📊
	return "\uD83D\uDCC1"; // 📁
}

/**
 * Format a byte count for display (e.g. "1.2 MB", "345 KB").
 */
function formatDocSize(bytes: number): string {
	if (typeof bytes !== "number" || bytes < 0) return "";
	if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)} MB`;
	if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	return `${bytes} B`;
}

/**
 * Render a document card with filename, icon, and download link.
 * @param container - parent element to append into
 * @param mediaSrc - URL to the document (media API endpoint)
 * @param filename - display filename
 * @param mimeType - MIME type for icon selection
 * @param sizeBytes - file size for display
 */
export function renderDocument(
	container: HTMLElement,
	mediaSrc: string,
	filename: string,
	mimeType?: string,
	sizeBytes?: number,
): void {
	const wrap = document.createElement("div");
	wrap.className = "document-container";

	const icon = document.createElement("span");
	icon.className = "document-icon";
	icon.textContent = documentIcon(mimeType, filename);

	const info = document.createElement("div");
	info.className = "document-info";

	const nameEl = document.createElement("span");
	nameEl.className = "document-filename";
	nameEl.textContent = filename || "document";

	info.appendChild(nameEl);

	if (sizeBytes != null && sizeBytes > 0) {
		const sizeEl = document.createElement("span");
		sizeEl.className = "document-size";
		sizeEl.textContent = formatDocSize(sizeBytes);
		info.appendChild(sizeEl);
	}

	const dlBtn = document.createElement("a");
	dlBtn.className = "document-download-btn";
	dlBtn.href = mediaSrc;
	dlBtn.download = filename || "document";
	// PDFs and text open in new tab; others download
	const isPdf = (mimeType || "").includes("pdf") || (filename || "").endsWith(".pdf");
	const isText = (mimeType || "").startsWith("text/");
	if (isPdf || isText) {
		dlBtn.target = "_blank";
		dlBtn.rel = "noopener noreferrer";
		dlBtn.textContent = "\u2197 Open";
		dlBtn.removeAttribute("download");
	} else {
		dlBtn.textContent = "\u2B07 Download";
	}

	wrap.appendChild(icon);
	wrap.appendChild(info);
	wrap.appendChild(dlBtn);
	container.appendChild(wrap);
}

// ── Waveform audio player ───────────────────────────────────

const WAVEFORM_BAR_COUNT = 48;
const WAVEFORM_MIN_HEIGHT = 0.08;

async function extractWaveform(audioSrc: string, barCount: number): Promise<number[]> {
	const ctx = new (window.AudioContext || window.webkitAudioContext!)();
	try {
		const response = await fetch(audioSrc);
		const buf = await response.arrayBuffer();
		const audioBuffer = await ctx.decodeAudioData(buf);
		const data = audioBuffer.getChannelData(0);
		if (data.length < barCount) {
			return new Array(barCount).fill(WAVEFORM_MIN_HEIGHT) as number[];
		}
		const step = Math.floor(data.length / barCount);
		const peaks: number[] = [];
		for (let i = 0; i < barCount; i++) {
			const start = i * step;
			const end = Math.min(start + step, data.length);
			let max = 0;
			for (let j = start; j < end; j++) {
				const abs = Math.abs(data[j]);
				if (abs > max) max = abs;
			}
			peaks.push(max);
		}
		let maxPeak = 0;
		for (const pk of peaks) {
			if (pk > maxPeak) maxPeak = pk;
		}
		maxPeak = maxPeak || 1;
		return peaks.map((v) => Math.max(WAVEFORM_MIN_HEIGHT, v / maxPeak));
	} finally {
		ctx.close();
	}
}

export function formatAudioDuration(seconds: number): string {
	if (!Number.isFinite(seconds) || seconds < 0) return "00:00";
	const totalSeconds = Math.floor(seconds);
	const m = Math.floor(totalSeconds / 60);
	const s = totalSeconds % 60;
	return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

function createPlaySvg(): SVGSVGElement {
	const NS = "http://www.w3.org/2000/svg";
	const el = document.createElementNS(NS, "svg");
	el.setAttribute("viewBox", "0 0 24 24");
	el.setAttribute("aria-hidden", "true");
	el.setAttribute("focusable", "false");
	el.setAttribute("fill", "currentColor");
	el.setAttribute("preserveAspectRatio", "xMidYMid meet");

	const path = document.createElementNS(NS, "path");
	path.setAttribute("d", "M8 5v14l11-7z");
	el.appendChild(path);
	return el;
}

function createPauseSvg(): SVGSVGElement {
	const NS = "http://www.w3.org/2000/svg";
	const el = document.createElementNS(NS, "svg");
	el.setAttribute("viewBox", "0 0 24 24");
	el.setAttribute("aria-hidden", "true");
	el.setAttribute("focusable", "false");
	el.setAttribute("fill", "currentColor");
	el.setAttribute("preserveAspectRatio", "xMidYMid meet");

	const left = document.createElementNS(NS, "rect");
	left.setAttribute("x", "6");
	left.setAttribute("y", "4");
	left.setAttribute("width", "4");
	left.setAttribute("height", "16");
	left.setAttribute("rx", "1");
	el.appendChild(left);

	const right = document.createElementNS(NS, "rect");
	right.setAttribute("x", "14");
	right.setAttribute("y", "4");
	right.setAttribute("width", "4");
	right.setAttribute("height", "16");
	right.setAttribute("rx", "1");
	el.appendChild(right);
	return el;
}

// ── Audio autoplay unlock ────────────────────────────────────
// Browsers block audio.play() without a recent user gesture. We "unlock"
// playback by creating a shared AudioContext on the first user action
// (sending a message / clicking record). Once resumed, all subsequent
// audio.play() calls on the page are allowed.
let _audioCtx: AudioContext | null = null;

/**
 * Call from a user-gesture handler (click / keydown) to unlock audio
 * playback for the current page session. Idempotent -- safe to call
 * multiple times.
 */
export function warmAudioPlayback(): void {
	if (!_audioCtx) {
		_audioCtx = new (window.AudioContext || window.webkitAudioContext!)();
		console.debug("[audio] created AudioContext, state:", _audioCtx.state);
	}
	if (_audioCtx.state === "suspended") {
		console.debug("[audio] resuming suspended AudioContext");
		_audioCtx.resume().catch((e: unknown) => console.warn("[audio] resume failed:", e));
	}
}

/**
 * Render a waveform audio player (Telegram-style bars) into `container`.
 * @param container - parent element to append into
 * @param audioSrc - audio URL (HTTP or data URI)
 * @param autoplay - start playback immediately
 */
// Track the most recently played audio element so spacebar can toggle it.
let _activeAudio: HTMLAudioElement | null = null;

function isEditableTarget(el: EventTarget | null): boolean {
	if (!(el && el instanceof HTMLElement)) return false;
	const tag = el.tagName;
	if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
	return el.isContentEditable;
}

document.addEventListener("keydown", (e: KeyboardEvent) => {
	if (e.key !== " " || e.repeat) return;
	if (isEditableTarget(e.target)) return;
	if (!_activeAudio) return;
	e.preventDefault();
	if (_activeAudio.paused) {
		_activeAudio.play().catch(() => undefined);
	} else {
		_activeAudio.pause();
	}
});

export function renderAudioPlayer(container: HTMLElement, audioSrc: string, autoplay?: boolean): void {
	const wrap = document.createElement("div");
	wrap.className = "waveform-player mt-2";

	const audio = document.createElement("audio");
	audio.preload = "auto";
	audio.src = audioSrc;

	const playBtn = document.createElement("button");
	playBtn.className = "waveform-play-btn";
	playBtn.type = "button";
	playBtn.appendChild(createPlaySvg());

	const barsWrap = document.createElement("div");
	barsWrap.className = "waveform-bars";

	const durEl = document.createElement("span");
	durEl.className = "waveform-duration";
	durEl.textContent = "00:00";

	wrap.appendChild(playBtn);
	wrap.appendChild(barsWrap);
	wrap.appendChild(durEl);
	container.appendChild(wrap);

	const bars: HTMLElement[] = [];
	for (let i = 0; i < WAVEFORM_BAR_COUNT; i++) {
		const bar = document.createElement("div");
		bar.className = "waveform-bar";
		bar.style.height = "20%";
		barsWrap.appendChild(bar);
		bars.push(bar);
	}

	extractWaveform(audioSrc, WAVEFORM_BAR_COUNT)
		.then((peaks) => {
			peaks.forEach((p, idx) => {
				bars[idx].style.height = `${p * 100}%`;
			});
		})
		.catch(() => {
			for (const b of bars) {
				b.style.height = `${20 + Math.random() * 60}%`;
			}
		});

	function syncDurationLabel(): void {
		if (!Number.isFinite(audio.duration) || audio.duration < 0) return;
		durEl.textContent = formatAudioDuration(audio.duration);
	}

	audio.addEventListener("loadedmetadata", syncDurationLabel);
	audio.addEventListener("durationchange", syncDurationLabel);
	audio.addEventListener("canplay", syncDurationLabel);

	playBtn.onclick = (): void => {
		if (audio.paused) {
			audio.play().catch(() => undefined);
		} else {
			audio.pause();
		}
	};

	let rafId = 0;
	let prevPlayed = -1;

	function tick(): void {
		if (!Number.isFinite(audio.duration) || audio.duration <= 0) {
			rafId = requestAnimationFrame(tick);
			return;
		}
		const progress = audio.currentTime / audio.duration;
		const playedCount = Math.floor(progress * WAVEFORM_BAR_COUNT);
		if (playedCount !== prevPlayed) {
			const lo = Math.min(playedCount, prevPlayed < 0 ? 0 : prevPlayed);
			const hi = Math.max(playedCount, prevPlayed < 0 ? WAVEFORM_BAR_COUNT : prevPlayed);
			for (let idx = lo; idx < hi; idx++) {
				bars[idx].classList.toggle("played", idx < playedCount);
			}
			prevPlayed = playedCount;
		}
		durEl.textContent = formatAudioDuration(audio.currentTime);
		rafId = requestAnimationFrame(tick);
	}

	audio.addEventListener("play", () => {
		_activeAudio = audio;
		playBtn.replaceChildren(createPauseSvg());
		prevPlayed = -1;
		rafId = requestAnimationFrame(tick);
	});

	audio.addEventListener("pause", () => {
		playBtn.replaceChildren(createPlaySvg());
		cancelAnimationFrame(rafId);
	});

	audio.addEventListener("ended", () => {
		if (_activeAudio === audio) _activeAudio = null;
		playBtn.replaceChildren(createPlaySvg());
		cancelAnimationFrame(rafId);
		for (const b of bars) b.classList.remove("played");
		prevPlayed = -1;
		if (Number.isFinite(audio.duration) && audio.duration >= 0) {
			durEl.textContent = formatAudioDuration(audio.duration);
		}
	});

	barsWrap.onclick = (e: MouseEvent): void => {
		if (!Number.isFinite(audio.duration) || audio.duration <= 0) return;
		const rect = barsWrap.getBoundingClientRect();
		const fraction = (e.clientX - rect.left) / rect.width;
		audio.currentTime = Math.max(0, Math.min(1, fraction)) * audio.duration;
		if (audio.paused) audio.play().catch(() => undefined);
	};

	if (autoplay) {
		// Ensure AudioContext is resumed (may have been unlocked by warmAudioPlayback).
		warmAudioPlayback();
		console.debug(
			"[audio] autoplay requested, readyState:",
			audio.readyState,
			"audioCtx:",
			_audioCtx?.state,
			"src:",
			audioSrc.substring(0, 60),
		);
		const doPlay = (): void => {
			console.debug("[audio] attempting play(), readyState:", audio.readyState, "paused:", audio.paused);
			audio
				.play()
				.then(() => console.debug("[audio] play() succeeded"))
				.catch((e: DOMException) => console.warn("[audio] play() rejected:", e.name, e.message));
		};
		// Wait for enough data to be buffered before starting playback.
		if (audio.readyState >= 3) {
			doPlay();
		} else {
			console.debug("[audio] waiting for canplay event");
			audio.addEventListener("canplay", doPlay, { once: true });
		}
	}
}

/**
 * Render a single clickable map link into `container`.
 * @param container - parent element to append into
 * @param links - map link payload
 * @param label - optional location label
 * @param heading - optional heading text
 */
function resolveMapUrl(links: MapLinks | null | undefined): string {
	if (!(links && typeof links === "object")) return "";
	if (typeof links.url === "string" && links.url.trim()) return links.url.trim();

	const providers: (keyof MapLinks)[] = ["google_maps", "apple_maps", "openstreetmap"];
	for (const provider of providers) {
		const providerUrl = links[provider];
		if (typeof providerUrl === "string" && providerUrl.trim()) return providerUrl.trim();
	}
	return "";
}

function mapPointHeading(point: MapPoint, index: number): string {
	const label = typeof point?.label === "string" ? point.label.trim() : "";
	if (label) return label;
	const latOk = typeof point?.latitude === "number" && Number.isFinite(point.latitude);
	const lonOk = typeof point?.longitude === "number" && Number.isFinite(point.longitude);
	if (latOk && lonOk) return `${point.latitude?.toFixed(5)}, ${point.longitude?.toFixed(5)}`;
	return `Location ${index + 1}`;
}

function splitMapLinkText(text: string | undefined): { primary: string; secondary: string } {
	const normalized = typeof text === "string" ? text.trim() : "";
	if (!normalized) return { primary: "", secondary: "" };
	const starIndex = normalized.indexOf("\u2B50");
	if (starIndex <= 0) return { primary: normalized, secondary: "" };
	const primary = normalized.slice(0, starIndex).trim();
	const secondary = normalized.slice(starIndex).trim();
	if (!(primary && secondary)) return { primary: normalized, secondary: "" };
	return { primary, secondary };
}

export function renderMapLinks(
	container: HTMLElement,
	links: MapLinks | null | undefined,
	label?: string,
	heading?: string,
): boolean {
	const mapUrl = resolveMapUrl(links);
	if (!mapUrl) return false;

	const block = document.createElement("div");
	block.className = "mt-2";
	const text = heading || (typeof label === "string" && label.trim() ? label.trim() : "Open map");
	const textParts = splitMapLinkText(text);
	const link = document.createElement("a");
	link.href = mapUrl;
	link.target = "_blank";
	link.rel = "noopener noreferrer";
	link.className = "text-xs map-link-row";
	const primary = document.createElement("span");
	primary.className = "map-link-name";
	primary.textContent = textParts.primary || text;
	link.appendChild(primary);
	if (textParts.secondary) {
		const secondary = document.createElement("span");
		secondary.className = "map-link-meta";
		secondary.textContent = textParts.secondary;
		link.appendChild(secondary);
	}
	link.title = `Open "${text}" in maps`;
	block.appendChild(link);
	container.appendChild(block);
	return true;
}

export function renderMapPointGroups(
	container: HTMLElement,
	points: MapPoint[] | null | undefined,
	fallbackLabel?: string,
): boolean {
	if (!Array.isArray(points) || points.length === 0) return false;

	let rendered = false;
	const showHeadings = points.length > 1;
	for (let i = 0; i < points.length; i++) {
		const point = points[i];
		if (!(point && typeof point === "object")) continue;
		const label = typeof point.label === "string" && point.label.trim() ? point.label.trim() : fallbackLabel;
		const heading = showHeadings ? mapPointHeading(point, i) : "";
		if (renderMapLinks(container, point.map_links, label, heading)) rendered = true;
	}
	return rendered;
}

/**
 * Parse the payload from `agents.list` into `{ defaultId, agents }`.
 * Handles both array (legacy) and object shapes.
 */
export function parseAgentsListPayload(payload: AgentEntry[] | AgentsListPayload): ParsedAgentsList {
	if (Array.isArray(payload)) {
		const legacyDefault = payload.find((agent) => agent?.is_default === true && typeof agent?.id === "string")?.id;
		return { defaultId: legacyDefault || "main", agents: payload };
	}
	const agents = Array.isArray(payload?.agents) ? payload.agents : [];
	const inferredDefault = agents.find((agent) => agent?.is_default === true && typeof agent?.id === "string")?.id;
	return {
		defaultId: typeof payload?.default_id === "string" ? payload.default_id : inferredDefault || "main",
		agents: agents,
	};
}

export function createEl(tag: string, attrs?: CreateElAttrs | null, children?: (HTMLElement | null)[]): HTMLElement {
	const el = document.createElement(tag);
	if (attrs) {
		Object.keys(attrs).forEach((k) => {
			const value = attrs[k];
			if (value === undefined) return;
			if (k === "className") el.className = value;
			else if (k === "textContent") el.textContent = value;
			else if (k === "style") el.style.cssText = value;
			else el.setAttribute(k, value);
		});
	}
	if (children) {
		children.forEach((c) => {
			if (c) el.appendChild(c);
		});
	}
	return el;
}

// Settings > Terminal (host shell via PTY + xterm.js over WebSocket)
//
// This page is entirely imperative DOM -- no Preact rendering.
// It manages an xterm.js terminal instance and a WebSocket to the server.
//
// Note: container.innerHTML is set with a static template string containing
// only hard-coded markup -- no user input is interpolated.

import { localizedApiErrorMessage } from "../helpers";

// ── Types ────────────────────────────────────────────────────

interface WindowInfo {
	id: string;
	index: number;
	name: string;
	active: boolean;
}

interface ReadyPayload {
	available: boolean;
	persistenceEnabled: boolean;
	persistenceAvailable: boolean;
	activeWindowId?: string;
	tmuxInstallCommand?: string;
	user?: string;
}

interface TerminalMessage {
	type: string;
	data?: string;
	encoding?: string;
	text?: string;
	level?: string;
	error?: string;
	windowId?: string;
	windows?: unknown[];
	activeWindowId?: string;
	available?: boolean;
	persistenceEnabled?: boolean;
	persistenceAvailable?: boolean;
	tmuxInstallCommand?: string;
	user?: string;
}

interface SocketMessage {
	type: string;
	[key: string]: unknown;
}

interface WindowsPayload {
	windows?: unknown[];
	activeWindowId?: string;
	available?: boolean;
	window?: { id?: string };
	windowId?: string;
}

interface XtermOptions {
	convertEol?: boolean;
	disableStdin?: boolean;
	cursorBlink?: boolean;
	scrollback?: number;
	fontFamily?: string;
	fontSize?: number;
	lineHeight?: number;
	theme?: Record<string, string>;
}

type TerminalCtorType = new (opts: XtermOptions) => XtermInstance;
type FitAddonCtorType = new () => FitAddonInstance;

interface XtermInstance {
	cols: number;
	rows: number;
	options: { theme?: Record<string, string>; [key: string]: unknown };
	buffer: { active: { baseY: number; viewportY: number } };
	parser: { registerOscHandler: (code: number, handler: () => boolean) => { dispose: () => void } };
	loadAddon: (addon: FitAddonInstance) => void;
	open: (el: HTMLElement) => void;
	onData: (handler: (data: string) => void) => { dispose: () => void };
	onResize: (handler: (size: { cols: number; rows: number }) => void) => { dispose: () => void };
	write: (data: string | Uint8Array, callback?: () => void) => void;
	reset: () => void;
	focus: () => void;
	scrollToBottom: () => void;
	dispose: () => void;
}

interface FitAddonInstance {
	fit: () => void;
}

// ── Module state ─────────────────────────────────────────────

let _container: HTMLElement | null = null;
let resizeObserver: ResizeObserver | null = null;
let themeObserver: MutationObserver | null = null;
let fitRaf = 0;
let windowResizeListener: (() => void) | null = null;
let fontsReadyListener: (() => void) | null = null;
let resizeSettleTimers: ReturnType<typeof setTimeout>[] = [];

let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let socket: WebSocket | null = null;
let shuttingDown = false;

let inputFlushTimer: ReturnType<typeof setTimeout> | null = null;
let pendingInput = "";
let windowsRefreshTimer: ReturnType<typeof setInterval> | null = null;

let terminalEl: HTMLElement | null = null;
let metaEl: HTMLElement | null = null;
let statusEl: HTMLElement | null = null;
let hintEl: HTMLElement | null = null;
let hintActionsEl: HTMLElement | null = null;
let installCommandEl: HTMLElement | null = null;
let sizeEl: HTMLElement | null = null;
let tabsEl: HTMLElement | null = null;
let newTabBtn: HTMLButtonElement | null = null;
let ctrlCBtn: HTMLButtonElement | null = null;
let clearBtn: HTMLButtonElement | null = null;
let restartBtn: HTMLButtonElement | null = null;
let installTmuxBtn: HTMLButtonElement | null = null;
let copyInstallBtn: HTMLButtonElement | null = null;

let xterm: XtermInstance | null = null;
let fitAddon: FitAddonInstance | null = null;
let xtermDataDisposable: { dispose: () => void } | null = null;
let xtermResizeDisposable: { dispose: () => void } | null = null;
let TerminalCtorRef: TerminalCtorType | null = null;
let FitAddonCtorRef: FitAddonCtorType | null = null;
let oscHandlerDisposables: { dispose: () => void }[] = [];

let terminalAvailable = false;
let lastSentCols = 0;
let lastSentRows = 0;
let tmuxInstallCommand = "";
let tmuxInstallPromptSeen = false;
let tmuxPersistenceEnabled = false;
let terminalWindows: WindowInfo[] = [];
let activeWindowId: string | null = null;
let pendingWindowId: string | null = null;
let creatingWindow = false;

const RECONNECT_DELAY_MS = 800;
const INPUT_FLUSH_MS = 16;
const WINDOW_REFRESH_MS = 2000;
const MAX_INPUT_CHUNK = 512;
const TmuxInstallPromptStorageKey = "moltis.settings.terminal.tmuxInstallPromptSeen.v1";

function readTmuxInstallPromptSeen(): boolean {
	try {
		if (typeof localStorage === "undefined") return false;
		return localStorage.getItem(TmuxInstallPromptStorageKey) === "1";
	} catch {
		return false;
	}
}

function markTmuxInstallPromptSeen(): void {
	tmuxInstallPromptSeen = true;
	try {
		if (typeof localStorage !== "undefined") localStorage.setItem(TmuxInstallPromptStorageKey, "1");
	} catch {
		/* private mode */
	}
}

function clearObservers(): void {
	if (resizeObserver) {
		resizeObserver.disconnect();
		resizeObserver = null;
	}
	if (themeObserver) {
		themeObserver.disconnect();
		themeObserver = null;
	}
	if (windowResizeListener) {
		window.removeEventListener("resize", windowResizeListener);
		windowResizeListener = null;
	}
	if (fontsReadyListener && typeof document !== "undefined" && document.fonts?.removeEventListener) {
		document.fonts.removeEventListener("loadingdone", fontsReadyListener);
		fontsReadyListener = null;
	}
}

function clearScheduledFit(): void {
	if (fitRaf) {
		cancelAnimationFrame(fitRaf);
		fitRaf = 0;
	}
}
function clearReconnectTimer(): void {
	if (reconnectTimer) {
		clearTimeout(reconnectTimer);
		reconnectTimer = null;
	}
}
function clearResizeSettleTimers(): void {
	for (const t of resizeSettleTimers) clearTimeout(t);
	resizeSettleTimers = [];
}
function clearInputQueue(): void {
	if (inputFlushTimer) {
		clearTimeout(inputFlushTimer);
		inputFlushTimer = null;
	}
	pendingInput = "";
}
function clearWindowsRefreshTimer(): void {
	if (windowsRefreshTimer) {
		clearInterval(windowsRefreshTimer);
		windowsRefreshTimer = null;
	}
}

function setStatus(text: string, level?: string): void {
	if (!statusEl) return;
	statusEl.textContent = text || "";
	statusEl.className = "terminal-status";
	if (level === "error") statusEl.classList.add("terminal-status-error");
	if (level === "ok") statusEl.classList.add("terminal-status-ok");
}

function setControlsEnabled(enabled: boolean): void {
	const allow = !!enabled;
	if (ctrlCBtn) ctrlCBtn.disabled = !allow;
	if (clearBtn) clearBtn.disabled = !allow;
	if (restartBtn) restartBtn.disabled = !allow;
	setWindowControlsEnabled();
}

function setInstallActionsVisible(visible: boolean): void {
	if (hintActionsEl) hintActionsEl.hidden = !visible;
}
function setWindowControlsEnabled(): void {
	if (newTabBtn) newTabBtn.disabled = !(tmuxPersistenceEnabled && terminalAvailable) || creatingWindow;
}

interface RawWindowPayload {
	id?: string;
	index?: number;
	name?: string;
	active?: boolean;
}

function normalizeWindowPayload(payloadWindow: unknown): WindowInfo | null {
	if (!(payloadWindow && typeof payloadWindow === "object")) return null;
	const pw = payloadWindow as RawWindowPayload;
	const id = typeof pw.id === "string" ? pw.id.trim() : "";
	if (!id) return null;
	const index = Number(pw.index);
	if (!Number.isFinite(index) || index < 0) return null;
	const name = typeof pw.name === "string" ? pw.name : "";
	return { id, index: Math.floor(index), name, active: pw.active === true };
}

function windowLabel(w: WindowInfo): string {
	const title = w.name?.trim() || "shell";
	return `${w.index}: ${title}`;
}

function renderWindowTabs(): void {
	if (!tabsEl) return;
	while (tabsEl.firstChild) tabsEl.removeChild(tabsEl.firstChild);
	if (!tmuxPersistenceEnabled) {
		const s = document.createElement("span");
		s.className = "terminal-tab-empty";
		s.textContent = "tmux unavailable";
		tabsEl.appendChild(s);
		return;
	}
	if (!terminalWindows.length) {
		const s = document.createElement("span");
		s.className = "terminal-tab-empty";
		s.textContent = "No tmux windows";
		tabsEl.appendChild(s);
		return;
	}
	for (const w of terminalWindows) {
		const tab = document.createElement("button");
		tab.type = "button";
		tab.className = "terminal-tab";
		if (w.id === activeWindowId) tab.classList.add("active");
		tab.title = `Attach ${windowLabel(w)}`;
		tab.textContent = windowLabel(w);
		tab.addEventListener("click", () => onWindowTabClick(w.id));
		tabsEl.appendChild(tab);
	}
}

function chooseActiveWindow(
	windows: WindowInfo[],
	preferred: string | null,
	payloadActive: string | null,
): string | null {
	if (!windows.length) return null;
	for (const c of [preferred, payloadActive, activeWindowId]) {
		if (c && windows.some((w) => w.id === c)) return c;
	}
	const active = windows.find((w) => w.active);
	return active ? active.id : windows[0].id;
}

function applyWindowsState(payload: WindowsPayload, preferred: string | null): void {
	const next: WindowInfo[] = [];
	for (const raw of Array.isArray(payload?.windows) ? payload.windows : []) {
		const p = normalizeWindowPayload(raw);
		if (p) next.push(p);
	}
	next.sort((a, b) => a.index - b.index);
	terminalWindows = next;
	const pa =
		typeof payload?.activeWindowId === "string" && payload.activeWindowId.trim() ? payload.activeWindowId.trim() : null;
	activeWindowId = chooseActiveWindow(next, preferred, pa);
	renderWindowTabs();
}

async function fetchTerminalWindows(): Promise<WindowsPayload> {
	const r = await fetch("/api/terminal/windows", { method: "GET", headers: { Accept: "application/json" } });
	let p: WindowsPayload;
	try {
		p = await r.json();
	} catch {
		p = {};
	}
	if (!r.ok) throw new Error(localizedApiErrorMessage(p as never, "Failed to list tmux windows"));
	return p;
}

async function refreshTerminalWindows(opts?: { preferredWindowId?: string | null; silent?: boolean }): Promise<void> {
	const preferred = opts?.preferredWindowId || pendingWindowId || null;
	try {
		const p = await fetchTerminalWindows();
		tmuxPersistenceEnabled = p?.available === true;
		applyWindowsState(p, preferred);
		if (pendingWindowId && activeWindowId === pendingWindowId) pendingWindowId = null;
		setWindowControlsEnabled();
		if (!tmuxPersistenceEnabled) clearWindowsRefreshTimer();
	} catch (e) {
		if (!opts?.silent) setStatus((e as Error)?.message || "Failed to refresh terminal windows", "error");
	}
}

function startWindowsRefreshLoop(): void {
	clearWindowsRefreshTimer();
	if (!tmuxPersistenceEnabled) return;
	windowsRefreshTimer = setInterval(() => {
		void refreshTerminalWindows({ silent: true });
	}, WINDOW_REFRESH_MS);
}

function sendSocketMessage(payload: SocketMessage): boolean {
	if (!(socket && socket.readyState === WebSocket.OPEN)) return false;
	try {
		socket.send(JSON.stringify(payload));
		return true;
	} catch {
		return false;
	}
}

function sendResizeIfChanged(force?: boolean): void {
	if (!(xterm && terminalAvailable)) return;
	const cols = xterm.cols || 0,
		rows = xterm.rows || 0;
	if (!(cols > 0 && rows > 0)) return;
	updateSizeIndicator(cols, rows);
	if (!force && cols === lastSentCols && rows === lastSentRows) return;
	lastSentCols = cols;
	lastSentRows = rows;
	sendSocketMessage({ type: "resize", cols, rows });
}

function scheduleFit(forceResize?: boolean): void {
	if (!fitAddon) return;
	const fr = forceResize === true;
	clearScheduledFit();
	fitRaf = requestAnimationFrame(() => {
		fitRaf = 0;
		if (!fitAddon) return;
		try {
			fitAddon.fit();
			sendResizeIfChanged(fr);
		} catch {
			/* transient layout */
		}
	});
}

function kickResizeSettleLoop(): void {
	if (!xterm) return;
	clearResizeSettleTimers();
	for (const d of [0, 50, 160, 380, 800]) {
		resizeSettleTimers.push(
			setTimeout(() => {
				if (xterm) {
					scheduleFit(true);
					sendResizeIfChanged(true);
				}
			}, d),
		);
	}
}

function requestWindowSwitch(windowId: string): boolean {
	if (!(tmuxPersistenceEnabled && windowId)) return false;
	pendingWindowId = windowId;
	activeWindowId = windowId;
	renderWindowTabs();
	setStatus("Switching tmux window...", "ok");
	return socket?.readyState === WebSocket.OPEN && sendSocketMessage({ type: "switch_window", window: windowId });
}

function handleActiveWindowEvent(payload: TerminalMessage): void {
	const wid = typeof payload?.windowId === "string" ? payload.windowId.trim() : "";
	if (!wid) return;
	activeWindowId = wid;
	pendingWindowId = null;
	renderWindowTabs();
	setStatus("Switched tmux window.", "ok");
	startWindowsRefreshLoop();
	kickResizeSettleLoop();
	if (xterm) xterm.focus();
	void refreshTerminalWindows({ preferredWindowId: wid, silent: true });
}

function onWindowTabClick(windowId: string): void {
	if (!(tmuxPersistenceEnabled && windowId) || windowId === activeWindowId) return;
	if (requestWindowSwitch(windowId)) return;
	terminalAvailable = false;
	setControlsEnabled(false);
	if (xterm) xterm.reset();
	connectTerminalSocket();
}

async function createTerminalWindow(): Promise<void> {
	if (!(tmuxPersistenceEnabled && terminalAvailable) || creatingWindow) return;
	creatingWindow = true;
	setWindowControlsEnabled();
	setStatus("Creating tmux window...", "ok");
	try {
		const r = await fetch("/api/terminal/windows", {
			method: "POST",
			headers: { Accept: "application/json", "Content-Type": "application/json" },
			body: JSON.stringify({}),
		});
		let p: WindowsPayload;
		try {
			p = await r.json();
		} catch {
			p = {};
		}
		if (!r.ok) throw new Error(localizedApiErrorMessage(p as never, "Failed to create tmux window"));
		const cid = p?.window?.id || p?.windowId || null;
		if (Array.isArray(p?.windows)) {
			tmuxPersistenceEnabled = true;
			applyWindowsState(p, cid);
		} else await refreshTerminalWindows({ preferredWindowId: cid, silent: true });
		if (cid && activeWindowId !== cid) {
			if (!requestWindowSwitch(cid)) {
				if (xterm) xterm.reset();
				connectTerminalSocket();
			}
		} else {
			if (xterm) xterm.reset();
			connectTerminalSocket();
		}
		setStatus("Created tmux window.", "ok");
	} catch (e) {
		setStatus((e as Error)?.message || "Failed to create tmux window", "error");
	} finally {
		creatingWindow = false;
		setWindowControlsEnabled();
	}
}

function updateSizeIndicator(cols: number, rows: number): void {
	if (!sizeEl) return;
	sizeEl.textContent = cols > 0 && rows > 0 ? `${cols}\u00d7${rows}` : "\u2014\u00d7\u2014";
}

function getCssVar(name: string, fallback: string): string {
	if (typeof document === "undefined") return fallback;
	return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

function buildXtermTheme(): Record<string, string> {
	return {
		background: getCssVar("--bg", "#0f1115"),
		foreground: getCssVar("--text", "#e4e4e7"),
		cursor: getCssVar("--accent", "#4ade80"),
		cursorAccent: getCssVar("--bg", "#0f1115"),
		selectionBackground: getCssVar("--accent-subtle", "#4ade801f"),
	};
}

function applyTheme(): void {
	if (xterm) xterm.options.theme = buildXtermTheme();
}

function registerOscStabilityGuards(): void {
	if (!(xterm?.parser && typeof xterm.parser.registerOscHandler === "function")) return;
	const swallow = () => true;
	for (const code of [4, 10, 11, 12, 104, 110, 111, 112]) {
		const d = xterm.parser.registerOscHandler(code, swallow);
		if (d && typeof d.dispose === "function") oscHandlerDisposables.push(d);
	}
}

function clearOscStabilityGuards(): void {
	for (const d of oscHandlerDisposables) {
		try {
			d.dispose();
		} catch {
			/* ignore */
		}
	}
	oscHandlerDisposables = [];
}

async function ensureXtermModules(): Promise<void> {
	if (TerminalCtorRef && FitAddonCtorRef) return;
	const [xtermMod, fitAddonMod] = await Promise.all([import("@xterm/xterm"), import("@xterm/addon-fit")]);
	TerminalCtorRef = (xtermMod as unknown as { Terminal: TerminalCtorType }).Terminal;
	FitAddonCtorRef = (fitAddonMod as unknown as { FitAddon: FitAddonCtorType }).FitAddon;
}

function queueInput(data: string): void {
	if (!terminalAvailable || typeof data !== "string" || !data.length) return;
	pendingInput += data;
	if (!inputFlushTimer)
		inputFlushTimer = setTimeout(() => {
			inputFlushTimer = null;
			flushInputQueue();
		}, INPUT_FLUSH_MS);
}

function flushInputQueue(): void {
	if (!(terminalAvailable && pendingInput)) return;
	while (pendingInput.length > 0) {
		const chunk = pendingInput.slice(0, MAX_INPUT_CHUNK);
		if (!sendSocketMessage({ type: "input", data: chunk })) break;
		pendingInput = pendingInput.slice(MAX_INPUT_CHUNK);
	}
	if (pendingInput.length > 0 && !inputFlushTimer)
		inputFlushTimer = setTimeout(() => {
			inputFlushTimer = null;
			flushInputQueue();
		}, INPUT_FLUSH_MS);
}

async function initXterm(): Promise<void> {
	if (!terminalEl) return;
	await ensureXtermModules();
	if (!(TerminalCtorRef && FitAddonCtorRef)) throw new Error("xterm failed to load");
	xterm = new TerminalCtorRef({
		convertEol: false,
		disableStdin: false,
		cursorBlink: true,
		scrollback: 4000,
		fontFamily: "JetBrains Mono, ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
		fontSize: 12,
		lineHeight: 1.35,
		theme: buildXtermTheme(),
	});
	registerOscStabilityGuards();
	fitAddon = new FitAddonCtorRef();
	xterm.loadAddon(fitAddon);
	xterm.open(terminalEl);
	xtermDataDisposable = xterm.onData((d: string) => queueInput(d));
	xtermResizeDisposable = xterm.onResize((sz: { cols: number; rows: number }) => {
		updateSizeIndicator(sz.cols, sz.rows);
		sendResizeIfChanged();
	});
	scheduleFit();
	terminalEl.addEventListener("click", () => {
		if (xterm) xterm.focus();
	});
	if (typeof ResizeObserver !== "undefined") {
		resizeObserver = new ResizeObserver(() => scheduleFit());
		resizeObserver.observe(terminalEl.parentElement || terminalEl);
	}
	if (typeof window !== "undefined") {
		windowResizeListener = () => scheduleFit();
		window.addEventListener("resize", windowResizeListener);
	}
	if (typeof document !== "undefined" && document.fonts?.ready && typeof document.fonts.ready.then === "function")
		document.fonts.ready.then(() => scheduleFit());
	if (typeof document !== "undefined" && document.fonts?.addEventListener) {
		fontsReadyListener = () => scheduleFit();
		document.fonts.addEventListener("loadingdone", fontsReadyListener);
	}
	themeObserver = new MutationObserver(() => applyTheme());
	themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
}

function disposeXterm(): void {
	clearObservers();
	clearScheduledFit();
	clearResizeSettleTimers();
	clearOscStabilityGuards();
	if (xtermDataDisposable) {
		xtermDataDisposable.dispose();
		xtermDataDisposable = null;
	}
	if (xtermResizeDisposable) {
		xtermResizeDisposable.dispose();
		xtermResizeDisposable = null;
	}
	if (xterm) {
		xterm.dispose();
		xterm = null;
	}
	fitAddon = null;
	lastSentCols = 0;
	lastSentRows = 0;
	updateSizeIndicator(0, 0);
}

function isNearBottom(): boolean {
	if (!xterm) return false;
	const b = xterm.buffer.active;
	return !b || b.baseY - b.viewportY <= 2;
}

function decodeBase64ToBytes(encoded: string): Uint8Array | null {
	if (!encoded) return null;
	try {
		const bin = atob(encoded);
		const b = new Uint8Array(bin.length);
		for (let i = 0; i < bin.length; i++) b[i] = bin.charCodeAt(i) & 0xff;
		return b;
	} catch {
		return null;
	}
}

function writeToXterm(chunk: string | Uint8Array, scrollBottom: boolean): void {
	if (!xterm) return;
	if ((typeof chunk === "string" && !chunk.length) || (chunk instanceof Uint8Array && !chunk.length)) {
		if (scrollBottom) xterm.scrollToBottom();
		return;
	}
	xterm.write(chunk, () => {
		if (scrollBottom && xterm) xterm.scrollToBottom();
	});
}

function appendOutputChunk(chunk: string | Uint8Array, forceBottom: boolean): void {
	if (!xterm) return;
	writeToXterm(chunk, forceBottom || isNearBottom());
}

function closeTerminalSocket(): void {
	if (!socket) return;
	const ws = socket;
	socket = null;
	ws.onopen = null;
	ws.onmessage = null;
	ws.onerror = null;
	ws.onclose = null;
	if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) ws.close();
}

function scheduleReconnect(): void {
	if (shuttingDown || reconnectTimer) return;
	reconnectTimer = setTimeout(() => {
		reconnectTimer = null;
		connectTerminalSocket();
	}, RECONNECT_DELAY_MS);
}

function applyReadyPayload(payload: ReadyPayload): void {
	terminalAvailable = !!payload.available;
	setControlsEnabled(terminalAvailable);
	const pe = !!payload.persistenceEnabled;
	tmuxPersistenceEnabled = pe;
	const pa = !!payload.persistenceAvailable;
	const paid =
		typeof payload.activeWindowId === "string" && payload.activeWindowId.trim() ? payload.activeWindowId.trim() : null;
	if (paid) activeWindowId = paid;
	pendingWindowId = null;
	const ic = payload.tmuxInstallCommand || "";
	const shouldOffer = terminalAvailable && !pe && !pa && ic.length > 0;
	const first = shouldOffer && !tmuxInstallPromptSeen;
	tmuxInstallCommand = shouldOffer ? ic : "";
	if (installCommandEl) installCommandEl.textContent = tmuxInstallCommand;
	if (installTmuxBtn) installTmuxBtn.textContent = first ? "Run install command (first time)" : "Run install command";
	setInstallActionsVisible(shouldOffer);
	if (metaEl)
		metaEl.textContent = terminalAvailable
			? pe
				? `Persistent tmux session, user ${payload.user || "unknown"}`
				: `Ephemeral host shell, user ${payload.user || "unknown"}`
			: "Host shell unavailable";
	if (hintEl) {
		if (!terminalAvailable) hintEl.textContent = "Unable to open host shell.";
		else if (pe)
			hintEl.textContent =
				"Interactive host shell with persistent tmux session. Click inside terminal and type commands directly.";
		else if (pa)
			hintEl.textContent =
				"Interactive host shell (ephemeral). Enable tmux persistence from terminal settings when available.";
		else if (ic)
			hintEl.textContent = first
				? "First connection tip: run the install command once to enable persistent tmux sessions."
				: `Interactive host shell (ephemeral). Install tmux for persistence: ${ic}`;
		else hintEl.textContent = "Interactive host shell (ephemeral). Install tmux to persist sessions across reconnects.";
	}
	if (first) markTmuxInstallPromptSeen();
	renderWindowTabs();
	setWindowControlsEnabled();
	if (terminalAvailable) {
		kickResizeSettleLoop();
		updateSizeIndicator(xterm?.cols || 0, xterm?.rows || 0);
		if (pe) {
			setStatus("Connected to host shell with persistent tmux session.", "ok");
			startWindowsRefreshLoop();
			void refreshTerminalWindows({ preferredWindowId: activeWindowId, silent: true });
		} else {
			setStatus("Connected to host shell (ephemeral session).", "ok");
			clearWindowsRefreshTimer();
		}
		flushInputQueue();
		if (xterm) xterm.focus();
	} else {
		clearWindowsRefreshTimer();
		updateSizeIndicator(0, 0);
		setStatus("Failed to open host shell.", "error");
	}
}

function handleTerminalMessage(payload: TerminalMessage): void {
	if (!(payload && typeof payload === "object")) return;
	switch (payload.type) {
		case "ready":
			applyReadyPayload(payload as unknown as ReadyPayload);
			break;
		case "active_window":
			handleActiveWindowEvent(payload);
			break;
		case "output":
			if (payload.encoding === "base64") {
				const b = decodeBase64ToBytes(payload.data || "");
				if (b) appendOutputChunk(b, false);
			} else appendOutputChunk(payload.data || "", false);
			break;
		case "status":
			setStatus(payload.text || "", payload.level || "");
			break;
		case "error":
			setStatus(payload.error || "Terminal error", "error");
			break;
		case "pong":
			break;
		default:
			break;
	}
}

function connectTerminalSocket(): void {
	if (typeof WebSocket === "undefined") {
		setStatus("WebSocket not supported in this browser", "error");
		return;
	}
	clearReconnectTimer();
	clearResizeSettleTimers();
	closeTerminalSocket();
	lastSentCols = 0;
	lastSentRows = 0;
	const proto = location.protocol === "https:" ? "wss:" : "ws:";
	let wsUrl = `${proto}//${location.host}/api/terminal/ws`;
	const tw = pendingWindowId || activeWindowId;
	if (tmuxPersistenceEnabled && tw) wsUrl += `?window=${encodeURIComponent(tw)}`;
	socket = new WebSocket(wsUrl);
	setStatus("Connecting terminal websocket...");
	socket.onopen = () => setStatus("Terminal websocket connected.", "ok");
	socket.onmessage = (ev: MessageEvent) => {
		let p: TerminalMessage | null = null;
		try {
			p = JSON.parse(ev.data as string);
		} catch {
			return;
		}
		if (p) handleTerminalMessage(p);
	};
	socket.onerror = () => {
		/* onclose handles */
	};
	socket.onclose = () => {
		socket = null;
		setControlsEnabled(false);
		terminalAvailable = false;
		clearWindowsRefreshTimer();
		setWindowControlsEnabled();
		if (shuttingDown) return;
		setStatus("Terminal disconnected. Reconnecting...", "error");
		scheduleReconnect();
	};
}

function sendControl(action: string): void {
	if (terminalAvailable) sendSocketMessage({ type: "control", action });
}

function bindEvents(): void {
	if (newTabBtn)
		newTabBtn.addEventListener("click", () => {
			void createTerminalWindow();
		});
	if (ctrlCBtn) ctrlCBtn.addEventListener("click", () => sendControl("ctrl_c"));
	if (clearBtn) clearBtn.addEventListener("click", () => sendControl("clear"));
	if (restartBtn) restartBtn.addEventListener("click", () => sendControl("restart"));
	if (installTmuxBtn)
		installTmuxBtn.addEventListener("click", () => {
			if (!(terminalAvailable && tmuxInstallCommand)) return;
			if (!sendSocketMessage({ type: "input", data: `${tmuxInstallCommand}\n` })) {
				setStatus("Failed to queue install command.", "error");
				return;
			}
			setStatus(`Queued install command: ${tmuxInstallCommand}`, "ok");
			if (xterm) xterm.focus();
		});
	if (copyInstallBtn)
		copyInstallBtn.addEventListener("click", async () => {
			if (!tmuxInstallCommand) return;
			if (!navigator.clipboard?.writeText) {
				setStatus("Clipboard API unavailable in this browser.", "error");
				return;
			}
			try {
				await navigator.clipboard.writeText(tmuxInstallCommand);
				setStatus("Install command copied to clipboard.", "ok");
			} catch {
				setStatus("Failed to copy install command.", "error");
			}
		});
}

// Static HTML template for terminal page layout. No user input is interpolated.
function buildTerminalHtml(): string {
	return [
		'<div class="terminal-page">',
		'<div class="terminal-toolbar">',
		'<div class="terminal-heading">',
		'<h2 class="text-lg font-medium text-[var(--text-strong)]">Terminal</h2>',
		'<div id="terminalMeta" class="terminal-meta"></div>',
		"</div>",
		'<div class="terminal-actions">',
		'<div id="terminalSize" class="terminal-size" title="Terminal size (columns \u00d7 rows)">\u2014\u00d7\u2014</div>',
		'<button id="terminalCtrlC" class="logs-btn" type="button" title="Send Ctrl+C">Ctrl+C</button>',
		'<button id="terminalClear" class="logs-btn" type="button" title="Send Ctrl+L">Clear</button>',
		'<button id="terminalRestart" class="logs-btn" type="button">Restart</button>',
		"</div></div>",
		'<div class="terminal-tabs-bar">',
		'<div id="terminalTabs" class="terminal-tabs" aria-label="tmux windows"></div>',
		'<button id="terminalNewTab" class="logs-btn terminal-new-tab" type="button" title="Create tmux window">+ Tab</button>',
		"</div>",
		'<div class="terminal-output-wrap">',
		'<div id="terminalOutput" class="terminal-output" aria-label="Host terminal output"></div>',
		"</div>",
		'<div id="terminalStatus" class="terminal-status"></div>',
		'<div id="terminalHint" class="terminal-hint">Interactive host shell. Click inside terminal and type commands directly.</div>',
		'<div id="terminalHintActions" class="terminal-hint-actions" hidden>',
		'<code id="terminalInstallCommand" class="terminal-hint-code"></code>',
		'<button id="terminalInstallTmux" class="logs-btn terminal-hint-btn terminal-hint-btn-primary" type="button">Run install command</button>',
		'<button id="terminalCopyInstall" class="logs-btn terminal-hint-btn" type="button">Copy</button>',
		"</div></div>",
	].join("");
}

export async function initTerminal(container: HTMLElement): Promise<void> {
	_container = container;
	shuttingDown = false;
	tmuxInstallPromptSeen = readTmuxInstallPromptSeen();
	tmuxInstallCommand = "";
	container.style.cssText = "display:flex;flex-direction:column;padding:0;overflow:hidden;min-height:0;";

	// Safe: static HTML with no user-supplied values
	const tpl = document.createElement("template");
	tpl.innerHTML = buildTerminalHtml();
	container.appendChild(tpl.content);

	terminalEl = container.querySelector("#terminalOutput");
	metaEl = container.querySelector("#terminalMeta");
	statusEl = container.querySelector("#terminalStatus");
	hintEl = container.querySelector("#terminalHint");
	hintActionsEl = container.querySelector("#terminalHintActions");
	installCommandEl = container.querySelector("#terminalInstallCommand");
	sizeEl = container.querySelector("#terminalSize");
	tabsEl = container.querySelector("#terminalTabs");
	newTabBtn = container.querySelector("#terminalNewTab");
	ctrlCBtn = container.querySelector("#terminalCtrlC");
	clearBtn = container.querySelector("#terminalClear");
	restartBtn = container.querySelector("#terminalRestart");
	installTmuxBtn = container.querySelector("#terminalInstallTmux");
	copyInstallBtn = container.querySelector("#terminalCopyInstall");

	setStatus("Initializing terminal...");
	setControlsEnabled(false);
	renderWindowTabs();
	bindEvents();

	try {
		await initXterm();
		await refreshTerminalWindows({ silent: true });
		connectTerminalSocket();
	} catch (err) {
		setStatus((err as Error).message || "Failed to initialize terminal", "error");
	}
}

export function teardownTerminal(): void {
	shuttingDown = true;
	clearReconnectTimer();
	clearResizeSettleTimers();
	closeTerminalSocket();
	clearInputQueue();
	clearWindowsRefreshTimer();
	disposeXterm();
	if (_container) while (_container.firstChild) _container.removeChild(_container.firstChild);
	_container = null;
	terminalEl = null;
	metaEl = null;
	statusEl = null;
	hintEl = null;
	hintActionsEl = null;
	installCommandEl = null;
	sizeEl = null;
	tabsEl = null;
	newTabBtn = null;
	ctrlCBtn = null;
	clearBtn = null;
	restartBtn = null;
	installTmuxBtn = null;
	copyInstallBtn = null;
	terminalAvailable = false;
	tmuxPersistenceEnabled = false;
	terminalWindows = [];
	activeWindowId = null;
	pendingWindowId = null;
	creatingWindow = false;
	tmuxInstallCommand = "";
}

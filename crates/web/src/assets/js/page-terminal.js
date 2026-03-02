// Settings > Terminal (host shell via PTY + xterm.js over WebSocket)

import { localizedApiErrorMessage } from "./helpers.js";

var _container = null;
var resizeObserver = null;
var themeObserver = null;
var fitRaf = 0;
var windowResizeListener = null;
var fontsReadyListener = null;
var resizeSettleTimers = [];

var reconnectTimer = null;
var socket = null;
var shuttingDown = false;

var inputFlushTimer = null;
var pendingInput = "";
var windowsRefreshTimer = null;

var terminalEl = null;
var metaEl = null;
var statusEl = null;
var hintEl = null;
var hintActionsEl = null;
var installCommandEl = null;
var sizeEl = null;
var tabsEl = null;
var newTabBtn = null;
var ctrlCBtn = null;
var clearBtn = null;
var restartBtn = null;
var installTmuxBtn = null;
var copyInstallBtn = null;

var xterm = null;
var fitAddon = null;
var xtermDataDisposable = null;
var xtermResizeDisposable = null;
var TerminalCtor = null;
var FitAddonCtor = null;
var oscHandlerDisposables = [];

var terminalAvailable = false;
var lastSentCols = 0;
var lastSentRows = 0;
var tmuxInstallCommand = "";
var tmuxInstallPromptSeen = false;
var tmuxPersistenceEnabled = false;
var terminalWindows = [];
var activeWindowId = null;
var pendingWindowId = null;
var creatingWindow = false;

var RECONNECT_DELAY_MS = 800;
var INPUT_FLUSH_MS = 16;
var WINDOW_REFRESH_MS = 2000;
var MAX_INPUT_CHUNK = 512;
var TmuxInstallPromptStorageKey = "moltis.settings.terminal.tmuxInstallPromptSeen.v1";

function readTmuxInstallPromptSeen() {
	try {
		if (typeof localStorage === "undefined") return false;
		return localStorage.getItem(TmuxInstallPromptStorageKey) === "1";
	} catch {
		return false;
	}
}

function markTmuxInstallPromptSeen() {
	tmuxInstallPromptSeen = true;
	try {
		if (typeof localStorage !== "undefined") {
			localStorage.setItem(TmuxInstallPromptStorageKey, "1");
		}
	} catch {
		// Ignore storage write errors in private/incognito contexts.
	}
}

function clearObservers() {
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

function clearScheduledFit() {
	if (fitRaf) {
		cancelAnimationFrame(fitRaf);
		fitRaf = 0;
	}
}

function clearReconnectTimer() {
	if (reconnectTimer) {
		clearTimeout(reconnectTimer);
		reconnectTimer = null;
	}
}

function clearResizeSettleTimers() {
	for (const timer of resizeSettleTimers) {
		clearTimeout(timer);
	}
	resizeSettleTimers = [];
}

function clearInputQueue() {
	if (inputFlushTimer) {
		clearTimeout(inputFlushTimer);
		inputFlushTimer = null;
	}
	pendingInput = "";
}

function clearWindowsRefreshTimer() {
	if (windowsRefreshTimer) {
		clearInterval(windowsRefreshTimer);
		windowsRefreshTimer = null;
	}
}

function setStatus(text, level) {
	if (!statusEl) return;
	statusEl.textContent = text || "";
	statusEl.className = "terminal-status";
	if (level === "error") statusEl.classList.add("terminal-status-error");
	if (level === "ok") statusEl.classList.add("terminal-status-ok");
}

function setControlsEnabled(enabled) {
	var allow = !!enabled;
	if (ctrlCBtn) ctrlCBtn.disabled = !allow;
	if (clearBtn) clearBtn.disabled = !allow;
	if (restartBtn) restartBtn.disabled = !allow;
	setWindowControlsEnabled();
}

function setInstallActionsVisible(visible) {
	if (!hintActionsEl) return;
	hintActionsEl.hidden = !visible;
}

function setWindowControlsEnabled() {
	if (!newTabBtn) return;
	newTabBtn.disabled = !(tmuxPersistenceEnabled && terminalAvailable) || creatingWindow;
}

function normalizeWindowPayload(payloadWindow) {
	if (!(payloadWindow && typeof payloadWindow === "object")) return null;
	var id = typeof payloadWindow.id === "string" ? payloadWindow.id.trim() : "";
	if (!id) return null;
	var index = Number(payloadWindow.index);
	if (!Number.isFinite(index) || index < 0) return null;
	var name = typeof payloadWindow.name === "string" ? payloadWindow.name : "";
	return {
		id: id,
		index: Math.floor(index),
		name: name,
		active: payloadWindow.active === true,
	};
}

function windowLabel(windowInfo) {
	var title = windowInfo.name?.trim() ? windowInfo.name.trim() : "shell";
	return `${windowInfo.index}: ${title}`;
}

function renderWindowTabs() {
	if (!tabsEl) return;
	tabsEl.innerHTML = "";
	if (!tmuxPersistenceEnabled) {
		var unsupported = document.createElement("span");
		unsupported.className = "terminal-tab-empty";
		unsupported.textContent = "tmux unavailable";
		tabsEl.appendChild(unsupported);
		return;
	}
	if (!terminalWindows.length) {
		var empty = document.createElement("span");
		empty.className = "terminal-tab-empty";
		empty.textContent = "No tmux windows";
		tabsEl.appendChild(empty);
		return;
	}
	for (const windowInfo of terminalWindows) {
		var tab = document.createElement("button");
		tab.type = "button";
		tab.className = "terminal-tab";
		if (windowInfo.id === activeWindowId) tab.classList.add("active");
		tab.title = `Attach ${windowLabel(windowInfo)}`;
		tab.textContent = windowLabel(windowInfo);
		tab.addEventListener("click", () => {
			onWindowTabClick(windowInfo.id);
		});
		tabsEl.appendChild(tab);
	}
}

function chooseActiveWindow(windows, preferredWindowId, payloadActiveWindowId) {
	if (!windows.length) return null;
	var orderedCandidates = [preferredWindowId, payloadActiveWindowId, activeWindowId];
	for (const candidate of orderedCandidates) {
		if (!candidate) continue;
		for (const windowInfo of windows) {
			if (windowInfo.id === candidate) return windowInfo.id;
		}
	}
	for (const windowInfo of windows) {
		if (windowInfo.active) return windowInfo.id;
	}
	return windows[0].id;
}

function applyWindowsState(payload, preferredWindowId) {
	var nextWindows = [];
	var rawWindows = Array.isArray(payload?.windows) ? payload.windows : [];
	for (const rawWindow of rawWindows) {
		var parsed = normalizeWindowPayload(rawWindow);
		if (parsed) nextWindows.push(parsed);
	}
	nextWindows.sort((a, b) => a.index - b.index);
	terminalWindows = nextWindows;
	var payloadActiveWindowId =
		typeof payload?.activeWindowId === "string" && payload.activeWindowId.trim() ? payload.activeWindowId.trim() : null;
	activeWindowId = chooseActiveWindow(nextWindows, preferredWindowId, payloadActiveWindowId);
	renderWindowTabs();
}

async function fetchTerminalWindows() {
	var response = await fetch("/api/terminal/windows", {
		method: "GET",
		headers: { Accept: "application/json" },
	});
	var payload = null;
	try {
		payload = await response.json();
	} catch {
		payload = {};
	}
	if (!response.ok) {
		throw new Error(localizedApiErrorMessage(payload, "Failed to list tmux windows"));
	}
	return payload;
}

async function refreshTerminalWindows(options) {
	var preferredWindowId = options?.preferredWindowId || pendingWindowId || null;
	var silent = options?.silent === true;
	try {
		var payload = await fetchTerminalWindows();
		tmuxPersistenceEnabled = payload?.available === true;
		applyWindowsState(payload, preferredWindowId);
		if (pendingWindowId && activeWindowId === pendingWindowId) {
			pendingWindowId = null;
		}
		setWindowControlsEnabled();
		if (!tmuxPersistenceEnabled) {
			clearWindowsRefreshTimer();
		}
	} catch (err) {
		if (!silent) {
			setStatus(err?.message || "Failed to refresh terminal windows", "error");
		}
	}
}

function startWindowsRefreshLoop() {
	clearWindowsRefreshTimer();
	if (!tmuxPersistenceEnabled) return;
	windowsRefreshTimer = setInterval(() => {
		void refreshTerminalWindows({ silent: true });
	}, WINDOW_REFRESH_MS);
}

function requestWindowSwitch(windowId) {
	if (!tmuxPersistenceEnabled) return false;
	if (!windowId) return false;
	pendingWindowId = windowId;
	activeWindowId = windowId;
	renderWindowTabs();
	setStatus("Switching tmux window...", "ok");
	if (
		socket &&
		socket.readyState === WebSocket.OPEN &&
		sendSocketMessage({ type: "switch_window", window: windowId })
	) {
		return true;
	}
	return false;
}

function handleActiveWindowEvent(payload) {
	var windowId = typeof payload?.windowId === "string" ? payload.windowId.trim() : "";
	if (!windowId) return;
	activeWindowId = windowId;
	pendingWindowId = null;
	renderWindowTabs();
	setStatus("Switched tmux window.", "ok");
	startWindowsRefreshLoop();
	kickResizeSettleLoop();
	if (xterm) xterm.focus();
	void refreshTerminalWindows({ preferredWindowId: windowId, silent: true });
}

function onWindowTabClick(windowId) {
	if (!tmuxPersistenceEnabled) return;
	if (!windowId || windowId === activeWindowId) return;
	if (requestWindowSwitch(windowId)) return;
	terminalAvailable = false;
	setControlsEnabled(false);
	if (xterm) xterm.reset();
	connectTerminalSocket();
}

async function createTerminalWindow() {
	if (!(tmuxPersistenceEnabled && terminalAvailable) || creatingWindow) return;
	creatingWindow = true;
	setWindowControlsEnabled();
	setStatus("Creating tmux window...", "ok");
	try {
		var response = await fetch("/api/terminal/windows", {
			method: "POST",
			headers: {
				Accept: "application/json",
				"Content-Type": "application/json",
			},
			body: JSON.stringify({}),
		});
		var payload = null;
		try {
			payload = await response.json();
		} catch {
			payload = {};
		}
		if (!response.ok) {
			throw new Error(localizedApiErrorMessage(payload, "Failed to create tmux window"));
		}
		var createdWindowId = payload?.window?.id || payload?.windowId || null;
		if (Array.isArray(payload?.windows)) {
			tmuxPersistenceEnabled = true;
			applyWindowsState(payload, createdWindowId);
		} else {
			await refreshTerminalWindows({ preferredWindowId: createdWindowId, silent: true });
		}
		if (createdWindowId && activeWindowId !== createdWindowId) {
			var switchedInBand = requestWindowSwitch(createdWindowId);
			if (!switchedInBand) {
				if (xterm) xterm.reset();
				connectTerminalSocket();
			}
		} else {
			if (xterm) xterm.reset();
			connectTerminalSocket();
		}
		setStatus("Created tmux window.", "ok");
	} catch (err) {
		setStatus(err?.message || "Failed to create tmux window", "error");
	} finally {
		creatingWindow = false;
		setWindowControlsEnabled();
	}
}

function updateSizeIndicator(cols, rows) {
	if (!sizeEl) return;
	if (cols > 0 && rows > 0) {
		sizeEl.textContent = `${cols}\u00d7${rows}`;
		return;
	}
	sizeEl.textContent = "\u2014\u00d7\u2014";
}

function getCssVar(name, fallback) {
	if (typeof document === "undefined") return fallback;
	var style = getComputedStyle(document.documentElement);
	return style.getPropertyValue(name).trim() || fallback;
}

function buildXtermTheme() {
	return {
		background: getCssVar("--bg", "#0f1115"),
		foreground: getCssVar("--text", "#e4e4e7"),
		cursor: getCssVar("--accent", "#4ade80"),
		cursorAccent: getCssVar("--bg", "#0f1115"),
		selectionBackground: getCssVar("--accent-subtle", "#4ade801f"),
	};
}

function applyTheme() {
	if (!xterm) return;
	xterm.options.theme = buildXtermTheme();
}

function registerOscStabilityGuards() {
	if (!(xterm?.parser && typeof xterm.parser.registerOscHandler === "function")) {
		return;
	}
	var swallow = () => true;
	var guardedCodes = [4, 10, 11, 12, 104, 110, 111, 112];
	for (const code of guardedCodes) {
		var disposable = xterm.parser.registerOscHandler(code, swallow);
		if (disposable && typeof disposable.dispose === "function") {
			oscHandlerDisposables.push(disposable);
		}
	}
}

function clearOscStabilityGuards() {
	for (const disposable of oscHandlerDisposables) {
		try {
			disposable.dispose();
		} catch {
			// Ignore parser teardown errors during xterm disposal.
		}
	}
	oscHandlerDisposables = [];
}

function sendSocketMessage(payload) {
	if (!(socket && socket.readyState === WebSocket.OPEN)) return false;
	try {
		socket.send(JSON.stringify(payload));
		return true;
	} catch {
		return false;
	}
}

function sendResizeIfChanged(forceSend) {
	if (!xterm) return;
	if (!terminalAvailable) return;
	var force = forceSend === true;
	var cols = xterm.cols || 0;
	var rows = xterm.rows || 0;
	if (!(cols > 0 && rows > 0)) return;
	updateSizeIndicator(cols, rows);
	if (!force && cols === lastSentCols && rows === lastSentRows) return;
	lastSentCols = cols;
	lastSentRows = rows;
	sendSocketMessage({ type: "resize", cols: cols, rows: rows });
}

function scheduleFit(forceResize) {
	if (!fitAddon) return;
	var shouldForceResize = forceResize === true;
	clearScheduledFit();
	fitRaf = requestAnimationFrame(() => {
		fitRaf = 0;
		if (!fitAddon) return;
		try {
			fitAddon.fit();
			sendResizeIfChanged(shouldForceResize);
		} catch {
			// xterm may throw during transient detach or hidden layout states.
		}
	});
}

function kickResizeSettleLoop() {
	if (!xterm) return;
	clearResizeSettleTimers();
	var settleDelays = [0, 50, 160, 380, 800];
	for (const delay of settleDelays) {
		var timer = setTimeout(() => {
			if (!xterm) return;
			scheduleFit(true);
			sendResizeIfChanged(true);
		}, delay);
		resizeSettleTimers.push(timer);
	}
}

async function ensureXtermModules() {
	if (TerminalCtor && FitAddonCtor) return;
	var [xtermMod, fitAddonMod] = await Promise.all([import("@xterm/xterm"), import("@xterm/addon-fit")]);
	TerminalCtor = xtermMod.Terminal;
	FitAddonCtor = fitAddonMod.FitAddon;
}

function queueInput(data) {
	if (!terminalAvailable) return;
	if (typeof data !== "string" || data.length === 0) return;
	pendingInput += data;
	if (!inputFlushTimer) {
		inputFlushTimer = setTimeout(() => {
			inputFlushTimer = null;
			flushInputQueue();
		}, INPUT_FLUSH_MS);
	}
}

function flushInputQueue() {
	if (!(terminalAvailable && pendingInput)) return;
	while (pendingInput.length > 0) {
		var chunk = pendingInput.slice(0, MAX_INPUT_CHUNK);
		if (!sendSocketMessage({ type: "input", data: chunk })) {
			break;
		}
		pendingInput = pendingInput.slice(MAX_INPUT_CHUNK);
	}
	if (pendingInput.length > 0 && !inputFlushTimer) {
		inputFlushTimer = setTimeout(() => {
			inputFlushTimer = null;
			flushInputQueue();
		}, INPUT_FLUSH_MS);
	}
}

async function initXterm() {
	if (!terminalEl) return;
	await ensureXtermModules();
	if (!(TerminalCtor && FitAddonCtor)) {
		throw new Error("xterm failed to load");
	}

	xterm = new TerminalCtor({
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
	fitAddon = new FitAddonCtor();
	xterm.loadAddon(fitAddon);
	xterm.open(terminalEl);
	xtermDataDisposable = xterm.onData((data) => {
		queueInput(data);
	});
	xtermResizeDisposable = xterm.onResize((size) => {
		updateSizeIndicator(size.cols || 0, size.rows || 0);
		sendResizeIfChanged();
	});
	scheduleFit();

	terminalEl.addEventListener("click", () => {
		if (xterm) xterm.focus();
	});

	if (typeof ResizeObserver !== "undefined") {
		resizeObserver = new ResizeObserver(() => {
			scheduleFit();
		});
		resizeObserver.observe(terminalEl.parentElement || terminalEl);
	}
	if (typeof window !== "undefined") {
		windowResizeListener = () => {
			scheduleFit();
		};
		window.addEventListener("resize", windowResizeListener);
	}
	if (typeof document !== "undefined" && document.fonts?.ready && typeof document.fonts.ready.then === "function") {
		document.fonts.ready.then(() => {
			scheduleFit();
		});
	}
	if (typeof document !== "undefined" && document.fonts?.addEventListener) {
		fontsReadyListener = () => {
			scheduleFit();
		};
		document.fonts.addEventListener("loadingdone", fontsReadyListener);
	}

	themeObserver = new MutationObserver(() => {
		applyTheme();
	});
	themeObserver.observe(document.documentElement, {
		attributes: true,
		attributeFilter: ["data-theme"],
	});
}

function disposeXterm() {
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

function isNearBottom() {
	if (!xterm) return false;
	var buffer = xterm.buffer.active;
	if (!buffer) return true;
	return buffer.baseY - buffer.viewportY <= 2;
}

function decodeBase64ToBytes(encoded) {
	if (typeof encoded !== "string" || encoded.length === 0) return null;
	try {
		var binary = atob(encoded);
		var bytes = new Uint8Array(binary.length);
		for (let i = 0; i < binary.length; i += 1) {
			bytes[i] = binary.charCodeAt(i) & 0xff;
		}
		return bytes;
	} catch {
		return null;
	}
}

function writeToXterm(chunk, scrollBottom) {
	if (!xterm) return;
	var content = typeof chunk === "string" ? chunk : chunk instanceof Uint8Array ? chunk : null;
	var isEmptyString = typeof content === "string" && content.length === 0;
	var isEmptyBytes = content instanceof Uint8Array && content.length === 0;
	if (!content || isEmptyString || isEmptyBytes) {
		if (scrollBottom) xterm.scrollToBottom();
		return;
	}
	xterm.write(content, () => {
		if (scrollBottom && xterm) xterm.scrollToBottom();
	});
}

function appendOutputChunk(chunk, forceBottom) {
	if (!xterm) return;
	if (typeof chunk !== "string" && !(chunk instanceof Uint8Array)) return;
	var atBottom = forceBottom || isNearBottom();
	writeToXterm(chunk, atBottom);
}

function closeTerminalSocket() {
	if (!socket) return;
	var ws = socket;
	socket = null;
	ws.onopen = null;
	ws.onmessage = null;
	ws.onerror = null;
	ws.onclose = null;
	if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
		ws.close();
	}
}

function scheduleReconnect() {
	if (shuttingDown || reconnectTimer) return;
	reconnectTimer = setTimeout(() => {
		reconnectTimer = null;
		connectTerminalSocket();
	}, RECONNECT_DELAY_MS);
}

function applyReadyPayload(payload) {
	terminalAvailable = !!payload.available;
	setControlsEnabled(terminalAvailable);
	var persistenceEnabled = !!payload.persistenceEnabled;
	tmuxPersistenceEnabled = persistenceEnabled;
	var persistenceAvailable = !!payload.persistenceAvailable;
	var payloadActiveWindowId =
		typeof payload.activeWindowId === "string" && payload.activeWindowId.trim() ? payload.activeWindowId.trim() : null;
	if (payloadActiveWindowId) {
		activeWindowId = payloadActiveWindowId;
	}
	pendingWindowId = null;
	var installCommand = payload.tmuxInstallCommand || "";
	var shouldOfferInstall =
		terminalAvailable && !persistenceEnabled && !persistenceAvailable && installCommand.length > 0;
	var firstTimeOffer = shouldOfferInstall && !tmuxInstallPromptSeen;
	tmuxInstallCommand = shouldOfferInstall ? installCommand : "";
	if (installCommandEl) {
		installCommandEl.textContent = tmuxInstallCommand;
	}
	if (installTmuxBtn) {
		installTmuxBtn.textContent = firstTimeOffer ? "Run install command (first time)" : "Run install command";
	}
	setInstallActionsVisible(shouldOfferInstall);

	if (metaEl) {
		if (terminalAvailable) {
			var user = payload.user || "unknown";
			if (persistenceEnabled) {
				metaEl.textContent = `Persistent tmux session, user ${user}`;
			} else {
				metaEl.textContent = `Ephemeral host shell, user ${user}`;
			}
		} else {
			metaEl.textContent = "Host shell unavailable";
		}
	}

	if (hintEl) {
		if (!terminalAvailable) {
			hintEl.textContent = "Unable to open host shell.";
		} else if (persistenceEnabled) {
			hintEl.textContent =
				"Interactive host shell with persistent tmux session. Click inside terminal and type commands directly.";
		} else if (persistenceAvailable) {
			hintEl.textContent =
				"Interactive host shell (ephemeral). Enable tmux persistence from terminal settings when available.";
		} else if (installCommand) {
			if (firstTimeOffer) {
				hintEl.textContent = "First connection tip: run the install command once to enable persistent tmux sessions.";
			} else {
				hintEl.textContent = `Interactive host shell (ephemeral). Install tmux for persistence: ${installCommand}`;
			}
		} else {
			hintEl.textContent = "Interactive host shell (ephemeral). Install tmux to persist sessions across reconnects.";
		}
	}

	if (firstTimeOffer) {
		markTmuxInstallPromptSeen();
	}
	renderWindowTabs();
	setWindowControlsEnabled();

	if (terminalAvailable) {
		kickResizeSettleLoop();
		updateSizeIndicator(xterm?.cols || 0, xterm?.rows || 0);
		if (persistenceEnabled) {
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

function handleTerminalMessage(payload) {
	if (!(payload && typeof payload === "object")) return;
	switch (payload.type) {
		case "ready":
			applyReadyPayload(payload);
			break;
		case "active_window":
			handleActiveWindowEvent(payload);
			break;
		case "output":
			if (payload.encoding === "base64") {
				var bytes = decodeBase64ToBytes(payload.data || "");
				if (bytes) {
					appendOutputChunk(bytes, false);
				}
			} else {
				appendOutputChunk(payload.data || "", false);
			}
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

function connectTerminalSocket() {
	if (typeof WebSocket === "undefined") {
		setStatus("WebSocket not supported in this browser", "error");
		return;
	}

	clearReconnectTimer();
	clearResizeSettleTimers();
	closeTerminalSocket();
	lastSentCols = 0;
	lastSentRows = 0;

	var proto = location.protocol === "https:" ? "wss:" : "ws:";
	var wsUrl = `${proto}//${location.host}/api/terminal/ws`;
	var targetWindowId = pendingWindowId || activeWindowId;
	if (tmuxPersistenceEnabled && targetWindowId) {
		wsUrl += `?window=${encodeURIComponent(targetWindowId)}`;
	}
	socket = new WebSocket(wsUrl);
	setStatus("Connecting terminal websocket...");

	socket.onopen = () => {
		setStatus("Terminal websocket connected.", "ok");
	};

	socket.onmessage = (event) => {
		var payload = null;
		try {
			payload = JSON.parse(event.data);
		} catch {
			return;
		}
		handleTerminalMessage(payload);
	};

	socket.onerror = () => {
		// onclose handles reconnection and user-facing state
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

function sendControl(action) {
	if (!terminalAvailable) return;
	sendSocketMessage({ type: "control", action: action });
}

function bindEvents() {
	if (newTabBtn) {
		newTabBtn.addEventListener("click", () => {
			void createTerminalWindow();
		});
	}

	if (ctrlCBtn) {
		ctrlCBtn.addEventListener("click", () => {
			sendControl("ctrl_c");
		});
	}

	if (clearBtn) {
		clearBtn.addEventListener("click", () => {
			sendControl("clear");
		});
	}

	if (restartBtn) {
		restartBtn.addEventListener("click", () => {
			sendControl("restart");
		});
	}

	if (installTmuxBtn) {
		installTmuxBtn.addEventListener("click", () => {
			if (!(terminalAvailable && tmuxInstallCommand)) return;
			if (!sendSocketMessage({ type: "input", data: `${tmuxInstallCommand}\n` })) {
				setStatus("Failed to queue install command.", "error");
				return;
			}
			setStatus(`Queued install command: ${tmuxInstallCommand}`, "ok");
			if (xterm) xterm.focus();
		});
	}

	if (copyInstallBtn) {
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
}

export async function initTerminal(container) {
	_container = container;
	shuttingDown = false;
	tmuxInstallPromptSeen = readTmuxInstallPromptSeen();
	tmuxInstallCommand = "";
	container.style.cssText = "display:flex;flex-direction:column;padding:0;overflow:hidden;min-height:0;";
	container.innerHTML = `
		<div class="terminal-page">
			<div class="terminal-toolbar">
				<div class="terminal-heading">
					<h2 class="text-lg font-medium text-[var(--text-strong)]">Terminal</h2>
					<div id="terminalMeta" class="terminal-meta"></div>
				</div>
				<div class="terminal-actions">
					<div id="terminalSize" class="terminal-size" title="Terminal size (columns \u00d7 rows)">\u2014\u00d7\u2014</div>
					<button id="terminalCtrlC" class="logs-btn" type="button" title="Send Ctrl+C">Ctrl+C</button>
					<button id="terminalClear" class="logs-btn" type="button" title="Send Ctrl+L">Clear</button>
					<button id="terminalRestart" class="logs-btn" type="button">Restart</button>
				</div>
			</div>
			<div class="terminal-tabs-bar">
				<div id="terminalTabs" class="terminal-tabs" aria-label="tmux windows"></div>
				<button id="terminalNewTab" class="logs-btn terminal-new-tab" type="button" title="Create tmux window">+ Tab</button>
			</div>
			<div class="terminal-output-wrap">
				<div id="terminalOutput" class="terminal-output" aria-label="Host terminal output"></div>
			</div>
			<div id="terminalStatus" class="terminal-status"></div>
			<div id="terminalHint" class="terminal-hint">Interactive host shell. Click inside terminal and type commands directly.</div>
			<div id="terminalHintActions" class="terminal-hint-actions" hidden>
				<code id="terminalInstallCommand" class="terminal-hint-code"></code>
				<button id="terminalInstallTmux" class="logs-btn terminal-hint-btn terminal-hint-btn-primary" type="button">Run install command</button>
				<button id="terminalCopyInstall" class="logs-btn terminal-hint-btn" type="button">Copy</button>
			</div>
		</div>
	`;

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
		setStatus(err.message || "Failed to initialize terminal", "error");
	}
}

export function teardownTerminal() {
	shuttingDown = true;
	clearReconnectTimer();
	clearResizeSettleTimers();
	closeTerminalSocket();
	clearInputQueue();
	clearWindowsRefreshTimer();
	disposeXterm();
	if (_container) {
		_container.innerHTML = "";
	}

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

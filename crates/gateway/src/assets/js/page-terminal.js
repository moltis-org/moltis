// Settings > Terminal (host shell via PTY + xterm.js over WebSocket)

var _container = null;
var resizeObserver = null;
var themeObserver = null;
var fitRaf = 0;

var reconnectTimer = null;
var socket = null;
var shuttingDown = false;

var inputFlushTimer = null;
var pendingInput = "";

var terminalEl = null;
var metaEl = null;
var statusEl = null;
var hintEl = null;
var hintActionsEl = null;
var installCommandEl = null;
var ctrlCBtn = null;
var clearBtn = null;
var restartBtn = null;
var installTmuxBtn = null;
var copyInstallBtn = null;

var xterm = null;
var fitAddon = null;
var xtermDataDisposable = null;
var TerminalCtor = null;
var FitAddonCtor = null;

var terminalAvailable = false;
var lastSentCols = 0;
var lastSentRows = 0;
var tmuxInstallCommand = "";
var tmuxInstallPromptSeen = false;

var RECONNECT_DELAY_MS = 800;
var INPUT_FLUSH_MS = 16;
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

function clearInputQueue() {
	if (inputFlushTimer) {
		clearTimeout(inputFlushTimer);
		inputFlushTimer = null;
	}
	pendingInput = "";
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
}

function setInstallActionsVisible(visible) {
	if (!hintActionsEl) return;
	hintActionsEl.hidden = !visible;
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

function sendSocketMessage(payload) {
	if (!(socket && socket.readyState === WebSocket.OPEN)) return false;
	try {
		socket.send(JSON.stringify(payload));
		return true;
	} catch {
		return false;
	}
}

function sendResizeIfChanged() {
	if (!xterm) return;
	if (!terminalAvailable) return;
	var cols = xterm.cols || 0;
	var rows = xterm.rows || 0;
	if (!(cols > 0 && rows > 0)) return;
	if (cols === lastSentCols && rows === lastSentRows) return;
	lastSentCols = cols;
	lastSentRows = rows;
	sendSocketMessage({ type: "resize", cols: cols, rows: rows });
}

function scheduleFit() {
	if (!fitAddon) return;
	clearScheduledFit();
	fitRaf = requestAnimationFrame(() => {
		fitRaf = 0;
		if (!fitAddon) return;
		try {
			fitAddon.fit();
			sendResizeIfChanged();
		} catch {
			// xterm may throw during transient detach or hidden layout states.
		}
	});
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
	fitAddon = new FitAddonCtor();
	xterm.loadAddon(fitAddon);
	xterm.open(terminalEl);
	xtermDataDisposable = xterm.onData((data) => {
		queueInput(data);
	});
	scheduleFit();

	terminalEl.addEventListener("click", () => {
		if (xterm) xterm.focus();
	});

	if (typeof ResizeObserver !== "undefined") {
		resizeObserver = new ResizeObserver(() => {
			scheduleFit();
		});
		resizeObserver.observe(terminalEl);
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
	if (xtermDataDisposable) {
		xtermDataDisposable.dispose();
		xtermDataDisposable = null;
	}
	if (xterm) {
		xterm.dispose();
		xterm = null;
	}
	fitAddon = null;
	lastSentCols = 0;
	lastSentRows = 0;
}

function isNearBottom() {
	if (!xterm) return false;
	var buffer = xterm.buffer.active;
	if (!buffer) return true;
	return buffer.baseY - buffer.viewportY <= 2;
}

function writeToXterm(text, scrollBottom) {
	if (!xterm) return;
	var content = typeof text === "string" ? text : "";
	if (!content) {
		if (scrollBottom) xterm.scrollToBottom();
		return;
	}
	xterm.write(content, () => {
		if (scrollBottom && xterm) xterm.scrollToBottom();
	});
}

function appendOutputChunk(text, forceBottom) {
	if (!xterm) return;
	if (typeof text !== "string" || text.length === 0) return;
	var atBottom = forceBottom || isNearBottom();
	writeToXterm(text, atBottom);
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
	var persistenceAvailable = !!payload.persistenceAvailable;
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
			var prompt = payload.promptSymbol || "$";
			var user = payload.user || "unknown";
			if (persistenceEnabled) {
				metaEl.textContent = `Host shell via PTY + tmux persistence - unsandboxed - user ${user} - prompt ${prompt}`;
			} else {
				metaEl.textContent = `Host shell via PTY (ephemeral) - unsandboxed - user ${user} - prompt ${prompt}`;
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

	if (terminalAvailable) {
		if (persistenceEnabled) {
			setStatus("Connected to host shell with persistent tmux session.", "ok");
		} else {
			setStatus("Connected to host shell (ephemeral session).", "ok");
		}
		flushInputQueue();
		sendResizeIfChanged();
		if (xterm) xterm.focus();
	} else {
		setStatus("Failed to open host shell.", "error");
	}
}

function handleTerminalMessage(payload) {
	if (!(payload && typeof payload === "object")) return;
	switch (payload.type) {
		case "ready":
			applyReadyPayload(payload);
			break;
		case "output":
			appendOutputChunk(payload.data || "", false);
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
	closeTerminalSocket();

	var proto = location.protocol === "https:" ? "wss:" : "ws:";
	socket = new WebSocket(`${proto}//${location.host}/api/terminal/ws`);
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
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	container.innerHTML = `
		<div class="terminal-page">
			<div class="terminal-toolbar">
				<div class="terminal-heading">
					<h2 class="text-lg font-medium text-[var(--text-strong)]">Terminal</h2>
					<div id="terminalMeta" class="terminal-meta"></div>
				</div>
				<div class="terminal-actions">
					<button id="terminalCtrlC" class="logs-btn" type="button" title="Send Ctrl+C">Ctrl+C</button>
					<button id="terminalClear" class="logs-btn" type="button" title="Send Ctrl+L">Clear</button>
					<button id="terminalRestart" class="logs-btn" type="button">Restart</button>
				</div>
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
	ctrlCBtn = container.querySelector("#terminalCtrlC");
	clearBtn = container.querySelector("#terminalClear");
	restartBtn = container.querySelector("#terminalRestart");
	installTmuxBtn = container.querySelector("#terminalInstallTmux");
	copyInstallBtn = container.querySelector("#terminalCopyInstall");

	setStatus("Initializing terminal...");
	setControlsEnabled(false);
	bindEvents();

	try {
		await initXterm();
		connectTerminalSocket();
	} catch (err) {
		setStatus(err.message || "Failed to initialize terminal", "error");
	}
}

export function teardownTerminal() {
	shuttingDown = true;
	clearReconnectTimer();
	closeTerminalSocket();
	clearInputQueue();
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
	ctrlCBtn = null;
	clearBtn = null;
	restartBtn = null;
	installTmuxBtn = null;
	copyInstallBtn = null;
	terminalAvailable = false;
	tmuxInstallCommand = "";
}

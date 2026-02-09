// ── Helpers ──────────────────────────────────────────────────
import * as S from "./state.js";

export function nextId() {
	S.setReqId(S.reqId + 1);
	return `ui-${S.reqId}`;
}

export function esc(s) {
	return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

export function renderMarkdown(raw) {
	var s = esc(raw);
	s = s.replace(/```(\w*)\n([\s\S]*?)```/g, (_, _lang, code) => `<pre><code>${code}</code></pre>`);
	s = s.replace(/`([^`]+)`/g, "<code>$1</code>");
	s = s.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
	return s;
}

export function sendRpc(method, params) {
	return new Promise((resolve) => {
		if (!S.ws || S.ws.readyState !== WebSocket.OPEN) {
			resolve({ ok: false, error: { message: "WebSocket not connected" } });
			return;
		}
		var id = nextId();
		S.pending[id] = resolve;
		S.ws.send(JSON.stringify({ type: "req", id: id, method: method, params: params }));
	});
}

export function formatTokens(n) {
	if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
	if (n >= 1000) return `${(n / 1000).toFixed(1)}K`;
	return String(n);
}

export function formatBytes(b) {
	if (b >= 1024) return `${(b / 1024).toFixed(1)} KB`;
	return `${b} B`;
}

function classifyJsonErrorObj(errObj) {
	var resetsAt = errObj.resets_at ? errObj.resets_at * 1000 : null;
	if (errObj.type === "usage_limit_reached" || (errObj.message && errObj.message.indexOf("usage limit") !== -1)) {
		return {
			icon: "",
			title: "Usage limit reached",
			detail: `Your ${errObj.plan_type || "current"} plan limit has been reached.`,
			resetsAt: resetsAt,
		};
	}
	if (errObj.type === "rate_limit_exceeded" || (errObj.message && errObj.message.indexOf("rate limit") !== -1)) {
		return {
			icon: "\u26A0\uFE0F",
			title: "Rate limited",
			detail: errObj.message || "Too many requests. Please wait a moment.",
			resetsAt: resetsAt,
		};
	}
	if (errObj.message) {
		return { icon: "\u26A0\uFE0F", title: "Error", detail: errObj.message, resetsAt: null };
	}
	return null;
}

function parseJsonError(message) {
	var jsonMatch = message.match(/\{[\s\S]*\}$/);
	if (!jsonMatch) return null;
	try {
		var err = JSON.parse(jsonMatch[0]);
		return classifyJsonErrorObj(err.error || err);
	} catch (_e) {
		/* fall through */
	}
	return null;
}

function parseHttpStatusError(message) {
	var statusMatch = message.match(/HTTP (\d{3})/);
	var code = statusMatch ? parseInt(statusMatch[1], 10) : 0;
	if (code === 401 || code === 403)
		return {
			icon: "\uD83D\uDD12",
			title: "Authentication error",
			detail: "Your session may have expired.",
			resetsAt: null,
		};
	if (code === 429)
		return {
			icon: "",
			title: "Rate limited",
			detail: "Too many requests.",
			resetsAt: null,
		};
	if (code >= 500)
		return {
			icon: "\uD83D\uDEA8",
			title: "Server error",
			detail: "The upstream provider returned an error.",
			resetsAt: null,
		};
	return null;
}

export function parseErrorMessage(message) {
	return (
		parseJsonError(message) ||
		parseHttpStatusError(message) || {
			icon: "\u26A0\uFE0F",
			title: "Error",
			detail: message,
			resetsAt: null,
		}
	);
}

export function updateCountdown(el, resetsAtMs) {
	var now = Date.now();
	var diff = resetsAtMs - now;
	if (diff <= 0) {
		el.textContent = "Limit should be reset now \u2014 try again!";
		el.className = "error-countdown reset-ready";
		return true;
	}
	var hours = Math.floor(diff / 3600000);
	var mins = Math.floor((diff % 3600000) / 60000);
	var parts = [];
	if (hours > 0) parts.push(`${hours}h`);
	parts.push(`${mins}m`);
	el.textContent = `Resets in ${parts.join(" ")}`;
	return false;
}

/** Build a short summary string for a tool call card. */
export function toolCallSummary(name, args, executionMode) {
	if (!args) return name || "tool";
	switch (name) {
		case "exec":
			return args.command || "exec";
		case "web_fetch":
			return `web_fetch ${args.url || ""}`.trim();
		case "web_search":
			return `web_search "${args.query || ""}"`;
		case "browser": {
			var action = args.action || "browser";
			var mode = executionMode ? ` (${executionMode})` : "";
			var url = args.url ? ` ${args.url}` : "";
			return `browser ${action}${mode}${url}`.trim();
		}
		default:
			return name || "tool";
	}
}

/**
 * Render a screenshot thumbnail with lightbox and download into `container`.
 * @param {HTMLElement} container - parent element to append into
 * @param {string} imgSrc - image URL (data URI or HTTP URL)
 * @param {number} [scale=1] - HiDPI scale factor
 */
export function renderScreenshot(container, imgSrc, scale) {
	if (!scale) scale = 1;
	var imgContainer = document.createElement("div");
	imgContainer.className = "screenshot-container";
	var img = document.createElement("img");
	img.src = imgSrc;
	img.className = "screenshot-thumbnail";
	img.alt = "Browser screenshot";
	img.title = "Click to view full size";

	img.onload = () => {
		if (scale > 1) {
			var logicalWidth = img.naturalWidth / scale;
			var logicalHeight = img.naturalHeight / scale;
			img.style.aspectRatio = `${logicalWidth} / ${logicalHeight}`;
		}
	};

	var downloadScreenshot = (e) => {
		e.stopPropagation();
		var link = document.createElement("a");
		link.href = imgSrc;
		link.download = `screenshot-${Date.now()}.png`;
		link.click();
	};

	img.onclick = () => {
		var overlay = document.createElement("div");
		overlay.className = "screenshot-lightbox";

		var lightboxContent = document.createElement("div");
		lightboxContent.className = "screenshot-lightbox-content";

		var header = document.createElement("div");
		header.className = "screenshot-lightbox-header";
		header.onclick = (e) => e.stopPropagation();

		var closeBtn = document.createElement("button");
		closeBtn.className = "screenshot-lightbox-close";
		closeBtn.textContent = "\u2715";
		closeBtn.title = "Close (Esc)";
		closeBtn.onclick = () => overlay.remove();

		var downloadBtn = document.createElement("button");
		downloadBtn.className = "screenshot-download-btn";
		downloadBtn.textContent = "\u2B07 Download";
		downloadBtn.onclick = downloadScreenshot;

		header.appendChild(closeBtn);
		header.appendChild(downloadBtn);

		var scrollContainer = document.createElement("div");
		scrollContainer.className = "screenshot-lightbox-scroll";
		scrollContainer.onclick = (e) => e.stopPropagation();

		var fullImg = document.createElement("img");
		fullImg.src = img.src;
		fullImg.className = "screenshot-lightbox-img";

		fullImg.onload = () => {
			var logicalWidth = fullImg.naturalWidth / scale;
			var logicalHeight = fullImg.naturalHeight / scale;
			var viewportWidth = window.innerWidth - 80;
			var displayWidth = Math.min(logicalWidth, viewportWidth);
			fullImg.style.width = `${displayWidth}px`;
			var displayHeight = (displayWidth / logicalWidth) * logicalHeight;
			fullImg.style.height = `${displayHeight}px`;
		};

		scrollContainer.appendChild(fullImg);
		lightboxContent.appendChild(header);
		lightboxContent.appendChild(scrollContainer);
		overlay.appendChild(lightboxContent);

		overlay.onclick = () => overlay.remove();
		var closeOnEscape = (e) => {
			if (e.key === "Escape") {
				overlay.remove();
				document.removeEventListener("keydown", closeOnEscape);
			}
		};
		document.addEventListener("keydown", closeOnEscape);
		document.body.appendChild(overlay);
	};

	var thumbDownloadBtn = document.createElement("button");
	thumbDownloadBtn.className = "screenshot-download-btn-small";
	thumbDownloadBtn.textContent = "\u2B07";
	thumbDownloadBtn.title = "Download screenshot";
	thumbDownloadBtn.onclick = downloadScreenshot;

	imgContainer.appendChild(img);
	imgContainer.appendChild(thumbDownloadBtn);
	container.appendChild(imgContainer);
}

// ── Waveform audio player ───────────────────────────────────

var WAVEFORM_BAR_COUNT = 48;
var WAVEFORM_MIN_HEIGHT = 0.08;

async function extractWaveform(audioSrc, barCount) {
	var ctx = new (window.AudioContext || window.webkitAudioContext)();
	try {
		var response = await fetch(audioSrc);
		var buf = await response.arrayBuffer();
		var audioBuffer = await ctx.decodeAudioData(buf);
		var data = audioBuffer.getChannelData(0);
		if (data.length < barCount) {
			return new Array(barCount).fill(WAVEFORM_MIN_HEIGHT);
		}
		var step = Math.floor(data.length / barCount);
		var peaks = [];
		for (var i = 0; i < barCount; i++) {
			var start = i * step;
			var end = Math.min(start + step, data.length);
			var max = 0;
			for (var j = start; j < end; j++) {
				var abs = Math.abs(data[j]);
				if (abs > max) max = abs;
			}
			peaks.push(max);
		}
		var maxPeak = 0;
		for (var pk of peaks) {
			if (pk > maxPeak) maxPeak = pk;
		}
		maxPeak = maxPeak || 1;
		return peaks.map((v) => Math.max(WAVEFORM_MIN_HEIGHT, v / maxPeak));
	} finally {
		ctx.close();
	}
}

function formatAudioDuration(seconds) {
	var m = Math.floor(seconds / 60);
	var s = Math.floor(seconds % 60);
	return `${m}:${s < 10 ? "0" : ""}${s}`;
}

function svgEl(tag, attrs) {
	var el = document.createElementNS("http://www.w3.org/2000/svg", tag);
	if (attrs) {
		for (var key in attrs) el.setAttribute(key, attrs[key]);
	}
	return el;
}

function createPlaySvg() {
	var svg = svgEl("svg", { viewBox: "0 0 24 24", fill: "currentColor" });
	svg.appendChild(svgEl("path", { d: "M8 5v14l11-7z" }));
	return svg;
}

function createPauseSvg() {
	var svg = svgEl("svg", { viewBox: "0 0 24 24", fill: "currentColor" });
	svg.appendChild(svgEl("rect", { x: "6", y: "4", width: "4", height: "16", rx: "1" }));
	svg.appendChild(svgEl("rect", { x: "14", y: "4", width: "4", height: "16", rx: "1" }));
	return svg;
}

// ── Audio autoplay unlock ────────────────────────────────────
// Browsers block audio.play() without a recent user gesture. We "unlock"
// playback by creating a shared AudioContext on the first user action
// (sending a message / clicking record). Once resumed, all subsequent
// audio.play() calls on the page are allowed.
var _audioCtx = null;

/**
 * Call from a user-gesture handler (click / keydown) to unlock audio
 * playback for the current page session. Idempotent — safe to call
 * multiple times.
 */
export function warmAudioPlayback() {
	if (!_audioCtx) {
		_audioCtx = new (window.AudioContext || window.webkitAudioContext)();
		console.debug("[audio] created AudioContext, state:", _audioCtx.state);
	}
	if (_audioCtx.state === "suspended") {
		console.debug("[audio] resuming suspended AudioContext");
		_audioCtx.resume().catch((e) => console.warn("[audio] resume failed:", e));
	}
}

/**
 * Render a waveform audio player (Telegram-style bars) into `container`.
 * @param {HTMLElement} container - parent element to append into
 * @param {string} audioSrc - audio URL (HTTP or data URI)
 * @param {boolean} [autoplay=false] - start playback immediately
 */
export function renderAudioPlayer(container, audioSrc, autoplay) {
	var wrap = document.createElement("div");
	wrap.className = "waveform-player mt-2";

	var audio = document.createElement("audio");
	audio.preload = "auto";
	audio.src = audioSrc;

	var playBtn = document.createElement("button");
	playBtn.className = "waveform-play-btn";
	playBtn.type = "button";
	playBtn.appendChild(createPlaySvg());

	var barsWrap = document.createElement("div");
	barsWrap.className = "waveform-bars";

	var durEl = document.createElement("span");
	durEl.className = "waveform-duration";
	durEl.textContent = "";

	wrap.appendChild(playBtn);
	wrap.appendChild(barsWrap);
	wrap.appendChild(durEl);
	container.appendChild(wrap);

	var bars = [];
	for (var i = 0; i < WAVEFORM_BAR_COUNT; i++) {
		var bar = document.createElement("div");
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
			for (var b of bars) {
				b.style.height = `${20 + Math.random() * 60}%`;
			}
		});

	audio.addEventListener("loadedmetadata", () => {
		durEl.textContent = formatAudioDuration(audio.duration);
	});

	playBtn.onclick = () => {
		if (audio.paused) {
			audio.play().catch(() => undefined);
		} else {
			audio.pause();
		}
	};

	var rafId = 0;
	var prevPlayed = -1;

	function tick() {
		if (!audio.duration) {
			rafId = requestAnimationFrame(tick);
			return;
		}
		var progress = audio.currentTime / audio.duration;
		var playedCount = Math.floor(progress * WAVEFORM_BAR_COUNT);
		if (playedCount !== prevPlayed) {
			var lo = Math.min(playedCount, prevPlayed < 0 ? 0 : prevPlayed);
			var hi = Math.max(playedCount, prevPlayed < 0 ? WAVEFORM_BAR_COUNT : prevPlayed);
			for (var idx = lo; idx < hi; idx++) {
				bars[idx].classList.toggle("played", idx < playedCount);
			}
			prevPlayed = playedCount;
		}
		durEl.textContent = formatAudioDuration(audio.currentTime);
		rafId = requestAnimationFrame(tick);
	}

	audio.addEventListener("play", () => {
		playBtn.replaceChildren(createPauseSvg());
		prevPlayed = -1;
		rafId = requestAnimationFrame(tick);
	});

	audio.addEventListener("pause", () => {
		playBtn.replaceChildren(createPlaySvg());
		cancelAnimationFrame(rafId);
	});

	audio.addEventListener("ended", () => {
		playBtn.replaceChildren(createPlaySvg());
		cancelAnimationFrame(rafId);
		for (var b of bars) b.classList.remove("played");
		prevPlayed = -1;
		if (audio.duration) durEl.textContent = formatAudioDuration(audio.duration);
	});

	barsWrap.onclick = (e) => {
		if (!audio.duration) return;
		var rect = barsWrap.getBoundingClientRect();
		var fraction = (e.clientX - rect.left) / rect.width;
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
		var doPlay = () => {
			console.debug("[audio] attempting play(), readyState:", audio.readyState, "paused:", audio.paused);
			audio
				.play()
				.then(() => console.debug("[audio] play() succeeded"))
				.catch((e) => console.warn("[audio] play() rejected:", e.name, e.message));
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
 * Render clickable map link buttons into `container`.
 * @param {HTMLElement} container - parent element to append into
 * @param {object} links - { google_maps, apple_maps, openstreetmap }
 * @param {string} [label] - optional location label
 */
export function renderMapLinks(container, links, label) {
	var row = document.createElement("div");
	row.className = "flex flex-wrap gap-2 mt-2";

	var services = [
		{ key: "google_maps", name: "Google Maps", icon: "\uD83C\uDF0D" },
		{ key: "apple_maps", name: "Apple Maps", icon: "\uD83D\uDDFA\uFE0F" },
		{ key: "openstreetmap", name: "OpenStreetMap", icon: "\uD83D\uDCCD" },
	];

	for (var svc of services) {
		var url = links[svc.key];
		if (!url) continue;
		var btn = document.createElement("a");
		btn.href = url;
		btn.target = "_blank";
		btn.rel = "noopener noreferrer";
		btn.className = "provider-btn provider-btn-secondary text-xs";
		btn.textContent = `${svc.icon} ${svc.name}`;
		if (label) btn.title = `Open "${label}" in ${svc.name}`;
		row.appendChild(btn);
	}

	container.appendChild(row);
}

export function createEl(tag, attrs, children) {
	var el = document.createElement(tag);
	if (attrs) {
		Object.keys(attrs).forEach((k) => {
			if (k === "className") el.className = attrs[k];
			else if (k === "textContent") el.textContent = attrs[k];
			else if (k === "style") el.style.cssText = attrs[k];
			else el.setAttribute(k, attrs[k]);
		});
	}
	if (children) {
		children.forEach((c) => {
			if (c) el.appendChild(c);
		});
	}
	return el;
}

// ── Helpers ──────────────────────────────────────────────────
import { hasTranslation, t } from "./i18n.js";
import * as S from "./state.js";

function translatedOrFallback(key, opts, fallback) {
	if (!key) return fallback;
	if (!hasTranslation(key, opts)) return fallback;
	var translated = t(key, opts);
	if (translated) return translated;
	return fallback;
}

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
		var id = nextId();
		S.pending[id] = resolve;
		S.ws.send(JSON.stringify({ type: "req", id: id, method: method, params: params }));
	});
}

export function localizedRpcErrorMessage(error) {
	if (!error) return t("errors:generic.title");
	if (error.code) {
		var key = `errors:codes.${error.code}`;
		var translated = t(key);
		if (translated && translated !== key) {
			return translated;
		}
	}
	return error.message || t("errors:generic.title");
}

export function localizeRpcError(error) {
	if (!error) return error;
	var message = localizedRpcErrorMessage(error);
	if (error.message === message) return error;
	return Object.assign({}, error, { message: message });
}

export function localizeStructuredError(error) {
	if (!error) return error;
	var title = translatedOrFallback(error.title_key, error.title_params, error.title || t("errors:generic.title"));
	var detail = translatedOrFallback(error.detail_key, error.detail_params, error.detail || "");
	if (title === error.title && detail === (error.detail || "")) return error;
	return Object.assign({}, error, { title: title, detail: detail });
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

function getResetsAtMs(errObj) {
	return errObj.resetsAt || (errObj.resets_at ? errObj.resets_at * 1000 : null);
}

function classifyStructuredError(errObj, resetsAt) {
	if (!(errObj.title_key || errObj.detail_key)) return null;
	return localizeStructuredError({
		icon: errObj.icon || "\u26A0\uFE0F",
		title: errObj.title || t("errors:generic.title"),
		detail: errObj.detail || errObj.message || "",
		provider: errObj.provider,
		resetsAt: resetsAt,
		title_key: errObj.title_key,
		detail_key: errObj.detail_key,
		title_params: errObj.title_params,
		detail_params: errObj.detail_params,
	});
}

function classifyUsageLimitError(errObj, resetsAt) {
	if (!(errObj.type === "usage_limit_reached" || (errObj.message && errObj.message.indexOf("usage limit") !== -1))) {
		return null;
	}
	return {
		icon: "",
		title: t("errors:usageLimitReached.title"),
		detail: t("errors:usageLimitReached.detail", { planType: errObj.plan_type || "current" }),
		resetsAt: resetsAt,
	};
}

function classifyRateLimitError(errObj, resetsAt) {
	if (!(errObj.type === "rate_limit_exceeded" || (errObj.message && errObj.message.indexOf("rate limit") !== -1))) {
		return null;
	}
	return {
		icon: "\u26A0\uFE0F",
		title: t("errors:rateLimited.title"),
		detail: errObj.message || t("errors:rateLimited.detail"),
		resetsAt: resetsAt,
	};
}

function classifyJsonErrorObj(errObj) {
	var resetsAt = getResetsAtMs(errObj);
	return (
		classifyStructuredError(errObj, resetsAt) ||
		classifyUsageLimitError(errObj, resetsAt) ||
		classifyRateLimitError(errObj, resetsAt) ||
		(errObj.message
			? { icon: "\u26A0\uFE0F", title: t("errors:generic.title"), detail: errObj.message, resetsAt: null }
			: null)
	);
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

export function parseErrorMessage(message) {
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

export function updateCountdown(el, resetsAtMs) {
	var now = Date.now();
	var diff = resetsAtMs - now;
	if (diff <= 0) {
		el.textContent = t("errors:countdown.resetReady");
		el.className = "error-countdown reset-ready";
		return true;
	}
	var hours = Math.floor(diff / 3600000);
	var mins = Math.floor((diff % 3600000) / 60000);
	var parts = [];
	if (hours > 0) parts.push(`${hours}h`);
	parts.push(`${mins}m`);
	el.textContent = t("errors:countdown.resetsIn", { time: parts.join(" ") });
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

export function formatAudioDuration(seconds) {
	if (!Number.isFinite(seconds) || seconds < 0) return "00:00";
	var totalSeconds = Math.floor(seconds);
	var m = Math.floor(totalSeconds / 60);
	var s = totalSeconds % 60;
	return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

function createPlaySvg() {
	var el = document.createElement("span");
	el.className = "icon icon-play";
	return el;
}

function createPauseSvg() {
	var el = document.createElement("span");
	el.className = "icon icon-pause";
	return el;
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
// Track the most recently played audio element so spacebar can toggle it.
var _activeAudio = null;

function isEditableTarget(el) {
	if (!el) return false;
	var tag = el.tagName;
	if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
	return el.isContentEditable;
}

document.addEventListener("keydown", (e) => {
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
	durEl.textContent = "00:00";

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

	function syncDurationLabel() {
		if (!Number.isFinite(audio.duration) || audio.duration < 0) return;
		durEl.textContent = formatAudioDuration(audio.duration);
	}

	audio.addEventListener("loadedmetadata", syncDurationLabel);
	audio.addEventListener("durationchange", syncDurationLabel);
	audio.addEventListener("canplay", syncDurationLabel);

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
		if (!Number.isFinite(audio.duration) || audio.duration <= 0) {
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
		for (var b of bars) b.classList.remove("played");
		prevPlayed = -1;
		if (Number.isFinite(audio.duration) && audio.duration >= 0) {
			durEl.textContent = formatAudioDuration(audio.duration);
		}
	});

	barsWrap.onclick = (e) => {
		if (!Number.isFinite(audio.duration) || audio.duration <= 0) return;
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
var MAP_SERVICE_ICONS = {
	google_maps: "map-google-maps.svg",
	apple_maps: "map-apple-maps.svg",
	openstreetmap: "map-openstreetmap.svg",
};

function createMapServiceIcon(serviceKey) {
	var iconFile = MAP_SERVICE_ICONS[serviceKey];
	if (!iconFile) return null;

	var icon = document.createElement("img");
	icon.src = new URL(`../icons/${iconFile}`, import.meta.url).toString();
	icon.alt = "";
	icon.width = 14;
	icon.height = 14;
	icon.className = "map-service-icon";
	icon.setAttribute("aria-hidden", "true");
	icon.setAttribute("decoding", "async");
	return icon;
}

export function renderMapLinks(container, links, label) {
	var row = document.createElement("div");
	row.className = "flex flex-wrap gap-2 mt-2";

	var services = [
		{ key: "google_maps", name: "Google Maps" },
		{ key: "apple_maps", name: "Apple Maps" },
		{ key: "openstreetmap", name: "OpenStreetMap" },
	];

	for (var svc of services) {
		var url = links[svc.key];
		if (!url) continue;
		var btn = document.createElement("a");
		btn.href = url;
		btn.target = "_blank";
		btn.rel = "noopener noreferrer";
		btn.className = "provider-btn provider-btn-secondary text-xs inline-flex items-center gap-1.5";
		var icon = createMapServiceIcon(svc.key);
		if (icon) btn.appendChild(icon);
		var labelEl = document.createElement("span");
		labelEl.textContent = svc.name;
		btn.appendChild(labelEl);
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

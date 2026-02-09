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

/**
 * Render an `<audio>` player into `container`.
 * @param {HTMLElement} container - parent element to append into
 * @param {string} audioSrc - audio URL (HTTP or data URI)
 * @param {boolean} [autoplay=false] - start playback immediately
 */
export function renderAudioPlayer(container, audioSrc, autoplay) {
	var wrap = document.createElement("div");
	wrap.className = "mt-2";
	var audio = document.createElement("audio");
	audio.controls = true;
	audio.preload = "none";
	audio.src = audioSrc;
	audio.className = "w-full max-w-md";
	wrap.appendChild(audio);
	container.appendChild(wrap);
	if (autoplay) audio.play().catch(() => undefined);
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

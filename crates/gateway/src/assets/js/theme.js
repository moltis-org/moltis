// ── Theme ────────────────────────────────────────────────────
import { $ } from "./state.js";

function getSystemTheme() {
	return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function applyTheme(mode) {
	var resolved = mode === "system" ? getSystemTheme() : mode;
	document.documentElement.setAttribute("data-theme", resolved);
	document.documentElement.style.colorScheme = resolved;
	updateThemeButtons(mode);
}

function updateThemeButtons(activeMode) {
	var buttons = document.querySelectorAll(".theme-btn");
	buttons.forEach((btn) => {
		btn.classList.toggle("active", btn.getAttribute("data-theme-val") === activeMode);
	});
}

export function initTheme() {
	var saved = localStorage.getItem("moltis-theme") || "system";
	applyTheme(saved);
	var mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
	var onSystemThemeChange = () => {
		var current = localStorage.getItem("moltis-theme") || "system";
		if (current === "system") applyTheme("system");
	};
	if (typeof mediaQuery.addEventListener === "function") {
		mediaQuery.addEventListener("change", onSystemThemeChange);
	} else if (typeof mediaQuery.addListener === "function") {
		// Legacy Safari fallback.
		mediaQuery.addListener(onSystemThemeChange);
	}

	var themeToggle = $("themeToggle");
	if (!themeToggle) return;
	themeToggle.addEventListener("click", (e) => {
		var btn = e.target.closest(".theme-btn");
		if (!btn) return;
		var mode = btn.getAttribute("data-theme-val");
		localStorage.setItem("moltis-theme", mode);
		applyTheme(mode);
	});
}

// ── Markdown body styles (for skill detail panel) ───────────
export function injectMarkdownStyles() {
	var ms = document.createElement("style");
	ms.textContent =
		".skill-body-md h1{font-size:1.25rem;font-weight:700;margin:16px 0 8px;padding-bottom:4px;border-bottom:1px solid var(--border)}" +
		".skill-body-md h2{font-size:1.1rem;font-weight:600;margin:14px 0 6px;padding-bottom:3px;border-bottom:1px solid var(--border)}" +
		".skill-body-md h3{font-size:.95rem;font-weight:600;margin:12px 0 4px}" +
		".skill-body-md h4{font-size:.88rem;font-weight:600;margin:10px 0 4px}" +
		".skill-body-md h5,.skill-body-md h6{font-size:.82rem;font-weight:600;margin:8px 0 4px}" +
		".skill-body-md p{margin:6px 0;line-height:1.6}" +
		".skill-body-md ul,.skill-body-md ol{margin:6px 0 6px 20px;padding:0}" +
		".skill-body-md ul{list-style:disc}" +
		".skill-body-md ol{list-style:decimal}" +
		".skill-body-md li{margin:2px 0;line-height:1.5}" +
		".skill-body-md li>ul,.skill-body-md li>ol{margin:2px 0 2px 16px}" +
		".skill-body-md code{background:var(--surface);padding:1px 5px;border-radius:4px;font-size:.82em;font-family:var(--font-mono)}" +
		".skill-body-md pre{background:var(--surface);border:1px solid var(--border);border-radius:var(--radius-sm);padding:10px 12px;overflow-x:auto;margin:8px 0;line-height:1.45}" +
		".skill-body-md pre code{background:none;padding:0;font-size:.78rem}" +
		".skill-body-md blockquote{border-left:3px solid var(--border);margin:8px 0;padding:4px 12px;color:var(--muted)}" +
		".skill-body-md a{color:var(--accent);text-decoration:underline}" +
		".skill-body-md a:hover{opacity:.8}" +
		".skill-body-md hr{border:none;border-top:1px solid var(--border);margin:12px 0}" +
		".skill-body-md table{border-collapse:collapse;width:100%;margin:8px 0;font-size:.8rem}" +
		".skill-body-md th,.skill-body-md td{border:1px solid var(--border);padding:5px 8px;text-align:left}" +
		".skill-body-md th{background:var(--surface);font-weight:600}" +
		".skill-body-md strong{font-weight:600}" +
		".skill-body-md em{font-style:italic}" +
		".skill-body-md img{max-width:100%;border-radius:var(--radius-sm)}" +
		".skill-body-md input[type=checkbox]{margin-right:4px}";
	document.head.appendChild(ms);
}

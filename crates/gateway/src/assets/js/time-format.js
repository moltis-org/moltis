// ── Auto-hydration for <time> elements using luxon ──────────

import { DateTime } from "luxon";

var SEVEN_DAYS_MS = 7 * 24 * 60 * 60 * 1000;

function parseTime(el) {
	var epochMs = el.getAttribute("data-epoch-ms");
	if (epochMs) return DateTime.fromMillis(Number(epochMs));

	var val = el.getAttribute("datetime");
	if (!val) return null;

	return DateTime.fromISO(val, { zone: "utc" });
}

function formatTime(dt, format) {
	if (format === "year-month") return dt.toFormat("LLL yyyy");
	var now = DateTime.now();
	var diff = now.toMillis() - dt.toMillis();
	if (diff >= 0 && diff < 30000) return "just now";
	if (diff >= 0 && diff < SEVEN_DAYS_MS) return dt.toRelative();
	return dt.toLocaleString(DateTime.DATETIME_MED);
}

function hydrateTimeElements() {
	var els = document.querySelectorAll("time[datetime]:not([data-hydrated]), time[data-epoch-ms]:not([data-hydrated])");
	for (var el of els) {
		var dt = parseTime(el);
		if (!dt?.isValid) continue;
		var fmt = el.getAttribute("data-format") || null;
		el.textContent = formatTime(dt, fmt);
		el.title = dt.toLocaleString(DateTime.DATETIME_FULL);
		el.setAttribute("data-hydrated", "1");
	}
}

// Re-hydrate when Preact re-renders (new <time> elements appear)
var observer = new MutationObserver(hydrateTimeElements);
observer.observe(document.body, { childList: true, subtree: true });

// Initial pass
hydrateTimeElements();

// Periodic refresh for relative times
setInterval(() => {
	// Clear hydrated marks so they get re-formatted
	for (var el of document.querySelectorAll("time[data-hydrated]")) {
		el.removeAttribute("data-hydrated");
	}
	hydrateTimeElements();
}, 60000);

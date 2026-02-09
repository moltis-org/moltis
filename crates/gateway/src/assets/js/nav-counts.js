// ── Nav sidebar count badges ────────────────────────────────
//
// Renders small count pills next to sidebar nav items.
// Populated from gon.counts at page load and updated live
// by individual page modules after data refreshes.

import * as gon from "./gon.js";

var ids = {
	projects: "navCountProjects",
	providers: "navCountProviders",
	channels: "navCountChannels",
	skills: "navCountSkills",
	mcp: "navCountMcp",
	crons: "navCountCrons",
	hooks: "navCountHooks",
	images: "navCountImages",
};

/** Update a single nav badge. Pass 0 to hide it. */
export function updateNavCount(key, n) {
	var id = ids[key];
	if (!id) return;
	var el = document.getElementById(id);
	if (!el) return;
	if (n > 0) {
		el.textContent = String(n);
		el.classList.add("visible");
	} else {
		el.textContent = "";
		el.classList.remove("visible");
	}
}

/** Apply all counts from a counts object. */
export function updateNavCounts(counts) {
	if (!counts) return;
	for (var key of Object.keys(ids)) {
		updateNavCount(key, counts[key] || 0);
	}
}

// Apply server-injected counts synchronously at module load.
updateNavCounts(gon.get("counts"));
gon.onChange("counts", updateNavCounts);

// Images count is loaded asynchronously because listing Docker images
// can be slow (or hang if Docker is not running). The server excludes it
// from gon counts to avoid blocking every page load.
fetch("/api/images/cached")
	.then((r) => (r.ok ? r.json() : null))
	.then((data) => {
		if (data?.images) updateNavCount("images", data.images.length);
	})
	.catch(() => {
		/* best-effort, badge stays hidden */
	});

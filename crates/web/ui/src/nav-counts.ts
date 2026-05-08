// ── Nav sidebar count badges ────────────────────────────────
//
// Renders small count pills next to sidebar nav items.
// Populated from gon.counts at page load and updated live
// by individual page modules after data refreshes.

import * as gon from "./gon";

const ids: Record<string, string> = {
	skills: "navCountSkills",
	crons: "navCountCrons",
};

/** Update a single nav badge. Pass 0 to hide it. */
export function updateNavCount(key: string, n: number): void {
	const id = ids[key];
	if (!id) return;
	const el = document.getElementById(id);
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
export function updateNavCounts(counts: Record<string, number> | null): void {
	if (!counts) return;
	for (const key of Object.keys(ids)) {
		updateNavCount(key, counts[key] || 0);
	}
}

// Apply server-injected counts synchronously at module load.
updateNavCounts(gon.get("counts") as Record<string, number> | null);
gon.onChange("counts", updateNavCounts as (value: unknown) => void);

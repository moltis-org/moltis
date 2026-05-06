// ── Project store (signal-based) ─────────────────────────────
//
// Single source of truth for project data and sidebar filter state.

import { signal } from "@preact/signals";
import { sendRpc } from "../helpers";
import type { ProjectInfo, RpcResponse } from "../types";

// ── Signals ──────────────────────────────────────────────────
export const projects = signal<ProjectInfo[]>([]);
export const activeProjectId = signal<string>(localStorage.getItem("moltis-project") || "");
export const projectFilterId = signal<string>(localStorage.getItem("moltis-project-filter") || "");

// ── Methods ──────────────────────────────────────────────────

/** Replace the full project list (e.g. after fetch or bootstrap). */
export function setAll(arr: ProjectInfo[]): void {
	projects.value = arr || [];
}

/** Fetch projects from the server via RPC. */
export function fetch(): Promise<void> {
	return sendRpc("projects.list", {}).then((r) => {
		const res = r as RpcResponse<ProjectInfo[]>;
		if (!res?.ok) return;
		setAll(res.payload || []);
	});
}

/** Set the active project id (bound to the active session's project). */
export function setActiveProjectId(id: string): void {
	activeProjectId.value = id || "";
}

/** Set the sidebar filter project id. Persists to localStorage. */
export function setFilterId(id: string): void {
	projectFilterId.value = id || "";
	if (id) {
		localStorage.setItem("moltis-project-filter", id);
	} else {
		localStorage.removeItem("moltis-project-filter");
	}
}

/** Look up a project by id. */
export function getById(id: string): ProjectInfo | null {
	return projects.value.find((p) => p.id === id) || null;
}

export const projectStore = {
	projects,
	activeProjectId,
	projectFilterId,
	setAll,
	fetch,
	setActiveProjectId,
	setFilterId,
	getById,
};

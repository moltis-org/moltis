// ── Skill source enum ────────────────────────────────────────
// Mirrors moltis_skills::types::SkillSource on the Rust side.

export enum SkillSource {
	Project = "project",
	Personal = "personal",
	Plugin = "plugin",
	Registry = "registry",
	Bundled = "bundled",
}

/** Sources that are stored as local files (can be deleted, not just disabled). */
export function isDiscoveredSource(source: string | undefined): boolean {
	return source === SkillSource.Personal || source === SkillSource.Project;
}

/** Whether a source string looks like a repo path (contains `/`). */
export function isRepoSource(source: string | undefined): boolean {
	return !!source?.includes("/");
}

// ── Skills page (Preact + Signals) ───────────────────────────
// Note: body_html is server-rendered trusted content from SKILL.md
// processed by pulldown-cmark on the Rust gateway side.

import { computed, signal, useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef } from "preact/hooks";
import { TabBar } from "../components/forms/Tabs";
import { onEvent } from "../events";
import { sendRpc } from "../helpers";
import { t } from "../i18n";
import { updateNavCount } from "../nav-counts";
import { registerPage } from "../router";
import { routes } from "../routes";
import * as S from "../state";
import {
	type BundledCategory,
	CATEGORY_META,
	categoryLabel,
	isDiscoveredSource,
	isRepoSource,
	SkillSource,
} from "../types/skill-source";
import { ConfirmDialog, requestConfirm } from "../ui";

// ── Types ────────────────────────────────────────────────────

interface SkillSummary {
	name: string;
	description?: string;
	category?: string;
	source?: string;
	enabled?: boolean;
	protected?: boolean;
	display_name?: string;
	quarantined?: boolean;
	trusted?: boolean;
	drifted?: boolean;
	eligible?: boolean;
}
interface SkillDetail extends SkillSummary {
	body?: string;
	body_html?: string;
	author?: string;
	version?: string;
	homepage?: string;
	source_url?: string;
	commit_sha?: string;
	commit_url?: string;
	commit_age_days?: number;
	compatibility?: string;
	allowed_tools?: string[];
	license?: string;
	license_url?: string;
	missing_bins?: string[];
	install_options?: { label?: string; kind?: string }[];
	requires?: { bins?: string[]; any_bins?: string[] };
	provenance?: { original_source?: string; original_commit_sha?: string; imported_from?: string };
	quarantine_reason?: string;
}
interface RepoSummary {
	source: string;
	skill_count: number;
	enabled_count: number;
	commit_sha?: string;
	quarantined?: boolean;
	drifted?: boolean;
	orphaned?: boolean;
	repo_name?: string;
	provenance?: { original_source?: string; original_commit_sha?: string; imported_from?: string };
	quarantine_reason?: string;
}
interface ToastItem {
	id: number;
	message: string;
	type: string;
}
interface InstallProgress {
	id: string;
	source: string;
	state: string;
}

const repos = signal<RepoSummary[]>([]);
const enabledSkills = signal<SkillSummary[]>([]);
const loading = signal(false);
const toasts = signal<ToastItem[]>([]);
let toastId = 0;
const installProgresses = signal<InstallProgress[]>([]);
let installProgressId = 0;

let prefetchPromise: Promise<unknown> | null = null;
function ensurePrefetch(): Promise<unknown> {
	if (!prefetchPromise)
		prefetchPromise = fetch("/api/skills")
			.then((r) => r.json())
			.then((data) => {
				if (data.skills) enabledSkills.value = data.skills;
				if (data.repos) repos.value = data.repos;
				return data;
			})
			.catch(() => null);
	return prefetchPromise;
}

const skillRepoMap = computed<Record<string, string>>(() => {
	const map: Record<string, string> = {};
	enabledSkills.value.forEach((s) => {
		if (s.source) map[s.name] = s.source;
	});
	return map;
});

function showToast(message: string, type: string): void {
	const id = ++toastId;
	toasts.value = toasts.value.concat([{ id, message, type }]);
	setTimeout(() => {
		toasts.value = toasts.value.filter((t) => t.id !== id);
	}, 4000);
}
function shortSha(sha: string | null | undefined): string {
	return sha?.slice(0, 12) || "";
}
function startInstallProgress(source: string, id?: string): string {
	const pid = id || `install-${++installProgressId}`;
	if (installProgresses.value.some((p) => p.id === pid)) return pid;
	installProgresses.value = [...installProgresses.value, { id: pid, source: source || "repository", state: "running" }];
	return pid;
}
function stopInstallProgress(id: string, _ok: boolean): void {
	installProgresses.value = installProgresses.value.filter((p) => p.id !== id);
}

function emergencyDisableAllSkills(): void {
	requestConfirm("Disable all third-party skills now?", { confirmLabel: "Disable All", danger: true }).then((yes) => {
		if (!yes) return;
		sendRpc("skills.emergency_disable", {}).then((res) => {
			if (!res?.ok) {
				showToast(`Emergency disable failed: ${res?.error || "unknown"}`, "error");
				return;
			}
			showToast(`Disabled ${(res.payload as Record<string, number>)?.skills_disabled || 0} skills`, "success");
			fetchAll();
		});
	});
}

function fetchAll(): void {
	loading.value = true;
	fetch("/api/skills")
		.then((r) => r.json())
		.then((data) => {
			if (data.skills) enabledSkills.value = data.skills;
			if (data.repos) repos.value = data.repos;
			loading.value = false;
			updateNavCount("skills", (data.skills || []).length);
		})
		.catch(() => {
			loading.value = false;
		});
}

function doInstall(source: string): Promise<void> {
	if (!(source && S.connected)) {
		if (!S.connected) showToast("Not connected to gateway.", "error");
		return Promise.resolve();
	}
	const opId = `skills-install-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
	const pid = startInstallProgress(source, opId);
	return sendRpc("skills.install", { source, op_id: opId }).then((res) => {
		if (res?.ok) {
			const p = (res.payload || {}) as Record<string, unknown[]>;
			showToast(`Installed ${source} (${(p.installed || []).length} skills)`, "success");
			fetchAll();
			stopInstallProgress(pid, true);
		} else {
			showToast(`Failed: ${res?.error || "unknown error"}`, "error");
			stopInstallProgress(pid, false);
		}
	});
}
function doImportBundle(path: string): Promise<void> {
	if (!(path && S.connected)) {
		if (!S.connected) showToast("Not connected.", "error");
		return Promise.resolve();
	}
	return sendRpc("skills.repos.import", { path }).then((res) => {
		if (res?.ok) {
			const p = (res.payload || {}) as Record<string, unknown>;
			showToast(`Imported ${p.repo_name || "bundle"} (${p.skill_count || 0} skills)`, "success");
			fetchAll();
		} else showToast(`Failed: ${res?.error || "unknown"}`, "error");
	});
}
function doExportBundle(source: string, path: string | null): Promise<void> {
	if (!(source && S.connected)) return Promise.resolve();
	const params: Record<string, string> = { source };
	if (path) params.path = path;
	return sendRpc("skills.repos.export", params).then((res) => {
		if (res?.ok) showToast(`Exported ${source}`, "success");
		else showToast(`Failed: ${res?.error || "unknown"}`, "error");
	});
}
export function doUnquarantine(source: string): Promise<void> {
	if (!(source && S.connected)) return Promise.resolve();
	return sendRpc("skills.repos.unquarantine", { source }).then((res) => {
		if (res?.ok) {
			showToast(`Cleared quarantine for ${source}`, "success");
			fetchAll();
		} else showToast(`Failed: ${res?.error || "unknown"}`, "error");
	});
}
function searchSkills(source: string, query: string): Promise<SkillSummary[]> {
	return fetch(`/api/skills/search?source=${encodeURIComponent(source)}&q=${encodeURIComponent(query)}`)
		.then((r) => r.json())
		.then((d) => d.skills || []);
}

function Toasts(): VNode {
	return (
		<div className="skills-toast-container">
			{toasts.value.map((t) => {
				const bg = t.type === "error" ? "var(--error, #e55)" : "var(--accent)";
				return (
					<div
						key={t.id}
						style={{
							pointerEvents: "auto",
							maxWidth: "420px",
							padding: "10px 16px",
							borderRadius: "6px",
							fontSize: ".8rem",
							fontWeight: 500,
							color: "#fff",
							background: bg,
							boxShadow: "0 4px 12px rgba(0,0,0,.15)",
						}}
					>
						{t.message}
					</div>
				);
			})}
		</div>
	);
}
function InstallProgressBar(): VNode | null {
	const items = installProgresses.value;
	if (!items.length) return null;
	return (
		<div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
			{items.map((p) => (
				<div
					key={p.id}
					style={{
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-sm)",
						padding: "8px 10px",
						background: "var(--surface)",
						fontSize: ".78rem",
						color: "var(--muted)",
					}}
				>
					<strong style={{ color: "var(--text-strong)" }}>Installing {p.source}...</strong>
					<div style={{ marginTop: "3px" }}>This may take a while.</div>
				</div>
			))}
		</div>
	);
}

function InstallBox(): VNode {
	const ref = useRef<HTMLInputElement>(null);
	const installing = useSignal(false);
	function go(): void {
		const v = ref.current?.value.trim();
		if (!v) return;
		installing.value = true;
		doInstall(v).then(() => {
			installing.value = false;
			if (ref.current) ref.current.value = "";
		});
	}
	return (
		<div className="skills-install-box">
			<input
				ref={ref}
				type="text"
				placeholder="owner/repo or full URL (e.g. anthropics/skills)"
				className="skills-install-input"
				onKeyDown={(e) => {
					if ((e as KeyboardEvent).key === "Enter") go();
				}}
			/>
			<button className="provider-btn" onClick={go} disabled={installing.value}>
				{installing.value ? "Installing\u2026" : "Install"}
			</button>
		</div>
	);
}
function BundleTransferBox(): VNode {
	const ref = useRef<HTMLInputElement>(null);
	const importing = useSignal(false);
	function go(): void {
		const p = ref.current?.value.trim();
		if (!p) return;
		importing.value = true;
		doImportBundle(p).finally(() => {
			importing.value = false;
		});
	}
	return (
		<div className="skills-install-box">
			<input
				ref={ref}
				type="text"
				placeholder="/path/to/skill-bundle.tar.gz"
				className="skills-install-input"
				onKeyDown={(e) => {
					if ((e as KeyboardEvent).key === "Enter") go();
				}}
			/>
			<button className="provider-btn provider-btn-secondary" onClick={go} disabled={importing.value}>
				{importing.value ? "Importing\u2026" : "Import Bundle"}
			</button>
		</div>
	);
}

const featuredSkills = [
	{ repo: "openclaw/skills", desc: "Community skills from ClawdHub" },
	{ repo: "anthropics/skills", desc: "Official Anthropic agent skills" },
	{ repo: "vercel-labs/agent-skills", desc: "Vercel agent skills collection" },
	{ repo: "vercel-labs/skills", desc: "Vercel skills toolkit" },
];
function FeaturedCard({ skill: f }: { skill: { repo: string; desc: string } }): VNode {
	const installing = useSignal(false);
	const href = /^https?:\/\//.test(f.repo) ? f.repo : `https://github.com/${f.repo}`;
	return (
		<div className="skills-featured-card">
			<div>
				<a
					href={href}
					target="_blank"
					rel="noopener noreferrer"
					style={{
						fontFamily: "var(--font-mono)",
						fontSize: ".82rem",
						fontWeight: 500,
						color: "var(--text-strong)",
						textDecoration: "none",
					}}
				>
					{f.repo}
				</a>
				<div style={{ fontSize: ".75rem", color: "var(--muted)" }}>{f.desc}</div>
			</div>
			<button
				onClick={() => {
					installing.value = true;
					doInstall(f.repo).then(() => {
						installing.value = false;
					});
				}}
				disabled={installing.value}
				style={{
					background: "var(--surface2)",
					border: "1px solid var(--border)",
					color: "var(--text)",
					borderRadius: "var(--radius-sm)",
					fontSize: ".72rem",
					padding: "4px 10px",
					cursor: "pointer",
					whiteSpace: "nowrap",
				}}
			>
				{installing.value ? "Installing\u2026" : "Install"}
			</button>
		</div>
	);
}
function FeaturedSection(): VNode {
	return (
		<div className="skills-section">
			<h3 className="skills-section-title">Featured Repositories</h3>
			<div className="skills-featured-grid">
				{featuredSkills.map((f) => (
					<FeaturedCard key={f.repo} skill={f} />
				))}
			</div>
		</div>
	);
}

function SkillMetadata({ detail: d }: { detail: SkillDetail }): VNode | null {
	if (!(d.author || d.version || d.homepage || d.source_url || d.commit_sha)) return null;
	return (
		<div
			style={{
				display: "flex",
				alignItems: "center",
				gap: "12px",
				marginBottom: "8px",
				fontSize: ".75rem",
				color: "var(--muted)",
				flexWrap: "wrap",
			}}
		>
			{d.author && <span>Author: {d.author}</span>}
			{d.version && <span>v{d.version}</span>}
			{d.commit_sha && (
				<span>
					Commit: <code>{shortSha(d.commit_sha)}</code>
				</span>
			)}
			{d.homepage && (
				<a
					href={d.homepage}
					target="_blank"
					rel="noopener noreferrer"
					style={{ color: "var(--accent)", textDecoration: "none" }}
				>
					{d.homepage.replace(/^https?:\/\//, "")}
				</a>
			)}
		</div>
	);
}

function SkillDetailPanel({
	detail: d,
	repoSource,
	onClose,
	onReload,
}: {
	detail: SkillDetail;
	repoSource?: string;
	onClose: () => void;
	onReload?: () => void;
}): VNode | null {
	const actionBusy = useSignal(false);
	const bodyRef = useRef<HTMLDivElement>(null);
	useEffect(() => {
		if (bodyRef.current && d?.body_html) {
			bodyRef.current.textContent = "";
			// Safe: body_html is server-rendered trusted HTML from SKILL.md via pulldown-cmark
			const tpl = document.createElement("template");
			tpl.innerHTML = d.body_html;
			bodyRef.current.appendChild(tpl.content);
			bodyRef.current.querySelectorAll("a").forEach((a) => {
				a.setAttribute("target", "_blank");
				a.setAttribute("rel", "noopener");
			});
		}
	}, [d?.body_html]);
	if (!d) return null;
	const isDisc = isDiscoveredSource(d.source);
	function doToggle(): void {
		actionBusy.value = true;
		sendRpc(d.enabled ? "skills.skill.disable" : "skills.skill.enable", { source: repoSource, skill: d.name }).then(
			(r) => {
				actionBusy.value = false;
				if (r?.ok) {
					if (isDisc) onClose();
					fetchAll();
					onReload?.();
				} else showToast(`Failed: ${r?.error || "unknown"}`, "error");
			},
		);
	}
	function onToggle(): void {
		if (!S.connected || actionBusy.value) return;
		if (isDisc && d.protected) {
			showToast(`Protected`, "error");
			return;
		}
		if (isDisc && d.enabled) {
			requestConfirm(`Delete "${d.name}"?`, { confirmLabel: "Delete", danger: true }).then((y) => {
				if (y) doToggle();
			});
			return;
		}
		doToggle();
	}
	return (
		<div className="skills-detail-panel" style={{ display: "block" }}>
			<div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: "8px" }}>
				<span
					style={{ fontFamily: "var(--font-mono)", fontSize: ".9rem", fontWeight: 600, color: "var(--text-strong)" }}
				>
					{d.display_name || d.name}
				</span>
				<div style={{ display: "flex", gap: "6px" }}>
					<button
						onClick={onToggle}
						disabled={actionBusy.value}
						className={
							isDisc && d.enabled
								? "provider-btn provider-btn-sm provider-btn-danger"
								: "provider-btn provider-btn-sm provider-btn-secondary"
						}
					>
						{actionBusy.value ? "..." : isDisc && d.enabled ? "Delete" : d.enabled ? "Disable" : "Enable"}
					</button>
					<button
						onClick={onClose}
						style={{ background: "none", border: "none", color: "var(--muted)", cursor: "pointer" }}
					>
						&times;
					</button>
				</div>
			</div>
			<SkillMetadata detail={d} />
			{d.description && <p style={{ margin: "0 0 8px", fontSize: ".82rem" }}>{d.description}</p>}
			{d.body_html && (
				<div
					style={{
						marginTop: "10px",
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-sm)",
						background: "var(--surface2)",
					}}
				>
					<div
						style={{
							padding: "6px 10px",
							borderBottom: "1px solid var(--border)",
							fontSize: ".68rem",
							color: "var(--muted)",
							fontFamily: "var(--font-mono)",
							textTransform: "uppercase",
						}}
					>
						SKILL.md source
					</div>
					<div
						ref={bodyRef}
						className="skill-body-md"
						style={{ padding: "10px", maxHeight: "400px", overflowY: "auto", fontSize: ".8rem", lineHeight: 1.5 }}
					/>
				</div>
			)}
			{!d.body_html && d.body && (
				<div
					style={{
						marginTop: "10px",
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-sm)",
						background: "var(--surface2)",
					}}
				>
					<pre
						style={{
							whiteSpace: "pre-wrap",
							wordBreak: "break-word",
							fontSize: ".78rem",
							fontFamily: "var(--font-mono)",
							margin: 0,
							padding: "10px",
							maxHeight: "400px",
							overflowY: "auto",
						}}
					>
						{d.body}
					</pre>
				</div>
			)}
		</div>
	);
}

function RepoCard({ repo }: { repo: RepoSummary }): VNode {
	const expanded = useSignal(false);
	const searchQuery = useSignal("");
	const searchResults = useSignal<SkillSummary[]>([]);
	const allSkills = useSignal<SkillSummary[]>([]);
	const searching = useSignal(false);
	const activeDetail = useSignal<SkillDetail | null>(null);
	const detailLoading = useSignal(false);
	const searchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
	const removingRepo = useSignal(false);
	const exportingRepo = useSignal(false);
	const unquarantiningRepo = useSignal(false);
	const isOrphan = repo.orphaned === true;
	const sourceLabel = isOrphan ? repo.repo_name : repo.source;
	const href = isOrphan ? null : /^https?:\/\//.test(repo.source) ? repo.source : `https://github.com/${repo.source}`;
	function toggle(): void {
		const w = !expanded.value;
		expanded.value = w;
		if (w && !isOrphan && !allSkills.value.length) {
			searching.value = true;
			searchSkills(repo.source, "").then((r) => {
				allSkills.value = r;
				searching.value = false;
			});
		}
	}
	function onSearch(e: Event): void {
		if (isOrphan) return;
		const q = (e.target as HTMLInputElement).value;
		searchQuery.value = q;
		activeDetail.value = null;
		if (searchTimer.current) clearTimeout(searchTimer.current);
		if (!q.trim()) {
			searchResults.value = [];
			return;
		}
		searching.value = true;
		searchTimer.current = setTimeout(() => {
			searchSkills(repo.source, q.trim()).then((r) => {
				searchResults.value = r;
				searching.value = false;
			});
		}, 200);
	}
	function loadDetail(s: SkillSummary): void {
		detailLoading.value = true;
		sendRpc("skills.skill.detail", { source: repo.source, skill: s.name }).then((r) => {
			detailLoading.value = false;
			if (r?.ok) activeDetail.value = r.payload as SkillDetail;
			else showToast(`Failed: ${r?.error || "unknown"}`, "error");
		});
	}
	const displayed = searchQuery.value.trim() ? searchResults.value : allSkills.value;
	return (
		<div className="skills-repo-card">
			<div className="skills-repo-header" onClick={toggle}>
				<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
					<span style={{ fontSize: ".65rem", color: "var(--muted)", transform: expanded.value ? "rotate(90deg)" : "" }}>
						{"\u25B6"}
					</span>
					{href ? (
						<a
							href={href}
							target="_blank"
							rel="noopener noreferrer"
							onClick={(e) => e.stopPropagation()}
							style={{
								fontFamily: "var(--font-mono)",
								fontSize: ".82rem",
								fontWeight: 500,
								color: "var(--text-strong)",
								textDecoration: "none",
							}}
						>
							{sourceLabel}
						</a>
					) : (
						<span
							style={{
								fontFamily: "var(--font-mono)",
								fontSize: ".82rem",
								fontWeight: 500,
								color: "var(--text-strong)",
							}}
						>
							{sourceLabel}
						</span>
					)}
					<span style={{ fontSize: ".72rem", color: "var(--muted)" }}>
						{repo.enabled_count}/{repo.skill_count} enabled
					</span>
				</div>
				<div style={{ display: "flex", gap: "6px" }}>
					{!isOrphan && (
						<button
							className="provider-btn provider-btn-sm provider-btn-secondary"
							disabled={exportingRepo.value}
							onClick={(e) => {
								e.stopPropagation();
								exportingRepo.value = true;
								doExportBundle(repo.source, null).finally(() => {
									exportingRepo.value = false;
								});
							}}
						>
							{exportingRepo.value ? "Exporting..." : "Export"}
						</button>
					)}
					{repo.quarantined && (
						<button
							className="provider-btn provider-btn-sm provider-btn-secondary"
							disabled={unquarantiningRepo.value}
							onClick={(e) => {
								e.stopPropagation();
								if (!S.connected || unquarantiningRepo.value) return;
								requestConfirm(`Clear quarantine for ${repo.source}?`, {
									confirmLabel: "Clear Quarantine",
								}).then((confirmed) => {
									if (!confirmed) return;
									unquarantiningRepo.value = true;
									doUnquarantine(repo.source).finally(() => {
										unquarantiningRepo.value = false;
									});
								});
							}}
						>
							{unquarantiningRepo.value ? "Clearing..." : "Clear Quarantine"}
						</button>
					)}
					<button
						className="provider-btn provider-btn-sm provider-btn-danger"
						disabled={removingRepo.value}
						onClick={(e) => {
							e.stopPropagation();
							removingRepo.value = true;
							sendRpc("skills.repos.remove", { source: repo.source }).then((r) => {
								removingRepo.value = false;
								if (r?.ok) fetchAll();
								else showToast(`Failed: ${r?.error || "unknown"}`, "error");
							});
						}}
					>
						{removingRepo.value ? "Removing..." : "Remove"}
					</button>
				</div>
			</div>
			{(repo.quarantined || repo.provenance) && expanded.value && (
				<div style={{ padding: "8px 12px", fontSize: ".78rem", color: "var(--muted)" }}>
					{repo.quarantined && (
						<div style={{ marginBottom: "6px", color: "var(--warning, #c77d00)", fontWeight: 600 }}>
							Quarantined{repo.quarantine_reason ? `: ${repo.quarantine_reason}` : ""}
						</div>
					)}
					{repo.provenance?.original_source && (
						<div>
							<strong>Original source:</strong> {repo.provenance.original_source}
						</div>
					)}
					{repo.provenance?.original_commit_sha && (
						<div>
							<strong>Original commit:</strong> <code>{shortSha(repo.provenance.original_commit_sha)}</code>
						</div>
					)}
					{repo.provenance?.imported_from && (
						<div>
							<strong>Imported from:</strong> <code>{repo.provenance.imported_from}</code>
						</div>
					)}
				</div>
			)}
			{expanded.value && (
				<div className="skills-repo-detail" style={{ display: "block" }}>
					<div style={{ marginBottom: "8px" }}>
						<input
							type="text"
							placeholder={`Search skills in ${repo.source}\u2026`}
							value={searchQuery.value}
							disabled={isOrphan}
							onInput={onSearch}
							style={{
								width: "100%",
								padding: "6px 10px",
								border: "1px solid var(--border)",
								borderRadius: "var(--radius-sm)",
								background: "var(--surface)",
								color: "var(--text)",
								fontSize: ".8rem",
								fontFamily: "var(--font-mono)",
								boxSizing: "border-box",
							}}
						/>
					</div>
					{!activeDetail.value && displayed.length > 0 && (
						<div className="skills-browse-list">
							{displayed.map((sk) => (
								<div key={sk.name} className="skills-ac-item" onClick={() => loadDetail(sk)}>
									<span style={{ fontFamily: "var(--font-mono)", fontWeight: 500, color: "var(--text-strong)" }}>
										{sk.display_name || sk.name}
									</span>
									{sk.description && (
										<span style={{ color: "var(--muted)", fontSize: ".72rem", marginLeft: "6px" }}>
											{sk.description}
										</span>
									)}
								</div>
							))}
						</div>
					)}
					{searching.value && !activeDetail.value && (
						<div style={{ padding: "8px", color: "var(--muted)", fontSize: ".78rem" }}>Searching...</div>
					)}
					{activeDetail.value && (
						<SkillDetailPanel
							detail={activeDetail.value}
							repoSource={repo.source}
							onClose={() => {
								activeDetail.value = null;
							}}
							onReload={() => loadDetail({ name: activeDetail.value?.name } as SkillSummary)}
						/>
					)}
				</div>
			)}
		</div>
	);
}

const bundledCategories = signal<BundledCategory[]>([]);
const bundledTotal = signal(0);

function fetchBundledCategories(): void {
	sendRpc("skills.bundled.categories", {}).then((res) => {
		if (res?.ok) {
			const payload = res.payload as { categories?: BundledCategory[]; total_skills?: number };
			bundledCategories.value = payload.categories || [];
			bundledTotal.value = payload.total_skills || 0;
		}
	});
}

function BundledCategoriesSection(): VNode {
	const cats = bundledCategories.value;
	const toggling = useSignal<string | null>(null);

	useEffect(() => {
		fetchBundledCategories();
	}, []);

	if (!cats.length) return <></>;

	function toggle(cat: BundledCategory): void {
		if (toggling.value) return;
		const newEnabled = !cat.enabled;
		toggling.value = cat.name;
		sendRpc("skills.bundled.toggle_category", { category: cat.name, enabled: newEnabled }).then((res) => {
			toggling.value = null;
			if (res?.ok) {
				bundledCategories.value = bundledCategories.value.map((c) =>
					c.name === cat.name ? { ...c, enabled: newEnabled } : c,
				);
				fetchAll();
			} else {
				showToast(`Failed: ${res?.error || "unknown"}`, "error");
			}
		});
	}

	const enabledCount = cats.filter((c) => c.enabled).length;

	return (
		<div className="skills-section">
			<div className="flex items-center gap-3 mb-2">
				<h3 className="skills-section-title" style={{ margin: 0 }}>
					{t("skills:bundledTitle")}
					<span className="ml-2 text-xs font-normal text-[var(--muted)]">
						({enabledCount}/{cats.length} enabled)
					</span>
				</h3>
			</div>
			<p className="text-xs text-[var(--muted)] mb-3">{t("skills:bundledDescription")}</p>
			<div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
				{cats.map((cat) => {
					const meta = CATEGORY_META[cat.name];
					const icon = meta?.icon || "\uD83D\uDCE6";
					return (
						<button
							key={cat.name}
							type="button"
							onClick={() => toggle(cat)}
							disabled={toggling.value === cat.name}
							className={`flex items-center gap-2 px-3 py-2 rounded-md border text-left cursor-pointer transition-colors ${
								cat.enabled
									? "border-[var(--accent)] bg-[var(--accent-bg,rgba(var(--accent-rgb,59,130,246),0.08))]"
									: "border-[var(--border)] bg-[var(--surface)] opacity-60"
							}`}
						>
							<span className="text-base shrink-0">{icon}</span>
							<div className="flex-1 min-w-0">
								<span className="text-xs font-medium text-[var(--text-strong)]">{categoryLabel(cat.name)}</span>
								<span className="text-xs text-[var(--muted)] ml-1">({cat.count})</span>
								{meta?.desc && <div className="text-xs text-[var(--muted)] truncate">{meta.desc}</div>}
							</div>
							{cat.enabled ? (
								<span className="icon icon-check-circle text-[var(--accent)] shrink-0" />
							) : (
								<span className="w-4 h-4 rounded-full border-2 border-[var(--border)] inline-block shrink-0" />
							)}
						</button>
					);
				})}
			</div>
		</div>
	);
}

function ReposSection(): VNode {
	return (
		<div className="skills-section">
			<h3 className="skills-section-title">Installed Repositories</h3>
			<div className="skills-section">
				{!repos.value?.length && (
					<div style={{ padding: "12px", color: "var(--muted)", fontSize: ".82rem" }}>No repositories installed.</div>
				)}
				{repos.value.map((r) => (
					<RepoCard key={r.source} repo={r} />
				))}
			</div>
		</div>
	);
}

function EnabledSkillsTable(): VNode | null {
	const s = enabledSkills.value;
	const map = skillRepoMap.value;
	const activeDetail = useSignal<SkillDetail | null>(null);
	const detailLoading = useSignal(false);
	const pending = useSignal<string | null>(null);
	const activeCategory = useSignal<string | null>(null);
	const searchQuery = useSignal("");
	if (!s?.length) return null;

	// Build sorted category list from skills
	const categories = computed(() => {
		const cats = new Set<string>();
		for (const sk of enabledSkills.value) {
			cats.add(sk.category || "other");
		}
		return Array.from(cats).sort();
	});

	// Filter skills by search query and active category
	const filtered = s.filter((sk) => {
		if (activeCategory.value && (sk.category || "other") !== activeCategory.value) return false;
		if (searchQuery.value) {
			const q = searchQuery.value.toLowerCase();
			return sk.name.toLowerCase().includes(q) || (sk.description || "").toLowerCase().includes(q);
		}
		return true;
	});

	function isDisc(sk: SkillSummary): boolean {
		return isDiscoveredSource(sk.source);
	}
	function doDisable(sk: SkillSummary): void {
		pending.value = sk.name;
		sendRpc("skills.skill.disable", { source: map[sk.name] || sk.source, skill: sk.name }).then((r) => {
			pending.value = null;
			if (r?.ok) {
				activeDetail.value = null;
				showToast(isDisc(sk) ? `Deleted ${sk.name}` : `Disabled ${sk.name}`, "success");
				fetchAll();
			} else showToast(`Failed: ${r?.error || "unknown"}`, "error");
		});
	}
	function onDisable(sk: SkillSummary): void {
		if (pending.value) return;
		if (isDisc(sk) && sk.protected) {
			showToast("Protected", "error");
			return;
		}
		if (isDisc(sk)) {
			requestConfirm(`Delete "${sk.name}"?`, { confirmLabel: "Delete", danger: true }).then((y) => {
				if (y) doDisable(sk);
			});
			return;
		}
		doDisable(sk);
	}
	function loadDetail(sk: SkillSummary): void {
		if (activeDetail.value?.name === sk.name) {
			activeDetail.value = null;
			return;
		}
		detailLoading.value = true;
		sendRpc("skills.skill.detail", { source: map[sk.name] || sk.source, skill: sk.name }).then((r) => {
			detailLoading.value = false;
			if (r?.ok) activeDetail.value = r.payload as SkillDetail;
		});
	}
	return (
		<div className="skills-section">
			<div className="flex items-center gap-3 mb-2">
				<h3 className="skills-section-title" style={{ margin: 0 }}>
					Enabled Skills
					<span className="ml-2 text-xs font-normal text-[var(--muted)]">
						({filtered.length}
						{filtered.length !== s.length ? ` of ${s.length}` : ""})
					</span>
				</h3>
				<input
					type="text"
					placeholder="Search skills..."
					value={searchQuery.value}
					onInput={(e) => {
						searchQuery.value = (e.target as HTMLInputElement).value;
					}}
					className="skills-install-input"
					style={{ maxWidth: "240px", fontSize: ".78rem", padding: "4px 8px" }}
				/>
			</div>
			{categories.value.length > 1 && (
				<div className="flex flex-wrap gap-1.5 mb-3">
					<button
						className={`skills-category-pill ${activeCategory.value === null ? "active" : ""}`}
						onClick={() => {
							activeCategory.value = null;
						}}
					>
						All ({s.length})
					</button>
					{categories.value.map((cat) => {
						const count = s.filter((sk) => (sk.category || "other") === cat).length;
						return (
							<button
								key={cat}
								className={`skills-category-pill ${activeCategory.value === cat ? "active" : ""}`}
								onClick={() => {
									activeCategory.value = activeCategory.value === cat ? null : cat;
								}}
							>
								{cat} ({count})
							</button>
						);
					})}
				</div>
			)}
			<div className="skills-table-wrap">
				<table style={{ width: "100%", borderCollapse: "collapse", fontSize: ".82rem" }}>
					<thead>
						<tr style={{ borderBottom: "1px solid var(--border)", background: "var(--surface)" }}>
							<th
								style={{
									textAlign: "left",
									padding: "8px 12px",
									fontWeight: 500,
									color: "var(--muted)",
									fontSize: ".75rem",
									textTransform: "uppercase",
								}}
							>
								Name
							</th>
							<th
								style={{
									textAlign: "left",
									padding: "8px 12px",
									fontWeight: 500,
									color: "var(--muted)",
									fontSize: ".75rem",
									textTransform: "uppercase",
								}}
							>
								Description
							</th>
							<th
								style={{
									textAlign: "left",
									padding: "8px 12px",
									fontWeight: 500,
									color: "var(--muted)",
									fontSize: ".75rem",
									textTransform: "uppercase",
								}}
							>
								Source
							</th>
							<th />
						</tr>
					</thead>
					<tbody>
						{filtered.map((sk) => {
							const isActive = activeDetail.value?.name === sk.name;
							return (
								<>
									<tr
										key={sk.name}
										className="cursor-pointer"
										style={{
											borderBottom: isActive ? "none" : "1px solid var(--border)",
											background: isActive ? "var(--bg-hover)" : undefined,
										}}
										onClick={() => loadDetail(sk)}
									>
										<td
											style={{
												padding: "8px 12px",
												fontWeight: 500,
												color: "var(--accent)",
												fontFamily: "var(--font-mono)",
											}}
										>
											{sk.name}
											{sk.category && !activeCategory.value && (
												<span className="skills-category-badge">{sk.category}</span>
											)}
										</td>
										<td style={{ padding: "8px 12px" }}>{sk.description || "\u2014"}</td>
										<td style={{ padding: "8px 12px" }}>
											<span className={isRepoSource(sk.source) ? "tier-badge" : "recommended-badge"}>{sk.source}</span>
										</td>
										<td style={{ padding: "8px 12px", textAlign: "right" }}>
											{sk.source !== SkillSource.Bundled && (
												<button
													disabled={(isDisc(sk) && sk.protected === true) || pending.value === sk.name}
													className={
														isDisc(sk)
															? "provider-btn provider-btn-sm provider-btn-danger"
															: "provider-btn provider-btn-sm provider-btn-secondary"
													}
													onClick={(e) => {
														e.stopPropagation();
														onDisable(sk);
													}}
												>
													{pending.value === sk.name ? "..." : isDisc(sk) ? "Delete" : "Disable"}
												</button>
											)}
										</td>
									</tr>
									{isActive && activeDetail.value && (
										<tr key={`${sk.name}-detail`}>
											<td colSpan={4} style={{ padding: 0, borderBottom: "1px solid var(--border)" }}>
												<SkillDetailPanel
													detail={activeDetail.value}
													repoSource={activeDetail.value.source}
													onClose={() => {
														activeDetail.value = null;
													}}
													onReload={() =>
														loadDetail({
															name: activeDetail.value?.name,
															source: activeDetail.value?.source,
														} as SkillSummary)
													}
												/>
											</td>
										</tr>
									)}
								</>
							);
						})}
					</tbody>
				</table>
			</div>
		</div>
	);
}

const activeTab = signal("skills");

const skillsTabs = computed(() => {
	const enabledCats = bundledCategories.value.filter((c) => c.enabled).length;
	const totalCats = bundledCategories.value.length;
	return [
		{ id: "skills", label: "Skills", badge: enabledSkills.value.length || undefined },
		{ id: "categories", label: "Categories", badge: totalCats ? `${enabledCats}/${totalCats}` : undefined },
		{ id: "repositories", label: "Repositories", badge: repos.value.length || undefined },
	];
});

function SkillsPageComponent(): VNode {
	useEffect(() => {
		ensurePrefetch().then(() => fetchAll());
		fetchBundledCategories();
		const off = onEvent("skills.install.progress", (p: unknown) => {
			const d = p as Record<string, string>;
			if (!d?.op_id) return;
			if (d.phase === "start") startInstallProgress(d.source || "repository", d.op_id);
			else if (d.phase === "done") stopInstallProgress(d.op_id, true);
			else if (d.phase === "error") stopInstallProgress(d.op_id, false);
		});
		return off;
	}, []);
	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div className="flex items-center gap-3">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Skills</h2>
				<button className="provider-btn provider-btn-secondary provider-btn-sm" onClick={fetchAll}>
					Refresh
				</button>
				<button className="provider-btn provider-btn-danger provider-btn-sm" onClick={emergencyDisableAllSkills}>
					Emergency Disable
				</button>
			</div>
			<p className="text-sm text-[var(--muted)]">
				SKILL.md-based skills.{" "}
				<a
					href="https://platform.claude.com/docs/en/agents-and-tools/agent-skills/overview"
					target="_blank"
					rel="noopener noreferrer"
					className="text-[var(--accent)]"
				>
					How to write a skill?
				</a>
			</p>
			<TabBar
				tabs={skillsTabs.value}
				active={activeTab.value}
				onChange={(id) => {
					activeTab.value = id;
				}}
			/>
			{activeTab.value === "skills" && (
				<>
					{loading.value && !enabledSkills.value.length && (
						<div style={{ padding: "24px", textAlign: "center", color: "var(--muted)" }}>Loading skills...</div>
					)}
					<EnabledSkillsTable />
				</>
			)}
			{activeTab.value === "categories" && <BundledCategoriesSection />}
			{activeTab.value === "repositories" && (
				<>
					<InstallBox />
					<BundleTransferBox />
					<InstallProgressBar />
					<FeaturedSection />
					<ReposSection />
				</>
			)}
		</div>
	);
}

let _skillsContainer: HTMLElement | null = null;
export function initSkills(container: HTMLElement): void {
	_skillsContainer = container;
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	render(
		<>
			<SkillsPageComponent />
			<Toasts />
			<ConfirmDialog />
		</>,
		container,
	);
}
export function teardownSkills(): void {
	if (_skillsContainer) render(null, _skillsContainer);
	_skillsContainer = null;
}
registerPage(routes.skills!, initSkills, teardownSkills);

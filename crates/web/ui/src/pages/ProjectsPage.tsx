// ── Projects page (Preact + Signals) ──────────────────────────

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef } from "preact/hooks";
import { sendRpc } from "../helpers";
import { t } from "../i18n";
import { fetchProjects } from "../projects";
import { registerPage } from "../router";
import { routes } from "../routes";
import * as S from "../state";
import { projects as projectsSig } from "../stores/project-store";
import type { ProjectInfo } from "../types";
import { ConfirmDialog, requestConfirm } from "../ui";

interface Project extends ProjectInfo {
	label: string;
	directory: string;
	system_prompt?: string | null;
	setup_command?: string | null;
	teardown_command?: string | null;
	branch_prefix?: string | null;
	auto_worktree?: boolean;
	sandbox_image?: string | null;
	detected?: boolean;
	created_at?: number;
	updated_at?: number;
}

interface CachedImage {
	tag: string;
}

interface PathInputProps {
	onAdd: (dir: string) => Promise<void>;
}

interface ProjectEditFormProps {
	project: Project;
}

interface ProjectCardProps {
	project: Project;
}

const completions = signal<string[]>([]);
const editingProject = signal<string | null>(null);
const detecting = signal(false);
const clearing = signal(false);
let _projectsContainer: HTMLElement | null = null;

function PathInput(props: PathInputProps): VNode {
	const inputRef = useRef<HTMLInputElement>(null);
	const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

	function onInput(): void {
		if (timerRef.current != null) clearTimeout(timerRef.current);
		timerRef.current = setTimeout(() => {
			const val = inputRef.current?.value || "";
			if (val.length < 2) {
				completions.value = [];
				return;
			}
			sendRpc("projects.complete_path", { partial: val }).then((res) => {
				if (!res?.ok) {
					completions.value = [];
					return;
				}
				completions.value = (res.payload as string[]) || [];
			});
		}, 200);
	}

	function selectPath(p: string): void {
		if (inputRef.current) {
			inputRef.current.value = `${p}/`;
			inputRef.current.focus();
		}
		completions.value = [];
		onInput();
	}

	return (
		<div className="project-dir-group">
			<div className="text-xs text-[var(--muted)] mb-1">{t("projects:pathInput.directory")}</div>
			<div className="flex gap-2 items-center">
				<input
					ref={inputRef}
					type="text"
					className="provider-key-input flex-1"
					placeholder={t("projects:pathInput.placeholder")}
					style={{ fontFamily: "var(--font-mono)" }}
					onInput={onInput}
				/>
				<button
					className="provider-btn"
					onClick={() => {
						const dir = inputRef.current?.value.trim();
						if (!dir) return;
						props.onAdd(dir).then(() => {
							if (inputRef.current) inputRef.current.value = "";
						});
					}}
				>
					{t("common:actions.add")}
				</button>
			</div>
			{completions.value.length > 0 && (
				<div className="project-completion" style={{ display: "block" }}>
					{completions.value.map((p) => (
						<div key={p} className="project-completion-item" onClick={() => selectPath(p)}>
							{p}
						</div>
					))}
				</div>
			)}
		</div>
	);
}

const cachedImages = signal<CachedImage[]>([]);

function fetchCachedImages(): void {
	fetch("/api/images/cached")
		.then((r) => (r.ok ? r.json() : { images: [] }))
		.then((data: { images?: CachedImage[] }) => {
			cachedImages.value = data.images || [];
		})
		.catch(() => {
			cachedImages.value = [];
		});
}

function ProjectEditForm(props: ProjectEditFormProps): VNode {
	const p = props.project;
	const labelRef = useRef<HTMLInputElement>(null);
	const dirRef = useRef<HTMLInputElement>(null);
	const promptRef = useRef<HTMLTextAreaElement>(null);
	const setupRef = useRef<HTMLInputElement>(null);
	const teardownRef = useRef<HTMLInputElement>(null);
	const prefixRef = useRef<HTMLInputElement>(null);
	const wtRef = useRef<HTMLInputElement>(null);
	const imageRef = useRef<HTMLInputElement>(null);

	useEffect(() => {
		fetchCachedImages();
	}, []);

	function onSave(): void {
		const updated: Project = JSON.parse(JSON.stringify(p));
		updated.label = labelRef.current?.value.trim() || p.label;
		updated.directory = dirRef.current?.value.trim() || p.directory;
		updated.system_prompt = promptRef.current?.value.trim() || null;
		updated.setup_command = setupRef.current?.value.trim() || null;
		updated.teardown_command = teardownRef.current?.value.trim() || null;
		updated.branch_prefix = prefixRef.current?.value.trim() || null;
		updated.auto_worktree = wtRef.current?.checked;
		updated.sandbox_image = imageRef.current?.value.trim() || null;
		updated.updated_at = Date.now();
		sendRpc("projects.upsert", updated).then(() => {
			editingProject.value = null;
			fetchProjects();
		});
	}

	function field(
		label: string,
		ref: { current: HTMLInputElement | null },
		value: string | null | undefined,
		placeholder: string,
		mono?: boolean,
	): VNode {
		return (
			<div className="project-edit-group">
				<div className="text-xs text-[var(--muted)] project-edit-label">{label}</div>
				<input
					ref={ref}
					type="text"
					className="provider-key-input"
					value={value || ""}
					placeholder={placeholder || ""}
					style={mono ? { fontFamily: "var(--font-mono)", width: "100%" } : { width: "100%" }}
				/>
			</div>
		);
	}

	return (
		<div className="project-edit-form">
			{field(t("projects:editForm.label"), labelRef, p.label, t("projects:editForm.labelPlaceholder"))}
			{field(t("projects:editForm.directory"), dirRef, p.directory, t("projects:editForm.directoryPlaceholder"), true)}
			<div className="project-edit-group">
				<div className="text-xs text-[var(--muted)] project-edit-label">{t("projects:editForm.systemPrompt")}</div>
				<textarea
					ref={promptRef}
					className="provider-key-input"
					placeholder={t("projects:editForm.systemPromptPlaceholder")}
					style={{ width: "100%", minHeight: "60px", resize: "vertical", fontSize: ".8rem" }}
				>
					{p.system_prompt || ""}
				</textarea>
			</div>
			{field(
				t("projects:editForm.setupCommand"),
				setupRef,
				p.setup_command,
				t("projects:editForm.setupCommandPlaceholder"),
				true,
			)}
			{field(
				t("projects:editForm.teardownCommand"),
				teardownRef,
				p.teardown_command,
				t("projects:editForm.teardownCommandPlaceholder"),
				true,
			)}
			{field(
				t("projects:editForm.branchPrefix"),
				prefixRef,
				p.branch_prefix,
				t("projects:editForm.branchPrefixPlaceholder"),
				true,
			)}
			<div className="project-edit-group">
				<div className="text-xs text-[var(--muted)] project-edit-label">{t("projects:editForm.sandboxImage")}</div>
				<input
					ref={imageRef}
					type="text"
					className="provider-key-input"
					list="project-image-list"
					value={p.sandbox_image || ""}
					placeholder={t("projects:editForm.sandboxImagePlaceholder")}
					style={{ width: "100%", fontFamily: "var(--font-mono)", fontSize: ".8rem" }}
				/>
				<datalist id="project-image-list">
					{cachedImages.value.map((img) => (
						<option key={img.tag} value={img.tag} />
					))}
				</datalist>
			</div>
			<label style={{ marginBottom: "10px", display: "flex", alignItems: "center", gap: "8px", cursor: "pointer" }}>
				<input ref={wtRef} type="checkbox" defaultChecked={p.auto_worktree} />
				<span className="text-xs text-[var(--text)]">{t("projects:editForm.autoWorktree")}</span>
			</label>
			<div style={{ display: "flex", gap: "8px" }}>
				<button className="provider-btn" onClick={onSave}>
					{t("common:actions.save")}
				</button>
				<button
					className="provider-btn provider-btn-secondary"
					onClick={() => {
						editingProject.value = null;
					}}
				>
					{t("common:actions.cancel")}
				</button>
			</div>
		</div>
	);
}

function ProjectCard(props: ProjectCardProps): VNode {
	const p = props.project;

	function onDelete(): void {
		sendRpc("projects.delete", { id: p.id }).then(() => fetchProjects());
	}

	return (
		<div className="provider-item" style={{ marginBottom: "6px" }}>
			<div style={{ flex: 1, minWidth: 0 }}>
				<div className="flex items-center gap-2">
					<div className="provider-item-name">{p.label || p.id}</div>
					{p.detected && <span className="provider-item-badge api-key">{t("projects:badges.auto")}</span>}
					{p.auto_worktree && <span className="provider-item-badge oauth">{t("projects:badges.worktree")}</span>}
					{p.setup_command && <span className="provider-item-badge api-key">{t("projects:badges.setup")}</span>}
					{p.teardown_command && <span className="provider-item-badge api-key">{t("projects:badges.teardown")}</span>}
					{p.branch_prefix && <span className="provider-item-badge oauth">{p.branch_prefix}/*</span>}
					{p.sandbox_image && (
						<span className="provider-item-badge api-key" title={p.sandbox_image}>
							{t("projects:badges.image")}
						</span>
					)}
				</div>
				<div
					style={{
						fontSize: ".72rem",
						color: "var(--muted)",
						fontFamily: "var(--font-mono)",
						whiteSpace: "nowrap",
						overflow: "hidden",
						textOverflow: "ellipsis",
						marginTop: "2px",
					}}
				>
					{p.directory}
				</div>
				{p.system_prompt && (
					<div style={{ fontSize: ".7rem", color: "var(--muted)", marginTop: "2px", fontStyle: "italic" }}>
						{t("projects:card.systemPromptPrefix")}
						{p.system_prompt.substring(0, 80)}
						{p.system_prompt.length > 80 ? "..." : ""}
					</div>
				)}
			</div>
			<div style={{ display: "flex", gap: "4px", flexShrink: 0 }}>
				<button
					className="session-action-btn"
					title={t("projects:card.editProject")}
					onClick={() => {
						editingProject.value = p.id;
					}}
				>
					{t("projects:card.edit")}
				</button>
				<button
					className="session-action-btn session-delete"
					title={t("projects:card.removeProject")}
					onClick={onDelete}
				>
					x
				</button>
			</div>
		</div>
	);
}

function ProjectsPageComponent(): VNode {
	useEffect(() => {
		sendRpc("projects.list", {}).then((res) => {
			if (res?.ok) S.setProjects((res.payload as Project[]) || []);
		});
	}, []);

	function onAdd(dir: string): Promise<void> {
		return sendRpc("projects.detect", { directories: [dir] }).then((res) => {
			if (res?.ok) {
				const detected = (res.payload as Project[]) || [];
				if (detected.length === 0) {
					const slug = dir.split("/").filter(Boolean).pop() || "project";
					const now = Date.now();
					return sendRpc("projects.upsert", {
						id: slug.toLowerCase().replace(/[^a-z0-9-]/g, "-"),
						label: slug,
						directory: dir,
						auto_worktree: false,
						detected: false,
						created_at: now,
						updated_at: now,
					}).then(() => fetchProjects()) as Promise<void>;
				}
				fetchProjects();
			}
		});
	}

	function onDetect(): void {
		detecting.value = true;
		sendRpc("projects.detect", { directories: [] }).then(() => {
			detecting.value = false;
			fetchProjects();
		});
	}

	function onClearAll(): void {
		if (clearing.value) return;
		requestConfirm(t("projects:confirmClearAll"), {
			confirmLabel: t("projects:confirmClearAllButton"),
			danger: true,
		}).then((yes) => {
			if (!yes) return;
			const ids = projectsSig.value.map((p) => p.id);
			if (ids.length === 0) return;
			clearing.value = true;
			let chain = Promise.resolve();
			for (const id of ids) {
				chain = chain.then(() => {
					sendRpc("projects.delete", { id: id });
				});
			}
			chain
				.then(() => fetchProjects())
				.finally(() => {
					clearing.value = false;
				});
		});
	}

	const list = projectsSig.value as Project[];
	const clearDisabled = clearing.value || detecting.value || list.length === 0;

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<div className="flex items-center gap-3">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">{t("projects:title")}</h2>
				<button
					className="provider-btn provider-btn-secondary"
					onClick={onDetect}
					disabled={detecting.value}
					title={t("projects:autoDetectTooltip")}
				>
					{detecting.value ? t("projects:detecting") : t("projects:autoDetect")}
				</button>
				<button
					className="provider-btn provider-btn-danger"
					onClick={onClearAll}
					disabled={clearDisabled}
					title={t("projects:clearAllTooltip")}
				>
					{clearing.value ? t("projects:clearing") : t("projects:clearAll")}
				</button>
			</div>
			<p className="text-xs text-[var(--muted)] max-w-form">
				Clear All only removes repository entries from Moltis, it does not delete anything from disk.
			</p>
			<p className="text-sm text-[var(--muted)]" style={{ maxWidth: "600px", margin: 0 }}>
				Projects bind sessions to a codebase directory. When a session is linked to a project, context files (CLAUDE.md,
				AGENTS.md, .cursorrules, and rule directories) are loaded automatically, scanned for risky prompt-injection
				patterns, and injected into the system prompt. Enable auto-worktree to give each session its own git branch for
				isolated work.
			</p>
			<p className="text-sm text-[var(--muted)]" style={{ maxWidth: "600px", margin: 0 }}>
				<strong className="text-[var(--text)]">Auto-detect</strong> scans common directories under your home folder (
				<code className="font-mono text-xs">~/Projects</code>, <code className="font-mono text-xs">~/Developer</code>,{" "}
				<code className="font-mono text-xs">~/src</code>, <code className="font-mono text-xs">~/code</code>,{" "}
				<code className="font-mono text-xs">~/repos</code>, <code className="font-mono text-xs">~/workspace</code>,{" "}
				<code className="font-mono text-xs">~/dev</code>, <code className="font-mono text-xs">~/git</code>) and Superset
				worktrees (<code className="font-mono text-xs">~/.superset/worktrees</code>) for git repositories and adds them
				as projects.
			</p>
			<div className="project-form-row">
				<PathInput onAdd={onAdd} />
			</div>
			<div style={{ maxWidth: "600px", marginTop: "8px" }}>
				{list.length === 0 && (
					<div className="text-sm text-[var(--muted)]" style={{ padding: "12px 0" }}>
						No projects configured. Add a directory above or use auto-detect.
					</div>
				)}
				{list.map((p) =>
					editingProject.value === p.id ? (
						<ProjectEditForm key={p.id} project={p} />
					) : (
						<ProjectCard key={p.id} project={p} />
					),
				)}
			</div>
			<ConfirmDialog />
		</div>
	);
}

export function initProjects(container: HTMLElement): void {
	_projectsContainer = container;
	container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";
	editingProject.value = null;
	completions.value = [];
	detecting.value = false;
	clearing.value = false;
	render(<ProjectsPageComponent />, container);
}

export function teardownProjects(): void {
	if (_projectsContainer) render(null, _projectsContainer);
	_projectsContainer = null;
}

registerPage(routes.projects!, initProjects, teardownProjects);

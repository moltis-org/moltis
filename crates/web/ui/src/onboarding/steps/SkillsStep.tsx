// ── Skills step (bundled categories, repositories, ClawHub) ───
//
// Tabbed onboarding step letting users:
// 1. Toggle bundled skill categories
// 2. Install featured or custom GitHub skill repos
// 3. Search and install ClawHub community skills

import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { TabBar } from "../../components/forms/Tabs";
import { sendRpc } from "../../helpers";
import { t } from "../../i18n";
import { ClawHubSection } from "../../pages/skills/ClawHubSection";
import { type BundledCategory, CATEGORY_META, categoryLabel } from "../../types/skill-source";
import { showToast } from "../../ui";

// ── Types ────────────────────────────────────────────────────

interface FeaturedRepo {
	repo: string;
	desc: string;
	hasRecipe?: boolean;
}

interface RepoSummary {
	source: string;
	skill_count: number;
	enabled_count: number;
}

const FEATURED: FeaturedRepo[] = [
	{ repo: "anthropics/skills", desc: "Official Anthropic agent skills" },
	{ repo: "vercel-labs/agent-skills", desc: "Vercel agent skills collection" },
	{ repo: "vercel-labs/skills", desc: "Vercel skills toolkit" },
	{ repo: "garrytan/gbrain", desc: "Knowledge graph with hybrid search for agent memory", hasRecipe: true },
];

// ── Categories tab ───────────────────────────────────────────

function CategoriesTab(): VNode {
	const [categories, setCategories] = useState<BundledCategory[]>([]);
	const [totalSkills, setTotalSkills] = useState(0);
	const [loading, setLoading] = useState(true);
	const [busy, setBusy] = useState(false);

	useEffect(() => {
		sendRpc("skills.bundled.categories", {}).then((res) => {
			if (res?.ok) {
				const payload = res.payload as { categories?: BundledCategory[]; total_skills?: number };
				setCategories(payload.categories || []);
				setTotalSkills(payload.total_skills || 0);
			}
			setLoading(false);
		});
	}, []);

	function toggle(cat: BundledCategory): void {
		if (busy) return;
		const newEnabled = !cat.enabled;
		setBusy(true);
		sendRpc("skills.bundled.toggle_category", { category: cat.name, enabled: newEnabled }).then((res) => {
			setBusy(false);
			if (res?.ok) {
				setCategories((prev) => prev.map((c) => (c.name === cat.name ? { ...c, enabled: newEnabled } : c)));
			}
		});
	}

	function bulkToggle(enabled: boolean): void {
		const targets = categories.filter((c) => c.enabled !== enabled);
		if (!targets.length || busy) return;
		setBusy(true);
		Promise.all(
			targets.map((c) =>
				sendRpc("skills.bundled.toggle_category", { category: c.name, enabled }).then((res) => ({
					name: c.name,
					ok: !!res?.ok,
				})),
			),
		).then((results) => {
			setBusy(false);
			const succeeded = new Set(results.filter((r) => r.ok).map((r) => r.name));
			if (succeeded.size > 0) {
				setCategories((prev) => prev.map((c) => (succeeded.has(c.name) ? { ...c, enabled } : c)));
			}
		});
	}

	const enabledCount = categories.filter((c) => c.enabled).length;
	const enabledSkillCount = categories.filter((c) => c.enabled).reduce((sum, c) => sum + c.count, 0);

	if (loading) {
		return (
			<div className="flex items-center justify-center gap-2 py-8">
				<div className="inline-block w-5 h-5 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin" />
				<span className="text-sm text-[var(--muted)]">{t("common:status.loading")}</span>
			</div>
		);
	}

	return (
		<>
			<div className="flex items-center justify-between">
				<span className="text-xs text-[var(--muted)]">
					{enabledCount} of {categories.length} categories ({enabledSkillCount} of {totalSkills} skills)
				</span>
				<div className="flex gap-2">
					<button
						type="button"
						className="text-xs text-[var(--accent)] hover:underline cursor-pointer bg-transparent border-none p-0"
						disabled={busy}
						onClick={() => bulkToggle(true)}
					>
						{t("onboarding:skills.enableAll")}
					</button>
					<span className="text-xs text-[var(--muted)]">/</span>
					<button
						type="button"
						className="text-xs text-[var(--accent)] hover:underline cursor-pointer bg-transparent border-none p-0"
						disabled={busy}
						onClick={() => bulkToggle(false)}
					>
						{t("onboarding:skills.disableAll")}
					</button>
				</div>
			</div>

			<div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
				{categories.map((cat) => {
					const meta = CATEGORY_META[cat.name];
					const icon = meta?.icon || "\uD83D\uDCE6";
					const desc = meta?.desc || "";
					return (
						<button
							key={cat.name}
							type="button"
							onClick={() => toggle(cat)}
							disabled={busy}
							className={`flex items-start gap-3 p-3 rounded-md border text-left cursor-pointer transition-colors ${
								cat.enabled
									? "border-[var(--accent)] bg-[var(--accent-bg,rgba(var(--accent-rgb,59,130,246),0.08))]"
									: "border-[var(--border)] bg-[var(--surface)] opacity-60"
							}`}
						>
							<span className="text-lg shrink-0 mt-0.5">{icon}</span>
							<div className="flex-1 min-w-0">
								<div className="flex items-center gap-2">
									<span className="text-sm font-medium text-[var(--text-strong)]">{categoryLabel(cat.name)}</span>
									<span className="text-xs text-[var(--muted)]">({cat.count})</span>
								</div>
								{desc && <div className="text-xs text-[var(--muted)] mt-0.5">{desc}</div>}
							</div>
							<div className="shrink-0 mt-1">
								{cat.enabled ? (
									<span className="icon icon-check-circle text-[var(--accent)]" />
								) : (
									<span className="w-4 h-4 rounded-full border-2 border-[var(--border)] inline-block" />
								)}
							</div>
						</button>
					);
				})}
			</div>
		</>
	);
}

// ── Repositories tab ─────────────────────────────────────────

function RepositoriesTab(): VNode {
	const installedRepos = useSignal<RepoSummary[]>([]);
	const installingRepo = useSignal<string | null>(null);
	const inputRef = useRef<HTMLInputElement>(null);

	useEffect(() => {
		fetchRepos();
	}, []);

	function fetchRepos(): void {
		fetch("/api/skills")
			.then((r) => r.json())
			.then((data) => {
				if (data.repos) installedRepos.value = data.repos;
			})
			.catch(console.error);
	}

	function isInstalled(source: string): boolean {
		return installedRepos.value.some((r) => r.source === source);
	}

	async function installRepo(source: string): Promise<void> {
		installingRepo.value = source;
		const res = await sendRpc("skills.install", { source });
		installingRepo.value = null;
		if (res?.ok) {
			showToast(`Installed ${source}`, "success");
			fetchRepos();
		} else {
			showToast(`Failed: ${res?.error?.message || "unknown"}`, "error");
		}
	}

	function installCustom(): void {
		const v = inputRef.current?.value.trim();
		if (!v) return;
		installRepo(v).then(() => {
			if (inputRef.current) inputRef.current.value = "";
		});
	}

	function orgAvatarUrl(source: string): string {
		const org = source.split("/")[0];
		return `https://github.com/${org}.png?size=48`;
	}

	return (
		<div className="flex flex-col gap-4">
			<div className="skills-install-box">
				<input
					ref={inputRef}
					type="text"
					placeholder="owner/repo or full URL (e.g. anthropics/skills)"
					className="skills-install-input"
					onKeyDown={(e) => {
						if ((e as KeyboardEvent).key === "Enter") installCustom();
					}}
				/>
				<button type="button" className="provider-btn" onClick={installCustom} disabled={installingRepo.value !== null}>
					{installingRepo.value !== null ? "Installing\u2026" : "Install"}
				</button>
			</div>

			<div>
				<h3 className="text-sm font-medium text-[var(--text-strong)] mb-2">Featured Repositories</h3>
				<div className="skills-featured-grid">
					{FEATURED.map((f) => {
						const installed = isInstalled(f.repo);
						const busy = installingRepo.value === f.repo;
						return (
							<div key={f.repo} className="skills-featured-card">
								<img
									src={orgAvatarUrl(f.repo)}
									alt=""
									style={{ width: "24px", height: "24px", borderRadius: "var(--radius-sm)", flexShrink: 0 }}
								/>
								<div style={{ flex: 1, minWidth: 0 }}>
									<a
										href={`https://github.com/${f.repo}`}
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
									type="button"
									onClick={() => {
										if (!(installed || busy)) installRepo(f.repo).catch(console.error);
									}}
									disabled={installed || busy}
									style={{
										background: "var(--surface2)",
										border: "1px solid var(--border)",
										color: installed ? "var(--success, #22c55e)" : "var(--text)",
										borderRadius: "var(--radius-sm)",
										fontSize: ".72rem",
										padding: "4px 10px",
										cursor: installed ? "default" : "pointer",
										whiteSpace: "nowrap",
										opacity: installed ? 0.8 : 1,
									}}
								>
									{installed ? "Installed" : busy ? "Installing\u2026" : "Install"}
								</button>
							</div>
						);
					})}
				</div>
			</div>
		</div>
	);
}

// ── Main SkillsStep ──────────────────────────────────────────

const TABS = [
	{ id: "categories", label: "Categories" },
	{ id: "repositories", label: "Repositories" },
	{ id: "clawhub", label: "ClawHub" },
];

export function SkillsStep({ onNext, onBack }: { onNext: () => void; onBack?: (() => void) | null }): VNode {
	const [activeTab, setActiveTab] = useState("categories");

	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">{t("onboarding:skills.title")}</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed">{t("onboarding:skills.description")}</p>

			<TabBar tabs={TABS} active={activeTab} onChange={setActiveTab} />

			{activeTab === "categories" && <CategoriesTab />}
			{activeTab === "repositories" && <RepositoriesTab />}
			{activeTab === "clawhub" && <ClawHubSection onChanged={() => {}} />}

			<div className="flex flex-wrap items-center gap-3 mt-1">
				{onBack && (
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onBack}>
						{t("common:actions.back")}
					</button>
				)}
				<div className="flex-1" />
				<button type="button" className="provider-btn" onClick={onNext}>
					{t("common:actions.continue")}
				</button>
			</div>
		</div>
	);
}

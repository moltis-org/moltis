// ── ClawHub skill search and install ─────────────────────────
import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { useRef } from "preact/hooks";
import { sendRpc } from "../../helpers";

// ── Types ────────────────────────────────────────────────────

interface ClawHubResult {
	score: number;
	slug: string;
	displayName?: string;
	summary?: string;
	updatedAt?: string;
	downloads?: number;
	ownerHandle?: string;
	verified?: boolean;
}

// ── Helpers ──────────────────────────────────────────────────

let searchTimer: ReturnType<typeof setTimeout> | null = null;

// ── Components ───────────────────────────────────────────────

function ResultCard({ result, onInstalled }: { result: ClawHubResult; onInstalled: () => void }): VNode {
	const installing = useSignal(false);
	const installed = useSignal(false);
	const error = useSignal<string | null>(null);

	async function doInstall(): Promise<void> {
		installing.value = true;
		error.value = null;
		const res = await sendRpc("skills.clawhub.install", { slug: result.slug });
		installing.value = false;
		if (res?.ok) {
			installed.value = true;
			// Auto-trust and enable the installed skill.
			const payload = res.payload as { installed?: Array<{ name?: string }> } | undefined;
			const skills = payload?.installed || [];
			const source = `clawhub:${result.slug}`;
			for (const skill of skills) {
				if (!skill.name) continue;
				await sendRpc("skills.skill.trust", { source, skill: skill.name, trusted: true });
				await sendRpc("skills.skill.enable", { source, skill: skill.name, enabled: true });
			}
			onInstalled();
		} else {
			error.value = String(res?.error || "Install failed");
		}
	}

	return (
		<div className="skills-featured-card">
			<div style={{ flex: 1, minWidth: 0 }}>
				<div style={{ display: "flex", alignItems: "center", gap: "6px", flexWrap: "wrap" }}>
					<span
						style={{
							fontFamily: "var(--font-mono)",
							fontSize: ".82rem",
							fontWeight: 500,
							color: "var(--text-strong)",
						}}
					>
						{result.displayName || result.slug}
					</span>
					{result.slug !== result.displayName && (
						<span style={{ fontSize: ".68rem", color: "var(--muted)" }}>{result.slug}</span>
					)}
					{result.verified && (
						<span
							style={{
								fontSize: ".62rem",
								padding: "1px 4px",
								borderRadius: "var(--radius-sm)",
								background: "var(--success-bg, rgba(34,197,94,.12))",
								color: "var(--success, #22c55e)",
							}}
						>
							verified
						</span>
					)}
					{result.ownerHandle && (
						<span style={{ fontSize: ".68rem", color: "var(--muted)" }}>by {result.ownerHandle}</span>
					)}
					{result.downloads != null && result.downloads > 0 && (
						<span style={{ fontSize: ".68rem", color: "var(--muted)" }}>
							{result.downloads.toLocaleString()} downloads
						</span>
					)}
				</div>
				{result.summary && (
					<div
						style={{
							fontSize: ".75rem",
							color: "var(--muted)",
							overflow: "hidden",
							textOverflow: "ellipsis",
							whiteSpace: "nowrap",
						}}
					>
						{result.summary}
					</div>
				)}
				{error.value && (
					<div style={{ fontSize: ".72rem", color: "var(--danger, #ef4444)", marginTop: "2px" }}>{error.value}</div>
				)}
			</div>
			<button
				onClick={() => {
					if (!(installed.value || installing.value)) doInstall().catch(console.error);
				}}
				disabled={installed.value || installing.value}
				style={{
					background: "var(--surface2)",
					border: "1px solid var(--border)",
					color: installed.value ? "var(--success, #22c55e)" : "var(--text)",
					borderRadius: "var(--radius-sm)",
					fontSize: ".72rem",
					padding: "4px 10px",
					cursor: installed.value ? "default" : "pointer",
					whiteSpace: "nowrap",
					opacity: installed.value ? 0.8 : 1,
				}}
			>
				{installed.value ? "Installed" : installing.value ? "Installing\u2026" : "Install"}
			</button>
		</div>
	);
}

export function ClawHubSection({ onChanged }: { onChanged: () => void }): VNode {
	const query = useSignal("");
	const results = useSignal<ClawHubResult[]>([]);
	const searching = useSignal(false);
	const searched = useSignal(false);
	const inputRef = useRef<HTMLInputElement>(null);

	function doSearch(q: string): void {
		if (!q.trim()) {
			results.value = [];
			searched.value = false;
			return;
		}
		searching.value = true;
		sendRpc("skills.clawhub.search", { query: q.trim() }).then((res) => {
			searching.value = false;
			searched.value = true;
			if (res?.ok) {
				const payload = res.payload as { results?: ClawHubResult[] } | undefined;
				results.value = payload?.results || [];
			}
		});
	}

	function onInput(e: Event): void {
		const v = (e.target as HTMLInputElement).value;
		query.value = v;
		if (searchTimer) clearTimeout(searchTimer);
		if (!v.trim()) {
			results.value = [];
			searched.value = false;
			return;
		}
		searchTimer = setTimeout(() => doSearch(v), 300);
	}

	function onKeyDown(e: Event): void {
		if ((e as KeyboardEvent).key === "Enter") {
			if (searchTimer) clearTimeout(searchTimer);
			doSearch(query.value);
		}
	}

	return (
		<div className="skills-section">
			<h3 className="skills-section-title">
				ClawHub
				<span style={{ fontSize: ".72rem", color: "var(--muted)", fontWeight: 400, marginLeft: "8px" }}>
					clawhub.ai — 52k+ community skills
				</span>
			</h3>
			<div className="skills-install-box">
				<input
					ref={inputRef}
					type="text"
					placeholder="Search ClawHub skills..."
					className="skills-install-input"
					value={query.value}
					onInput={onInput}
					onKeyDown={onKeyDown}
				/>
				<button
					className="provider-btn"
					disabled={searching.value || !query.value.trim()}
					onClick={() => doSearch(query.value)}
				>
					{searching.value ? "Searching\u2026" : "Search"}
				</button>
			</div>
			{results.value.length > 0 && (
				<div style={{ display: "flex", flexDirection: "column", gap: "4px", marginTop: "8px" }}>
					{results.value.map((r) => (
						<ResultCard key={r.slug} result={r} onInstalled={onChanged} />
					))}
				</div>
			)}
			{searched.value && results.value.length === 0 && !searching.value && (
				<div style={{ fontSize: ".78rem", color: "var(--muted)", padding: "12px 0", textAlign: "center" }}>
					No skills found. Try a different search term.
				</div>
			)}
		</div>
	);
}

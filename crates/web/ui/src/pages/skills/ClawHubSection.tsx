// ── ClawHub skill search, detail, and install ────────────────
import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { useEffect, useRef } from "preact/hooks";
import { sendRpc } from "../../helpers";
import { showToast } from "../../ui";

// ── Types ────────────────────────────────────────────────────

interface ClawHubResult {
	score: number;
	slug: string;
	displayName?: string;
	summary?: string;
	updatedAt?: number;
	version?: string;
	downloads?: number;
	ownerHandle?: string;
	ownerImage?: string;
	installRef?: string;
	stars?: number;
}

interface ScanData {
	status?: string;
	hasWarnings?: boolean;
	virustotalUrl?: string;
	scanners?: {
		vt?: { verdict?: string; analysis?: string };
		llm?: { verdict?: string; analysis?: string };
	};
}

interface ClawHubSkillInfo {
	skill: {
		slug: string;
		displayName?: string;
		summary?: string;
		stats?: { downloads?: number; installsAllTime?: number; stars?: number };
	};
	latestVersion?: { version: string; changelog?: string; license?: string };
	owner?: { handle?: string; displayName?: string; image?: string };
	moderation?: { isSuspicious?: boolean; verdict?: string } | null;
}

// ── Helpers ──────────────────────────────────────────────────

const CLAWHUB_SEARCH_TIMEOUT_MS = 20_000;

function relativeTime(ms: number): string {
	const diff = Date.now() - ms;
	const mins = Math.floor(diff / 60000);
	if (mins < 60) return `${mins}m ago`;
	const hours = Math.floor(mins / 60);
	if (hours < 24) return `${hours}h ago`;
	const days = Math.floor(hours / 24);
	if (days < 30) return `${days}d ago`;
	const months = Math.floor(days / 30);
	if (months < 12) return `${months}mo ago`;
	return `${Math.floor(months / 12)}y ago`;
}

function fmtNumber(n: number): string {
	if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
	return n.toString();
}

// ── Security scan panel ──────────────────────────────────────

function SecurityScanPanel({ scan }: { scan: ScanData | null }): VNode | null {
	if (!scan) return null;

	const isClean = scan.status === "clean";
	const bg = isClean ? "var(--success-bg, rgba(34,197,94,.08))" : "var(--warning-bg, rgba(234,179,8,.12))";
	const vt = scan.scanners?.vt;
	const llm = scan.scanners?.llm;

	return (
		<div
			style={{
				marginTop: "8px",
				padding: "8px 10px",
				borderRadius: "var(--radius-sm)",
				background: bg,
				fontSize: ".72rem",
			}}
		>
			<div style={{ fontWeight: 600, marginBottom: "4px" }}>
				Security Scan: {isClean ? "Benign" : scan.status || "Unknown"}
				{!isClean && scan.hasWarnings && (
					<span style={{ color: "var(--warning, #eab308)", marginLeft: "6px" }}>(has warnings)</span>
				)}
			</div>
			{vt && (
				<div style={{ display: "flex", alignItems: "center", gap: "6px", marginBottom: "2px" }}>
					<span style={{ fontWeight: 500 }}>VirusTotal:</span>
					<span style={{ color: vt.verdict === "benign" ? "var(--success, #22c55e)" : "var(--warning, #eab308)" }}>
						{vt.verdict || "unknown"}
					</span>
					{scan.virustotalUrl && (
						<a
							href={scan.virustotalUrl}
							target="_blank"
							rel="noopener noreferrer"
							style={{ color: "var(--accent)", fontSize: ".68rem" }}
						>
							view report
						</a>
					)}
				</div>
			)}
			{llm && (
				<div style={{ display: "flex", alignItems: "center", gap: "6px" }}>
					<span style={{ fontWeight: 500 }}>AI Analysis:</span>
					<span style={{ color: llm.verdict === "benign" ? "var(--success, #22c55e)" : "var(--warning, #eab308)" }}>
						{llm.verdict || "unknown"}
					</span>
				</div>
			)}
		</div>
	);
}

// ── Detail panel ─────────────────────────────────────────────

function DetailPanel({
	slug,
	reference,
	onClose,
	onInstalled,
}: {
	slug: string;
	reference: string;
	onClose: () => void;
	onInstalled: () => void;
}): VNode {
	const info = useSignal<ClawHubSkillInfo | null>(null);
	const scan = useSignal<ScanData | null>(null);
	const loading = useSignal(true);
	const error = useSignal<string | null>(null);
	const installing = useSignal(false);
	const installed = useSignal(false);

	// Fetch info + scan in parallel on mount (once only).
	const fetched = useRef(false);
	useEffect(() => {
		if (fetched.current) return;
		fetched.current = true;
		Promise.all([sendRpc("skills.clawhub.info", { reference }), sendRpc("skills.clawhub.scan", { reference })])
			.then(([infoRes, scanRes]) => {
				loading.value = false;
				if (infoRes?.ok) info.value = infoRes.payload as ClawHubSkillInfo;
				else error.value = infoRes?.error?.message || "Failed to load skill info";
				if (scanRes?.ok) {
					const p = scanRes.payload as { security?: ScanData } | undefined;
					if (p?.security) scan.value = p.security;
				}
			})
			.catch(() => {
				loading.value = false;
				error.value = "Failed to load skill info";
			});
	}, [reference]);

	async function doInstall(): Promise<void> {
		installing.value = true;
		try {
			const res = await sendRpc("skills.clawhub.install", { reference });
			if (res?.ok) {
				installed.value = true;
				showToast("Installed — review and enable the skill in the Skills tab.", "success");
				onInstalled();
			} else {
				error.value = res?.error?.message || "Install failed";
			}
		} finally {
			installing.value = false;
		}
	}

	const d = info.value;
	const s = d?.skill;
	const stats = s?.stats;
	const owner = d?.owner;
	const ver = d?.latestVersion;

	return (
		<div
			style={{
				border: "1px solid var(--border)",
				borderRadius: "var(--radius-sm)",
				background: "var(--surface)",
				padding: "12px 14px",
				marginTop: "8px",
			}}
		>
			<div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start" }}>
				<div style={{ flex: 1, minWidth: 0 }}>
					{loading.value && <div style={{ color: "var(--muted)", fontSize: ".78rem" }}>Loading...</div>}
					{error.value && <div style={{ color: "var(--danger, #ef4444)", fontSize: ".78rem" }}>{error.value}</div>}
					{s && (
						<>
							<div style={{ display: "flex", alignItems: "center", gap: "8px", flexWrap: "wrap" }}>
								{owner?.image && (
									<img src={owner.image} alt="" style={{ width: "28px", height: "28px", borderRadius: "50%" }} />
								)}
								<div>
									{owner?.handle && (
										<span style={{ fontSize: ".72rem", color: "var(--muted)" }}>@{owner.handle} / </span>
									)}
									<span
										style={{
											fontSize: ".92rem",
											fontWeight: 600,
											color: "var(--text-strong)",
										}}
									>
										{s.displayName || s.slug}
									</span>
									{ver && (
										<span
											style={{
												fontSize: ".68rem",
												padding: "1px 5px",
												marginLeft: "6px",
												borderRadius: "var(--radius-sm)",
												background: "var(--surface2)",
												color: "var(--muted)",
											}}
										>
											v{ver.version}
										</span>
									)}
								</div>
							</div>
							{s.summary && <p style={{ fontSize: ".78rem", color: "var(--muted)", margin: "6px 0" }}>{s.summary}</p>}

							{/* Stats row */}
							<div
								style={{ display: "flex", gap: "10px", fontSize: ".72rem", color: "var(--muted)", marginTop: "6px" }}
							>
								{stats?.stars != null && stats.stars > 0 && <span>&#11088; {stats.stars}</span>}
								{stats?.downloads != null && stats.downloads > 0 && <span>{fmtNumber(stats.downloads)} downloads</span>}
								{stats?.installsAllTime != null && stats.installsAllTime > 0 && (
									<span>{stats.installsAllTime} installs</span>
								)}
							</div>

							{/* Security scan */}
							<SecurityScanPanel scan={scan.value} />

							{/* Changelog */}
							{ver?.changelog && (
								<div style={{ marginTop: "8px" }}>
									<div
										style={{ fontSize: ".72rem", fontWeight: 600, color: "var(--text-strong)", marginBottom: "2px" }}
									>
										Changelog (v{ver.version})
									</div>
									<div
										style={{
											fontSize: ".72rem",
											color: "var(--muted)",
											whiteSpace: "pre-wrap",
											maxHeight: "100px",
											overflow: "auto",
										}}
									>
										{ver.changelog}
									</div>
								</div>
							)}

							{ver?.license && (
								<div style={{ marginTop: "4px", fontSize: ".68rem", color: "var(--muted)" }}>
									License: {ver.license}
								</div>
							)}
						</>
					)}
				</div>
				<div style={{ display: "flex", flexDirection: "column", gap: "4px", marginLeft: "12px", flexShrink: 0 }}>
					<button
						onClick={() => {
							if (!(installed.value || installing.value)) doInstall().catch(console.error);
						}}
						disabled={installed.value || installing.value || loading.value}
						className="provider-btn provider-btn-sm"
						style={{ minWidth: "80px" }}
					>
						{installed.value ? "Installed" : installing.value ? "Installing\u2026" : "Install"}
					</button>
					<button
						onClick={onClose}
						className="provider-btn provider-btn-sm provider-btn-secondary"
						style={{ minWidth: "80px" }}
					>
						Close
					</button>
					<a
						href={`https://clawhub.ai/${owner?.handle || "_"}/${slug}`}
						target="_blank"
						rel="noopener noreferrer"
						className="provider-btn provider-btn-sm provider-btn-secondary"
						style={{ minWidth: "80px", textAlign: "center", textDecoration: "none", display: "block" }}
					>
						ClawHub
					</a>
				</div>
			</div>
		</div>
	);
}

// ── Result card ──────────────────────────────────────────────

function ResultCard({ result, onInstalled }: { result: ClawHubResult; onInstalled: () => void }): VNode {
	const expanded = useSignal(false);

	return (
		<div>
			<div
				className="skills-featured-card"
				onClick={() => {
					expanded.value = !expanded.value;
				}}
				style={{ cursor: "pointer" }}
			>
				<div style={{ flex: 1, minWidth: 0 }}>
					<div style={{ display: "flex", alignItems: "center", gap: "6px", flexWrap: "wrap" }}>
						{result.ownerHandle && (
							<span style={{ fontSize: ".68rem", color: "var(--muted)" }}>@{result.ownerHandle} /</span>
						)}
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
						{result.updatedAt && (
							<span style={{ fontSize: ".65rem", color: "var(--muted)" }}>{relativeTime(result.updatedAt)}</span>
						)}
						{result.stars != null && result.stars > 0 && (
							<span style={{ fontSize: ".65rem", color: "var(--muted)" }}>&#11088; {result.stars}</span>
						)}
						{result.downloads != null && result.downloads > 0 && (
							<span style={{ fontSize: ".65rem", color: "var(--muted)" }}>{fmtNumber(result.downloads)}</span>
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
				</div>
				<span
					style={{
						fontSize: ".68rem",
						color: "var(--accent)",
						whiteSpace: "nowrap",
					}}
				>
					{expanded.value ? "Close" : "View"}
				</span>
			</div>
			{expanded.value && (
				<DetailPanel
					slug={result.slug}
					reference={result.installRef || result.slug}
					onInstalled={onInstalled}
					onClose={() => {
						expanded.value = false;
					}}
				/>
			)}
		</div>
	);
}

// ── Main section ─────────────────────────────────────────────

export function ClawHubSection({ onChanged }: { onChanged: () => void }): VNode {
	const query = useSignal("");
	const results = useSignal<ClawHubResult[]>([]);
	const searching = useSignal(false);
	const searched = useSignal(false);
	const error = useSignal<string | null>(null);
	const inputRef = useRef<HTMLInputElement>(null);
	const searchTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
	const searchGeneration = useRef(0);

	useEffect(
		() => () => {
			searchGeneration.current += 1;
			if (searchTimer.current) clearTimeout(searchTimer.current);
		},
		[],
	);

	function doSearch(q: string, generation: number): void {
		if (generation !== searchGeneration.current) return;
		const trimmedQuery = q.trim();
		if (!trimmedQuery) {
			results.value = [];
			searched.value = false;
			error.value = null;
			return;
		}
		searching.value = true;
		error.value = null;
		sendRpc("skills.clawhub.search", { query: trimmedQuery }, CLAWHUB_SEARCH_TIMEOUT_MS)
			.then((res) => {
				if (generation !== searchGeneration.current) return;
				searching.value = false;
				searched.value = true;
				if (res?.ok) {
					const payload = res.payload as { results?: ClawHubResult[] } | undefined;
					results.value = payload?.results || [];
					console.info("ClawHub search completed", { query: trimmedQuery, resultCount: results.value.length });
					return;
				}
				const message = res?.error?.message || "ClawHub search failed";
				results.value = [];
				error.value = message;
				console.error("ClawHub search failed", { query: trimmedQuery, error: res?.error });
				showToast(message, "error");
			})
			.catch((cause: unknown) => {
				if (generation !== searchGeneration.current) return;
				searching.value = false;
				searched.value = true;
				results.value = [];
				const message = cause instanceof Error ? cause.message : "ClawHub search failed";
				error.value = message;
				console.error("ClawHub search failed", { query: trimmedQuery, cause });
				showToast(message, "error");
			});
	}

	function searchNow(q: string): void {
		doSearch(q, ++searchGeneration.current);
	}

	function onInput(e: Event): void {
		const v = (e.target as HTMLInputElement).value;
		query.value = v;
		const generation = ++searchGeneration.current;
		searching.value = false;
		searched.value = false;
		results.value = [];
		error.value = null;
		if (searchTimer.current) clearTimeout(searchTimer.current);
		if (!v.trim()) {
			return;
		}
		searchTimer.current = setTimeout(() => doSearch(v, generation), 300);
	}

	function onKeyDown(e: Event): void {
		if ((e as KeyboardEvent).key === "Enter") {
			if (searchTimer.current) clearTimeout(searchTimer.current);
			searchNow(query.value);
		}
	}

	return (
		<div className="skills-section">
			<h3 className="skills-section-title">
				ClawHub
				<span
					style={{
						fontSize: ".72rem",
						color: "var(--muted)",
						fontWeight: 400,
						marginLeft: "8px",
					}}
				>
					52k+ community skills from{" "}
					<a href="https://clawhub.ai" target="_blank" rel="noopener noreferrer" style={{ color: "var(--accent)" }}>
						clawhub.ai
					</a>
				</span>
			</h3>
			<div className="skills-install-box">
				<input
					ref={inputRef}
					type="text"
					placeholder="Search ClawHub skills (e.g. csv, weather, github)..."
					className="skills-install-input"
					value={query.value}
					onInput={onInput}
					onKeyDown={onKeyDown}
				/>
				<button
					className="provider-btn"
					disabled={searching.value || !query.value.trim()}
					onClick={() => searchNow(query.value)}
				>
					{searching.value ? "Searching\u2026" : "Search"}
				</button>
			</div>
			{error.value && (
				<div
					role="alert"
					className="mt-2 rounded border border-[var(--danger)]/40 bg-[var(--danger)]/10 px-3 py-2 text-sm text-[var(--danger)]"
				>
					ClawHub search failed: {error.value}
				</div>
			)}
			{results.value.length > 0 && (
				<div style={{ display: "flex", flexDirection: "column", gap: "4px", marginTop: "8px" }}>
					{results.value.map((r) => (
						<ResultCard key={r.installRef || `${r.ownerHandle || ""}/${r.slug}`} result={r} onInstalled={onChanged} />
					))}
				</div>
			)}
			{searched.value && !error.value && results.value.length === 0 && !searching.value && (
				<div
					style={{
						fontSize: ".78rem",
						color: "var(--muted)",
						padding: "12px 0",
						textAlign: "center",
					}}
				>
					No skills found. Try a different search term.
				</div>
			)}
		</div>
	);
}

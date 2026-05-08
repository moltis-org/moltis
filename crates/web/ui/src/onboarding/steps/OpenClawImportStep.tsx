// ── OpenClaw import step (conditional) ────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { refresh as refreshGon } from "../../gon";
import { sendRpc } from "../../helpers";
import { ErrorPanel, ensureWsConnected } from "../shared";

// ── Constants ───────────────────────────────────────────────

const WS_RETRY_LIMIT = 75;
const WS_RETRY_DELAY_MS = 200;

// ── Types ───────────────────────────────────────────────────

interface ImportSelection {
	identity: boolean;
	providers: boolean;
	skills: boolean;
	memory: boolean;
	channels: boolean;
	sessions: boolean;
	workspace_files: boolean;
	[key: string]: boolean;
}

interface ScanResult {
	detected?: boolean;
	home_dir?: string;
	identity_available?: boolean;
	identity_agent_name?: string;
	identity_theme?: string;
	providers_available?: boolean;
	skills_count?: number;
	memory_available?: boolean;
	memory_files_count?: number;
	channels_available?: boolean;
	telegram_accounts?: number;
	discord_accounts?: number;
	unsupported_channels?: string[];
	sessions_count?: number;
	workspace_files_available?: boolean;
	workspace_files_found?: string[];
	agents?: Array<{
		openclaw_id: string;
		name?: string;
		is_default?: boolean;
		theme?: string;
	}>;
}

interface ImportResult {
	categories?: Array<{
		category: string;
		status: string;
		items_imported: number;
		items_skipped: number;
		warnings?: string[];
	}>;
	todos?: Array<{ feature: string; description: string }>;
}

interface CategoryDef {
	key: string;
	label: string;
	available: boolean;
	detail: string | null;
}

const IMPORT_CATEGORY_ICONS: Record<string, string> = {
	identity: "\uD83D\uDC64",
	providers: "\uD83D\uDD11",
	skills: "\u2728",
	memory: "\uD83E\uDDE0",
	channels: "\uD83D\uDCAC",
	sessions: "\uD83D\uDCBE",
	workspace_files: "\uD83D\uDCC1",
};

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: import step manages scan, selection, import, and result display
export function OpenClawImportStep({ onNext, onBack }: { onNext: () => void; onBack?: (() => void) | null }): VNode {
	const [loading, setLoading] = useState(true);
	const [scan, setScan] = useState<ScanResult | null>(null);
	const [importing, setImporting] = useState(false);
	const [done, setDone] = useState(false);
	const [result, setResult] = useState<ImportResult | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [selection, setSelection] = useState<ImportSelection>({
		identity: true,
		providers: true,
		skills: true,
		memory: true,
		channels: true,
		sessions: true,
		workspace_files: true,
	});

	useEffect(() => {
		let cancelled = false;
		let attempts = 0;
		let retryTimer: ReturnType<typeof setTimeout> | null = null;

		function loadScan(): void {
			if (cancelled) return;
			(
				sendRpc("openclaw.scan", {}) as Promise<{
					ok?: boolean;
					payload?: ScanResult;
					error?: { code?: string; message?: string };
				}>
			).then((res) => {
				if (cancelled) return;
				if (res?.ok) {
					setScan(res.payload || null);
					setLoading(false);
					return;
				}

				if (
					(res?.error?.code === "UNAVAILABLE" || res?.error?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
					attempts += 1;
					ensureWsConnected();
					retryTimer = setTimeout(loadScan, WS_RETRY_DELAY_MS);
					return;
				}

				setError(res?.error?.message || "Failed to scan OpenClaw installation");
				setLoading(false);
			});
		}

		ensureWsConnected();
		loadScan();
		return () => {
			cancelled = true;
			if (retryTimer) {
				clearTimeout(retryTimer);
				retryTimer = null;
			}
		};
	}, []);

	function toggleCategory(key: string): void {
		setSelection((prev) => {
			const next = { ...prev };
			next[key] = !prev[key];
			return next;
		});
	}

	async function doImport(): Promise<void> {
		setImporting(true);
		setError(null);
		const res = (await sendRpc("openclaw.import", selection)) as {
			ok?: boolean;
			payload?: ImportResult;
			error?: { message?: string };
		};
		setImporting(false);
		if (res?.ok) {
			setResult(res.payload || null);
			await refreshGon();
			setDone(true);
		} else {
			setError(res?.error?.message || "Import failed");
		}
	}

	if (loading) {
		return (
			<div className="flex flex-col items-center justify-center gap-3 min-h-[200px]">
				<div className="inline-block w-8 h-8 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin" />
				<div className="text-sm text-[var(--muted)]">Scanning OpenClaw installation&hellip;</div>
			</div>
		);
	}

	if (done && result) {
		const total = (result.categories || []).reduce((sum, cat) => sum + (Number(cat.items_imported) || 0), 0);
		return (
			<div className="flex flex-col gap-4">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Import Complete</h2>
				<p className="text-xs text-[var(--muted)] leading-relaxed">{total} item(s) imported from OpenClaw.</p>
				{result.categories ? (
					<div className="flex flex-col gap-1">
						{result.categories.map((cat) => (
							<div key={cat.category} className="text-xs text-[var(--text)]">
								<span className="font-mono">
									[
									{cat.status === "success"
										? "\u2713"
										: cat.status === "partial"
											? "~"
											: cat.status === "skipped"
												? "-"
												: "!"}
									]
								</span>{" "}
								{cat.category}: {cat.items_imported} imported, {cat.items_skipped} skipped
								{(cat.warnings || []).map((w) => (
									<div key={w} className="text-[var(--warn)] ml-6">
										{w}
									</div>
								))}
							</div>
						))}
					</div>
				) : null}
				{(result.todos?.length ?? 0) > 0 ? (
					<div className="text-xs text-[var(--muted)]">
						<div className="font-medium">Not yet supported in Moltis:</div>
						{(result.todos || []).map((td) => (
							<div key={td.feature}>
								&bull; {td.feature}: {td.description}
							</div>
						))}
					</div>
				) : null}
				<div className="flex flex-wrap items-center gap-3 mt-1">
					<button type="button" className="provider-btn" onClick={onNext}>
						Continue
					</button>
				</div>
			</div>
		);
	}

	if (!scan?.detected) {
		return (
			<div className="flex flex-col gap-4">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Import from OpenClaw</h2>
				<p className="text-xs text-[var(--muted)]">Could not scan OpenClaw installation.</p>
				<div className="flex flex-wrap items-center gap-3 mt-1">
					{onBack ? (
						<button type="button" className="provider-btn provider-btn-secondary" onClick={onBack}>
							Back
						</button>
					) : null}
					<button type="button" className="provider-btn" onClick={onNext}>
						Skip
					</button>
				</div>
			</div>
		);
	}

	const telegramAccounts = Number(scan.telegram_accounts) || 0;
	const discordAccounts = Number(scan.discord_accounts) || 0;
	const channelParts: string[] = [];
	if (telegramAccounts > 0) channelParts.push(`${telegramAccounts} Telegram account(s)`);
	if (discordAccounts > 0) channelParts.push(`${discordAccounts} Discord account(s)`);
	const channelDetail = channelParts.length > 0 ? channelParts.join(", ") : null;
	const unsupportedChannels = (scan.unsupported_channels || []).filter(
		(channel) => String(channel).toLowerCase() !== "discord",
	);

	const categories: CategoryDef[] = [
		{
			key: "identity",
			label: "Identity",
			available: !!scan.identity_available,
			detail: [scan.identity_agent_name, scan.identity_theme].filter(Boolean).join(", ") || null,
		},
		{
			key: "providers",
			label: "Providers",
			available: !!scan.providers_available,
			detail: null,
		},
		{
			key: "skills",
			label: "Skills",
			available: (scan.skills_count ?? 0) > 0,
			detail: `${scan.skills_count} skill(s)`,
		},
		{
			key: "memory",
			label: "Memory",
			available: !!scan.memory_available,
			detail: `${scan.memory_files_count} memory file(s)`,
		},
		{
			key: "channels",
			label: "Channels",
			available: !!scan.channels_available,
			detail: channelDetail,
		},
		{
			key: "sessions",
			label: "Sessions",
			available: (scan.sessions_count ?? 0) > 0,
			detail: `${scan.sessions_count} session(s)`,
		},
		{
			key: "workspace_files",
			label: "Workspace Files",
			available: !!scan.workspace_files_available,
			detail: (scan.workspace_files_found?.length ?? 0) > 0 ? scan.workspace_files_found?.join(", ") || null : null,
		},
	];
	const anySelected = categories.some((c) => c.available && selection[c.key]);

	const workspaceMissing = !scan.memory_available && (scan.skills_count ?? 0) === 0 && !scan.identity_theme;

	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Import from OpenClaw</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed">
				We detected an OpenClaw installation at <code className="text-[var(--text)]">{scan.home_dir}</code>. Select the
				data you'd like to bring into Moltis.
			</p>
			<p className="text-xs text-[var(--muted)] leading-relaxed">
				This is a read-only copy &mdash; your OpenClaw installation will not be modified or removed. You can keep using
				OpenClaw alongside Moltis, and re-import at any time from Settings.
			</p>
			{workspaceMissing ? (
				<p className="text-xs text-[var(--muted)] leading-relaxed">
					If OpenClaw ran on another machine, copy its workspace directory (e.g. <code>clawd/</code>) into{" "}
					<code>{scan.home_dir}/</code> or <code>~/</code> for a full import including identity, memory, and skills.
				</p>
			) : null}
			{error ? <ErrorPanel message={error} /> : null}
			<div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
				{categories.map((cat) => {
					const checked = selection[cat.key] && cat.available;
					return (
						<button
							key={cat.key}
							type="button"
							onClick={() => cat.available && !importing && toggleCategory(cat.key)}
							disabled={!cat.available || importing}
							className={`flex items-center gap-3 p-3 rounded-md border text-left cursor-pointer transition-colors ${
								cat.available
									? checked
										? "border-[var(--accent)] bg-[var(--accent-bg,rgba(var(--accent-rgb,59,130,246),0.08))]"
										: "border-[var(--border)] bg-[var(--surface)] opacity-60"
									: "border-[var(--border)] bg-[var(--surface)] opacity-40 cursor-not-allowed"
							}`}
						>
							<span className="text-lg shrink-0">{IMPORT_CATEGORY_ICONS[cat.key] || "\uD83D\uDCE6"}</span>
							<div className="flex-1 min-w-0">
								<span className="text-sm font-medium text-[var(--text-strong)]">{cat.label}</span>
								{cat.detail && cat.available ? (
									<div className="text-xs text-[var(--muted)] mt-0.5">{cat.detail}</div>
								) : null}
								{cat.available ? null : <div className="text-xs text-[var(--muted)] mt-0.5">not found</div>}
							</div>
							<div className="shrink-0">
								{checked ? (
									<span className="icon icon-check-circle text-[var(--accent)]" />
								) : (
									<span className="w-4 h-4 rounded-full border-2 border-[var(--border)] inline-block" />
								)}
							</div>
						</button>
					);
				})}
			</div>
			{(scan.agents?.length ?? 0) > 1 ? (
				<div
					className="text-xs text-[var(--muted)] leading-relaxed border border-[var(--border)] rounded p-2"
					style="max-width:400px;"
				>
					<span className="font-medium text-[var(--text)]">{scan.agents?.length} agents detected</span>
					<span className="ml-1">&mdash; non-default agents will be created as separate personas:</span>
					<ul className="mt-1 ml-4 list-disc">
						{(scan.agents || []).map((a) => (
							<li key={a.openclaw_id}>
								<span className="text-[var(--text)]">{a.name || a.openclaw_id}</span>
								{a.is_default ? <span className="ml-1 text-[var(--muted)]">(default)</span> : null}
								{a.theme ? <span className="ml-1 text-[var(--muted)]">&mdash; {a.theme}</span> : null}
							</li>
						))}
					</ul>
				</div>
			) : null}
			{unsupportedChannels.length > 0 ? (
				<p className="text-xs text-[var(--muted)]">
					Unsupported channels (coming soon): {unsupportedChannels.join(", ")}
				</p>
			) : null}
			<div className="flex flex-wrap items-center gap-3 mt-1">
				{onBack ? (
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onBack} disabled={importing}>
						Back
					</button>
				) : null}
				<button type="button" className="provider-btn" onClick={doImport} disabled={!anySelected || importing}>
					{importing ? "Importing\u2026" : "Import Selected"}
				</button>
				<button
					type="button"
					className="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
					onClick={onNext}
					disabled={importing}
				>
					Skip for now
				</button>
			</div>
		</div>
	);
}

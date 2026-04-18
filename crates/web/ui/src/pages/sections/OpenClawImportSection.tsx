// ── OpenClaw Import section ───────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { SectionHeading } from "../../components/forms";
import { sendRpc } from "../../helpers";
import { ChannelType } from "../../types";
import type { RpcResponse } from "./_shared";
import { rerender } from "./_shared";

interface ScanResult {
	detected?: boolean;
	home_dir?: string;
	telegram_accounts?: number;
	discord_accounts?: number;
	unsupported_channels?: string[];
	identity_available?: boolean;
	providers_available?: boolean;
	skills_count?: number;
	memory_available?: boolean;
	memory_files_count?: number;
	channels_available?: boolean;
	sessions_count?: number;
}

interface ImportCategory {
	category: string;
	status: string;
	items_imported: number;
	items_skipped: number;
}

interface ImportResult {
	categories?: ImportCategory[];
}

interface ImportSelection {
	identity: boolean;
	providers: boolean;
	skills: boolean;
	memory: boolean;
	channels: boolean;
	sessions: boolean;
	[key: string]: boolean;
}

export function OpenClawImportSection(): VNode {
	const [importLoading, setImportLoading] = useState(true);
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
	});

	useEffect(() => {
		let cancelled = false;
		sendRpc("openclaw.scan", {}).then((res: RpcResponse) => {
			if (cancelled) return;
			if (res?.ok) setScan(res.payload as ScanResult);
			else setError("Failed to scan OpenClaw installation");
			setImportLoading(false);
			rerender();
		});
		return () => {
			cancelled = true;
		};
	}, []);

	function toggleCategory(key: string): void {
		setSelection((prev) => {
			const next = Object.assign({}, prev);
			next[key] = !prev[key];
			return next;
		});
	}

	function doImport(): void {
		setImporting(true);
		setError(null);
		sendRpc("openclaw.import", selection).then((res: RpcResponse) => {
			setImporting(false);
			if (res?.ok) {
				setResult(res.payload as ImportResult);
				setDone(true);
			} else {
				setError((res?.error as { message?: string })?.message || "Import failed");
			}
			rerender();
		});
	}

	if (importLoading) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<SectionHeading title="OpenClaw Import" />
				<div className="text-xs text-[var(--muted)]">Scanning{"\u2026"}</div>
			</div>
		);
	}

	if (!scan?.detected) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<SectionHeading title="OpenClaw Import" />
				<div className="text-xs text-[var(--muted)]">No OpenClaw installation detected.</div>
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
		(channel) => String(channel).toLowerCase() !== ChannelType.Discord,
	);

	const categories = [
		{ key: "identity", label: "Identity", available: scan.identity_available, detail: undefined as string | undefined },
		{
			key: "providers",
			label: "Providers",
			available: scan.providers_available,
			detail: undefined as string | undefined,
		},
		{
			key: "skills",
			label: "Skills",
			available: (scan.skills_count || 0) > 0,
			detail: `${scan.skills_count} skill(s)`,
		},
		{
			key: "memory",
			label: "Memory",
			available: scan.memory_available,
			detail: `${scan.memory_files_count} memory file(s)`,
		},
		{
			key: "channels",
			label: "Channels",
			available: scan.channels_available,
			detail: channelDetail || undefined,
		},
		{
			key: "sessions",
			label: "Sessions",
			available: (scan.sessions_count || 0) > 0,
			detail: `${scan.sessions_count} session(s)`,
		},
	];
	const anySelected = categories.some((c) => c.available && selection[c.key]);

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<SectionHeading title="OpenClaw Import" />
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "600px", margin: 0 }}>
				Import data from your OpenClaw installation at <code className="text-[var(--text)]">{scan.home_dir}</code>. This
				is a read-only copy {"\u2014"} your OpenClaw files will not be modified or removed. You can keep using both side
				by side and re-import whenever you like.
			</p>
			{error ? (
				<div role="alert" className="alert-error-text whitespace-pre-line" style={{ maxWidth: "600px" }}>
					<span className="text-[var(--error)] font-medium">Error:</span> {error}
				</div>
			) : null}
			{done && result ? (
				<div className="flex flex-col gap-2" style={{ maxWidth: "600px" }}>
					<div className="text-sm font-medium text-[var(--ok)]">
						Import complete:{" "}
						{(result.categories || []).reduce((sum, cat) => sum + (Number(cat.items_imported) || 0), 0)} item(s)
						imported.
					</div>
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
								</div>
							))}
						</div>
					) : null}
					<button
						className="provider-btn provider-btn-secondary mt-2"
						style={{ width: "fit-content" }}
						onClick={() => {
							setDone(false);
							setResult(null);
							rerender();
						}}
					>
						Import Again
					</button>
				</div>
			) : (
				<div className="flex flex-col gap-2" style={{ maxWidth: "400px" }}>
					{categories.map((cat) => (
						<label
							key={cat.key}
							className={`flex items-center gap-2 text-sm cursor-pointer ${cat.available ? "text-[var(--text)]" : "text-[var(--muted)] opacity-60"}`}
						>
							<input
								type="checkbox"
								checked={selection[cat.key] && cat.available}
								disabled={!cat.available || importing}
								onChange={() => toggleCategory(cat.key)}
							/>
							<span>{cat.label}</span>
							{cat.detail && cat.available ? <span className="text-xs text-[var(--muted)]">({cat.detail})</span> : null}
							{cat.available ? null : <span className="text-xs text-[var(--muted)]">(not found)</span>}
						</label>
					))}
				</div>
			)}
			{!done && unsupportedChannels.length > 0 ? (
				<p className="text-xs text-[var(--muted)]" style={{ maxWidth: "600px" }}>
					Unsupported channels (coming soon): {unsupportedChannels.join(", ")}
				</p>
			) : null}
			{done ? null : (
				<button
					className="provider-btn mt-2"
					style={{ width: "fit-content" }}
					onClick={doImport}
					disabled={!anySelected || importing}
				>
					{importing ? "Importing\u2026" : "Import Selected"}
				</button>
			)}
		</div>
	);
}

// ── Hermes Import section ───���─────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { SectionHeading } from "../../components/forms";
import { sendRpc } from "../../helpers";
import type { RpcResponse } from "./_shared";
import { rerender } from "./_shared";

interface HermesScanResult {
	detected?: boolean;
	home_dir?: string;
	has_credentials?: boolean;
	credentials_count?: number;
	skills_count?: number;
	has_memory?: boolean;
	memory_files?: string[];
}

interface ImportCategory {
	category: string;
	status: string;
	items_imported: number;
	items_skipped: number;
}

interface ImportResult {
	categories?: ImportCategory[];
	total_imported?: number;
}

interface HermesSelection {
	credentials: boolean;
	skills: boolean;
	memory: boolean;
	[key: string]: boolean;
}

const CATEGORY_ICONS: Record<string, string> = {
	credentials: "\uD83D\uDD11",
	skills: "\u2728",
	memory: "\uD83E\uDDE0",
};

export function HermesImportSection(): VNode {
	const [loading, setLoading] = useState(true);
	const [scan, setScan] = useState<HermesScanResult | null>(null);
	const [importing, setImporting] = useState(false);
	const [done, setDone] = useState(false);
	const [result, setResult] = useState<ImportResult | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [selection, setSelection] = useState<HermesSelection>({
		credentials: true,
		skills: true,
		memory: true,
	});

	useEffect(() => {
		let cancelled = false;
		sendRpc("hermes.detect", {}).then((res: RpcResponse) => {
			if (cancelled) return;
			if (res?.ok) setScan(res.payload as HermesScanResult);
			else setError("Failed to scan Hermes installation");
			setLoading(false);
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
		sendRpc("hermes.import", selection).then((res: RpcResponse) => {
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

	if (loading) {
		return (
			<div>
				<SectionHeading title="Hermes" />
				<div className="text-xs text-[var(--muted)]">Scanning{"\u2026"}</div>
			</div>
		);
	}

	if (!scan?.detected) {
		return (
			<div>
				<SectionHeading title="Hermes" />
				<div className="text-xs text-[var(--muted)]">No Hermes installation detected.</div>
			</div>
		);
	}

	const memoryDetail = (scan.memory_files || []).join(", ");

	const categories = [
		{
			key: "credentials",
			label: "Credentials",
			available: scan.has_credentials,
			detail: scan.credentials_count ? `${scan.credentials_count} provider(s)` : undefined,
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
			available: scan.has_memory,
			detail: memoryDetail || undefined,
		},
	];
	const anySelected = categories.some((c) => c.available && selection[c.key]);

	return (
		<div>
			<SectionHeading title="Hermes" />
			<p className="text-xs text-[var(--muted)] leading-relaxed mb-3 max-w-[600px]">
				Import data from your Hermes installation at <code className="text-[var(--text)]">{scan.home_dir}</code>. This
				is a read-only copy {"\u2014"} your Hermes files will not be modified.
			</p>
			{error ? (
				<div role="alert" className="alert-error-text whitespace-pre-line mb-3 max-w-[600px]">
					<span className="text-[var(--error)] font-medium">Error:</span> {error}
				</div>
			) : null}
			{done && result ? (
				<div className="flex flex-col gap-2 max-w-[600px]">
					<div className="text-sm font-medium text-[var(--ok)]">
						Import complete: {result.total_imported || 0} item(s) imported.
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
						type="button"
						className="provider-btn provider-btn-secondary mt-2 w-fit"
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
				<div className="grid grid-cols-1 sm:grid-cols-2 gap-2 max-w-[600px]">
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
								<span className="text-lg shrink-0">{CATEGORY_ICONS[cat.key] || "\uD83D\uDCE6"}</span>
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
			)}
			{done ? null : (
				<button
					type="button"
					className="provider-btn mt-3 w-fit"
					onClick={doImport}
					disabled={!anySelected || importing}
				>
					{importing ? "Importing\u2026" : "Import Selected"}
				</button>
			)}
		</div>
	);
}

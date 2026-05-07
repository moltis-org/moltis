// ── Moltis data import/export section ─────────────────────────

import type { VNode } from "preact";
import { useRef, useState } from "preact/hooks";
import { CheckboxField, Loading, SectionHeading, StatusMessage, SubHeading } from "../../components/forms";
import { rerender } from "./_shared";

interface ImportedItem {
	category: string;
	path: string;
	action: string;
}

interface ImportPreview {
	manifest: {
		format_version: number;
		moltis_version: string;
		created_at: string;
		inventory: {
			config_files: string[];
			workspace_files: string[];
			has_moltis_db: boolean;
			has_memory_db: boolean;
			session_files: string[];
			media_files: string[];
		};
	};
	imported: ImportedItem[];
	skipped: ImportedItem[];
	warnings: string[];
}

export function MoltisDataSection(): VNode {
	return (
		<div className="flex flex-col gap-6">
			<ExportSection />
			<hr className="border-gray-700" />
			<ImportDataSection />
		</div>
	);
}

// ── Export ────────────────────────────────────────────────────

function ExportSection(): VNode {
	const [includeKeys, setIncludeKeys] = useState(true);
	const [includeMedia, setIncludeMedia] = useState(false);
	const [exporting, setExporting] = useState(false);
	const [error, setError] = useState<string | null>(null);

	function doExport(): void {
		setExporting(true);
		setError(null);
		const params = new URLSearchParams();
		params.set("include_provider_keys", String(includeKeys));
		params.set("include_media", String(includeMedia));

		fetch(`/api/data/export?${params.toString()}`)
			.then((res) => {
				if (!res.ok) throw new Error(`Export failed: ${res.statusText}`);
				return res.blob().then((blob) => {
					const disposition = res.headers.get("content-disposition") || "";
					const match = /filename="?([^"]+)"?/.exec(disposition);
					const filename = match?.[1] || "moltis-backup.tar.gz";
					const url = URL.createObjectURL(blob);
					const a = document.createElement("a");
					a.href = url;
					a.download = filename;
					a.click();
					URL.revokeObjectURL(url);
				});
			})
			.catch((e: Error) => {
				setError(e.message);
			})
			.finally(() => {
				setExporting(false);
				rerender();
			});
	}

	return (
		<div className="flex flex-col gap-3">
			<SectionHeading title="Export" />
			<p className="text-sm text-gray-400">
				Download a backup of your Moltis config, databases, sessions, and workspace files.
			</p>
			<div className="flex flex-col gap-2">
				<CheckboxField
					label="Include provider API keys"
					checked={includeKeys}
					onChange={(checked) => {
						setIncludeKeys(checked);
						rerender();
					}}
				/>
				<CheckboxField
					label="Include session media (audio, images)"
					checked={includeMedia}
					onChange={(checked) => {
						setIncludeMedia(checked);
						rerender();
					}}
				/>
			</div>
			<div>
				<button type="button" className="provider-btn" disabled={exporting} onClick={doExport}>
					{exporting ? "Exporting..." : "Download backup (.tar.gz)"}
				</button>
			</div>
			<StatusMessage error={error} />
		</div>
	);
}

// ── Import ───────────────────────────────────────────────────

function ImportDataSection(): VNode {
	const [conflict, setConflict] = useState<"skip" | "overwrite">("skip");
	const [uploading, setUploading] = useState(false);
	const [applying, setApplying] = useState(false);
	const [preview, setPreview] = useState<ImportPreview | null>(null);
	const [result, setResult] = useState<ImportPreview | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [selectedFile, setSelectedFile] = useState<File | null>(null);
	const fileRef = useRef<HTMLInputElement>(null);

	function onFileSelect(file: File | null): void {
		setSelectedFile(file);
		setPreview(null);
		setResult(null);
		setError(null);
		if (file) {
			doPreview(file);
		}
		rerender();
	}

	function doPreview(file: File): void {
		setUploading(true);
		setError(null);
		file
			.arrayBuffer()
			.then((buf) =>
				fetch(`/api/data/import/preview?conflict=${conflict}`, {
					method: "POST",
					headers: { "Content-Type": "application/gzip" },
					body: buf,
				}),
			)
			.then((res) => res.json())
			.then((data: ImportPreview & { ok?: boolean; error?: string }) => {
				if (data.ok === false) {
					setError(data.error || "Preview failed");
				} else {
					setPreview(data);
				}
			})
			.catch((e: Error) => setError(e.message))
			.finally(() => {
				setUploading(false);
				rerender();
			});
	}

	function doApply(): void {
		if (!selectedFile) return;
		setApplying(true);
		setError(null);
		selectedFile
			.arrayBuffer()
			.then((buf) =>
				fetch(`/api/data/import?conflict=${conflict}`, {
					method: "POST",
					headers: { "Content-Type": "application/gzip" },
					body: buf,
				}),
			)
			.then((res) => res.json())
			.then((data: ImportPreview & { ok?: boolean; error?: string }) => {
				if (data.ok === false) {
					setError(data.error || "Import failed");
				} else {
					setResult(data);
					setPreview(null);
				}
			})
			.catch((e: Error) => setError(e.message))
			.finally(() => {
				setApplying(false);
				rerender();
			});
	}

	return (
		<div className="flex flex-col gap-3">
			<SectionHeading title="Import" />
			<p className="text-sm text-gray-400">Restore from a previously exported Moltis backup archive.</p>

			{/* Conflict strategy */}
			<div className="flex flex-col gap-1">
				<SubHeading title="On conflict" />
				<div className="flex gap-4 text-sm">
					<label className="flex items-center gap-1.5 cursor-pointer">
						<input
							type="radio"
							name="conflict"
							checked={conflict === "skip"}
							onChange={() => {
								setConflict("skip");
								rerender();
							}}
						/>
						Skip existing
					</label>
					<label className="flex items-center gap-1.5 cursor-pointer">
						<input
							type="radio"
							name="conflict"
							checked={conflict === "overwrite"}
							onChange={() => {
								setConflict("overwrite");
								rerender();
							}}
						/>
						Overwrite
					</label>
				</div>
			</div>

			{/* File picker */}
			<button
				type="button"
				className="border-2 border-dashed border-gray-600 rounded-lg p-6 text-center cursor-pointer hover:border-gray-400 transition-colors w-full bg-transparent"
				onClick={() => fileRef.current?.click()}
				onDragOver={(e) => e.preventDefault()}
				onDrop={(e) => {
					e.preventDefault();
					const file = e.dataTransfer?.files[0];
					if (file) onFileSelect(file);
				}}
			>
				<input
					ref={fileRef}
					type="file"
					accept=".tar.gz,.tgz"
					className="hidden"
					onChange={(e) => {
						const file = (e.target as HTMLInputElement).files?.[0] || null;
						onFileSelect(file);
					}}
				/>
				{selectedFile ? (
					<span className="text-sm">
						Selected: <strong>{selectedFile.name}</strong> ({(selectedFile.size / 1024 / 1024).toFixed(1)} MB)
					</span>
				) : (
					<span className="text-sm text-gray-400">Drop a .tar.gz archive here or click to select</span>
				)}
			</button>

			{uploading ? <Loading /> : null}
			<StatusMessage error={error} />

			{/* Preview */}
			{preview ? <PreviewTable preview={preview} onApply={doApply} applying={applying} /> : null}

			{/* Result */}
			{result ? <ResultTable result={result} /> : null}
		</div>
	);
}

// ── Preview table ────────────────────────────────────────────

interface PreviewTableProps {
	preview: ImportPreview;
	onApply: () => void;
	applying: boolean;
}

function PreviewTable({ preview, onApply, applying }: PreviewTableProps): VNode {
	const inv = preview.manifest.inventory;
	return (
		<div className="flex flex-col gap-3 p-3 bg-gray-800 rounded-lg">
			<SubHeading title="Archive preview" />
			<p className="text-xs text-gray-400">
				Moltis {preview.manifest.moltis_version} — {preview.manifest.created_at}
			</p>
			<table className="text-sm w-full">
				<tbody>
					<Row label="Config files" value={inv.config_files.length} />
					<Row label="Workspace files" value={inv.workspace_files.length} />
					<Row label="moltis.db" value={inv.has_moltis_db ? "Yes" : "No"} />
					<Row label="memory.db" value={inv.has_memory_db ? "Yes" : "No"} />
					<Row label="Sessions" value={inv.session_files.filter((f) => f.endsWith(".jsonl")).length} />
					<Row label="Media files" value={inv.media_files.length} />
				</tbody>
			</table>
			{preview.warnings.length > 0 ? (
				<div className="text-xs text-yellow-400">
					{preview.warnings.map((w) => (
						<p key={w}>{w}</p>
					))}
				</div>
			) : null}
			<button type="button" className="provider-btn" disabled={applying} onClick={onApply}>
				{applying ? "Importing..." : "Apply import"}
			</button>
		</div>
	);
}

// ── Result table ─────────────────────────────────────────────

function ResultTable({ result }: { result: ImportPreview }): VNode {
	return (
		<div className="flex flex-col gap-2 p-3 bg-gray-800 rounded-lg">
			<SubHeading title="Import complete" />
			<StatusMessage success={`${result.imported.length} items imported, ${result.skipped.length} skipped.`} />
			{result.imported.length > 0 ? (
				<details className="text-xs">
					<summary className="cursor-pointer text-gray-400">Imported ({result.imported.length})</summary>
					<ul className="mt-1 ml-4 list-disc">
						{result.imported.map((item) => (
							<li key={`${item.category}-${item.path}`}>
								[{item.category}] {item.path} — {item.action}
							</li>
						))}
					</ul>
				</details>
			) : null}
			{result.skipped.length > 0 ? (
				<details className="text-xs">
					<summary className="cursor-pointer text-gray-400">Skipped ({result.skipped.length})</summary>
					<ul className="mt-1 ml-4 list-disc">
						{result.skipped.map((item) => (
							<li key={`${item.category}-${item.path}`}>
								[{item.category}] {item.path} — {item.action}
							</li>
						))}
					</ul>
				</details>
			) : null}
			{result.warnings.length > 0 ? (
				<div className="text-xs text-yellow-400 mt-1">
					{result.warnings.map((w) => (
						<p key={w}>{w}</p>
					))}
				</div>
			) : null}
		</div>
	);
}

function Row({ label, value }: { label: string; value: string | number }): VNode {
	return (
		<tr>
			<td className="pr-4 text-gray-400 py-0.5">{label}</td>
			<td className="py-0.5">{value}</td>
		</tr>
	);
}

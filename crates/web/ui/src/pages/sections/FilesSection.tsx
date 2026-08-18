import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { SectionHeading } from "../../components/forms";
import { targetValue } from "../../typed-events";
import {
	bufferDownload,
	createDirectory,
	deleteEntry,
	downloadFile,
	type FileEntry,
	isConflict,
	listEntries,
	moveEntry,
	uploadFile,
} from "./files/api";
import {
	collectDroppedContents,
	contentsFromFileList,
	type DroppedContents,
	type UploadDirectory,
	type UploadFile,
} from "./files/drop-files";

type SortKey = "name" | "modifiedAt" | "sizeBytes";
type SortDirection = "ascending" | "descending";

interface SortState {
	key: SortKey;
	direction: SortDirection;
}

interface ConfirmationState {
	title: string;
	message: string;
	confirmLabel: string;
	danger: boolean;
	resolve: (confirmed: boolean) => void;
}

type EditorState =
	| { kind: "new-folder"; value: string }
	| { kind: "rename"; entry: FileEntry; value: string }
	| { kind: "move"; entry: FileEntry; destination: string; browsePath: string };

interface ConfirmationDialogProps {
	state: ConfirmationState;
	onClose: (confirmed: boolean) => void;
}

interface SaveFileHandle {
	createWritable(): Promise<WritableStream<Uint8Array>>;
}

interface SaveFilePickerWindow {
	showSaveFilePicker?: (options: { suggestedName: string }) => Promise<SaveFileHandle>;
}

const MAX_BUFFERED_BROWSER_DOWNLOAD_BYTES = 64 * 1024 * 1024;

interface EditorDialogProps {
	state: EditorState;
	browseFolders: FileEntry[];
	browseLoading: boolean;
	browseError: string | null;
	onChange: (state: EditorState) => void;
	onClose: () => void;
	onSubmit: () => void;
}

interface UploadResult {
	item: UploadFile;
	error: unknown;
}

function errorMessage(error: unknown, fallback: string): string {
	return error instanceof Error && error.message ? error.message : fallback;
}

function normalizePath(path: string): string {
	const parts: string[] = [];
	for (const part of path.replaceAll("\\", "/").split("/")) {
		if (!part || part === ".") continue;
		if (part === "..") {
			parts.pop();
			continue;
		}
		parts.push(part);
	}
	return parts.join("/");
}

function joinPath(parent: string, child: string): string {
	return normalizePath(parent ? `${parent}/${child}` : child);
}

function parentPath(path: string): string {
	const parts = normalizePath(path).split("/").filter(Boolean);
	parts.pop();
	return parts.join("/");
}

function validName(name: string): boolean {
	const trimmed = name.trim();
	return Boolean(trimmed && trimmed !== "." && trimmed !== ".." && !trimmed.includes("/") && !trimmed.includes("\\"));
}

function formatSize(size: number | null): string {
	if (size === null) return "-";
	if (size < 1024) return `${size} B`;
	const units = ["KiB", "MiB", "GiB", "TiB"];
	let value = size / 1024;
	let unit = units[0];
	for (let index = 1; index < units.length && value >= 1024; index += 1) {
		value /= 1024;
		unit = units[index];
	}
	return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)} ${unit}`;
}

function formatDate(value: string | null): string {
	if (!value) return "-";
	const date = new Date(value);
	return Number.isNaN(date.valueOf()) ? "-" : date.toLocaleString();
}

function compareNullableNumbers(left: number | null, right: number | null, direction: SortDirection): number {
	if (left === null && right === null) return 0;
	if (left === null) return 1;
	if (right === null) return -1;
	return direction === "ascending" ? left - right : right - left;
}

function sortedEntries(entries: FileEntry[], sort: SortState): FileEntry[] {
	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Each sort key has distinct null and direction semantics.
	return entries.slice().sort((left, right) => {
		const leftFolder = left.kind === "directory";
		const rightFolder = right.kind === "directory";
		if (leftFolder !== rightFolder) return leftFolder ? -1 : 1;

		let comparison = 0;
		if (sort.key === "name") {
			comparison = left.name.localeCompare(right.name, undefined, { numeric: true, sensitivity: "base" });
			if (sort.direction === "descending") comparison *= -1;
		} else if (sort.key === "modifiedAt") {
			const leftTime = left.modifiedAt ? Date.parse(left.modifiedAt) : null;
			const rightTime = right.modifiedAt ? Date.parse(right.modifiedAt) : null;
			comparison = compareNullableNumbers(
				leftTime !== null && Number.isNaN(leftTime) ? null : leftTime,
				rightTime !== null && Number.isNaN(rightTime) ? null : rightTime,
				sort.direction,
			);
		} else {
			comparison = compareNullableNumbers(left.sizeBytes, right.sizeBytes, sort.direction);
		}

		return comparison || left.name.localeCompare(right.name, undefined, { numeric: true, sensitivity: "base" });
	});
}

async function uploadWithConcurrency(
	items: UploadFile[],
	destination: string,
	signal: AbortSignal,
): Promise<UploadResult[]> {
	let nextIndex = 0;
	const results: UploadResult[] = [];
	const worker = async (): Promise<void> => {
		for (;;) {
			const index = nextIndex;
			nextIndex += 1;
			if (index >= items.length) return;
			const item = items[index];
			try {
				await uploadFile(joinPath(destination, item.relativePath), item.file, false, signal);
			} catch (error) {
				results.push({ item, error });
			}
		}
	};
	await Promise.all(Array.from({ length: Math.min(4, items.length) }, worker));
	return results;
}

function breadcrumbParts(path: string): Array<{ label: string; path: string }> {
	const parts = normalizePath(path).split("/").filter(Boolean);
	return parts.map((label, index) => ({ label, path: parts.slice(0, index + 1).join("/") }));
}

function ConfirmationDialog({ state, onClose }: ConfirmationDialogProps): VNode {
	const confirmRef = useRef<HTMLButtonElement>(null);
	useEffect(() => confirmRef.current?.focus(), []);
	return (
		<div className="fixed inset-0 z-[110] flex items-center justify-center bg-black/50 p-4" role="presentation">
			<div
				className="w-full max-w-md rounded-lg border border-[var(--border)] bg-[var(--surface)] p-5 shadow-xl"
				role="alertdialog"
				aria-modal="true"
				aria-labelledby="files-confirm-title"
				aria-describedby="files-confirm-message"
			>
				<h3 id="files-confirm-title" className="text-base font-semibold text-[var(--text-strong)]">
					{state.title}
				</h3>
				<p id="files-confirm-message" className="mt-2 text-sm text-[var(--text)]">
					{state.message}
				</p>
				<div className="mt-5 flex justify-end gap-2">
					<button type="button" className="provider-btn provider-btn-secondary" onClick={() => onClose(false)}>
						Cancel
					</button>
					<button
						ref={confirmRef}
						type="button"
						className={`provider-btn ${state.danger ? "provider-btn-danger" : ""}`}
						onClick={() => onClose(true)}
					>
						{state.confirmLabel}
					</button>
				</div>
			</div>
		</div>
	);
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: The three small editor modes intentionally share one accessible dialog shell.
function EditorDialog({
	state,
	browseFolders,
	browseLoading,
	browseError,
	onChange,
	onClose,
	onSubmit,
}: EditorDialogProps): VNode {
	const title =
		state.kind === "new-folder"
			? "New Folder"
			: state.kind === "rename"
				? `Rename ${state.entry.name}`
				: `Move ${state.entry.name}`;
	const label = state.kind === "move" ? "Destination folder" : state.kind === "rename" ? "New name" : "Folder name";
	const value = state.kind === "move" ? state.destination : state.value;
	const inputRef = useRef<HTMLInputElement>(null);
	useEffect(() => inputRef.current?.focus(), []);

	function updateValue(nextValue: string): void {
		if (state.kind === "move") onChange({ ...state, destination: nextValue });
		else onChange({ ...state, value: nextValue });
	}

	return (
		<div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/50 p-4" role="presentation">
			<form
				className="w-full max-w-lg rounded-lg border border-[var(--border)] bg-[var(--surface)] p-5 shadow-xl"
				role="dialog"
				aria-modal="true"
				aria-labelledby="files-editor-title"
				onSubmit={(event) => {
					event.preventDefault();
					onSubmit();
				}}
			>
				<div className="flex items-center justify-between gap-3">
					<h3 id="files-editor-title" className="text-base font-semibold text-[var(--text-strong)]">
						{title}
					</h3>
					<button type="button" className="text-lg text-[var(--muted)]" aria-label="Close dialog" onClick={onClose}>
						x
					</button>
				</div>
				<label className="mt-4 block text-xs text-[var(--muted)]" htmlFor="files-editor-input">
					{label}
				</label>
				<input
					ref={inputRef}
					id="files-editor-input"
					className="provider-key-input mt-1"
					value={value}
					onInput={(event) => updateValue(targetValue(event))}
					autoComplete="off"
				/>

				{state.kind === "move" ? (
					<div className="mt-4 rounded-md border border-[var(--border)] bg-[var(--bg)] p-3">
						<nav className="flex flex-wrap items-center gap-1 text-xs" aria-label="Browse destination folders">
							<button
								type="button"
								className="text-[var(--accent)] hover:underline"
								onClick={() => onChange({ ...state, browsePath: "" })}
							>
								Files
							</button>
							{breadcrumbParts(state.browsePath).map((part) => (
								<span key={part.path} className="flex items-center gap-1">
									<span aria-hidden="true">/</span>
									<button
										type="button"
										className="text-[var(--accent)] hover:underline"
										onClick={() => onChange({ ...state, browsePath: part.path })}
									>
										{part.label}
									</button>
								</span>
							))}
						</nav>
						<button
							type="button"
							className="provider-btn provider-btn-secondary provider-btn-sm mt-3"
							onClick={() => onChange({ ...state, destination: state.browsePath })}
						>
							Use this folder
						</button>
						{browseLoading ? <p className="mt-3 text-xs text-[var(--muted)]">Loading folders...</p> : null}
						{browseError ? (
							<p className="mt-3 text-xs text-[var(--error)]" role="alert">
								{browseError}
							</p>
						) : null}
						{browseLoading || browseError ? null : (
							<ul className="mt-3 max-h-36 space-y-1 overflow-y-auto" aria-label="Folders">
								{browseFolders.length === 0 ? <li className="text-xs text-[var(--muted)]">No subfolders</li> : null}
								{browseFolders.map((folder) => (
									<li key={folder.path}>
										<button
											type="button"
											className="w-full rounded px-2 py-1 text-left text-xs text-[var(--text)] hover:bg-[var(--bg-hover)]"
											onClick={() => onChange({ ...state, browsePath: joinPath(state.browsePath, folder.name) })}
										>
											{folder.name}
										</button>
									</li>
								))}
							</ul>
						)}
					</div>
				) : null}

				<div className="mt-5 flex justify-end gap-2">
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onClose}>
						Cancel
					</button>
					<button type="submit" className="provider-btn">
						{state.kind === "new-folder" ? "Create" : state.kind === "rename" ? "Rename" : "Move"}
					</button>
				</div>
			</form>
		</div>
	);
}

export function FilesSection(): VNode {
	const [currentPath, setCurrentPath] = useState("");
	const [entries, setEntries] = useState<FileEntry[]>([]);
	const [loading, setLoading] = useState(true);
	const [error, setError] = useState<string | null>(null);
	const [status, setStatus] = useState<string | null>(null);
	const [uploading, setUploading] = useState(false);
	const [sort, setSort] = useState<SortState>({ key: "name", direction: "ascending" });
	const [refreshVersion, setRefreshVersion] = useState(0);
	const [dragging, setDragging] = useState(false);
	const [confirmation, setConfirmation] = useState<ConfirmationState | null>(null);
	const [editor, setEditor] = useState<EditorState | null>(null);
	const [browseFolders, setBrowseFolders] = useState<FileEntry[]>([]);
	const [browseLoading, setBrowseLoading] = useState(false);
	const [browseError, setBrowseError] = useState<string | null>(null);
	const fileInputRef = useRef<HTMLInputElement>(null);
	const folderInputRef = useRef<HTMLInputElement>(null);
	const dragDepth = useRef(0);
	const uploadActive = useRef(false);
	const uploadController = useRef<AbortController | null>(null);
	const confirmationQueue = useRef<ConfirmationState[]>([]);

	useEffect(() => {
		folderInputRef.current?.setAttribute("webkitdirectory", "");
		folderInputRef.current?.setAttribute("directory", "");
		return () => {
			uploadController.current?.abort();
			for (const pending of confirmationQueue.current) pending.resolve(false);
			confirmationQueue.current = [];
		};
	}, []);

	useEffect(() => {
		const controller = new AbortController();
		setLoading(true);
		setError(null);
		listEntries(currentPath, controller.signal)
			.then((response) => {
				setEntries(response.entries);
				setLoading(false);
			})
			.catch((requestError: unknown) => {
				if (requestError instanceof Error && requestError.name === "AbortError") return;
				setEntries([]);
				setLoading(false);
				setError(errorMessage(requestError, "Could not load files"));
			});
		return () => controller.abort();
	}, [currentPath, refreshVersion]);

	const moveBrowsePath = editor?.kind === "move" ? editor.browsePath : null;
	useEffect(() => {
		if (moveBrowsePath === null) {
			setBrowseFolders([]);
			setBrowseError(null);
			return;
		}
		const controller = new AbortController();
		setBrowseLoading(true);
		setBrowseError(null);
		listEntries(moveBrowsePath, controller.signal)
			.then((response) => {
				setBrowseFolders(response.entries.filter((entry) => entry.kind === "directory"));
				setBrowseLoading(false);
			})
			.catch((requestError: unknown) => {
				if (requestError instanceof Error && requestError.name === "AbortError") return;
				setBrowseFolders([]);
				setBrowseLoading(false);
				setBrowseError(errorMessage(requestError, "Could not browse folders"));
			});
		return () => controller.abort();
	}, [moveBrowsePath]);

	function refresh(): void {
		setRefreshVersion((version) => version + 1);
	}

	function requestConfirmation(title: string, message: string, confirmLabel: string, danger = false): Promise<boolean> {
		return new Promise((resolve) => {
			const pending = { title, message, confirmLabel, danger, resolve };
			setConfirmation((current) => {
				if (!current) return pending;
				confirmationQueue.current.push(pending);
				return current;
			});
		});
	}

	function closeConfirmation(confirmed: boolean): void {
		setConfirmation((current) => {
			if (!current) return null;
			current.resolve(confirmed);
			return confirmationQueue.current.shift() ?? null;
		});
	}

	function setSortKey(key: SortKey): void {
		setSort((previous) => {
			if (previous.key === key) {
				return { key, direction: previous.direction === "ascending" ? "descending" : "ascending" };
			}
			return { key, direction: key === "name" ? "ascending" : "descending" };
		});
	}

	async function createUploadDirectories(directories: UploadDirectory[], destination: string): Promise<string[]> {
		const failures: string[] = [];
		const ordered = directories
			.slice()
			.sort((left, right) => left.relativePath.split("/").length - right.relativePath.split("/").length);
		for (const directory of ordered) {
			try {
				await createDirectory(joinPath(destination, directory.relativePath));
			} catch (requestError) {
				if (!isConflict(requestError))
					failures.push(errorMessage(requestError, `Could not create ${directory.relativePath}`));
			}
		}
		return failures;
	}

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Uploads aggregate directory, file, conflict, and retry outcomes in one transaction.
	async function uploadContents(contents: DroppedContents, destination: string): Promise<void> {
		if (contents.files.length === 0 && contents.directories.length === 0) return;
		if (uploadActive.current) {
			setError("Wait for the current upload to finish before starting another one.");
			return;
		}
		uploadActive.current = true;
		setUploading(true);
		const controller = new AbortController();
		uploadController.current = controller;
		setError(null);
		setStatus(`Uploading ${contents.files.length} file${contents.files.length === 1 ? "" : "s"}...`);
		try {
			const failures = await createUploadDirectories(contents.directories, destination);
			const uploadResults = await uploadWithConcurrency(contents.files, destination, controller.signal);
			let uploaded = contents.files.length - uploadResults.length;

			for (const result of uploadResults) {
				if (!isConflict(result.error)) {
					failures.push(errorMessage(result.error, `Could not upload ${result.item.relativePath}`));
					continue;
				}
				const overwrite = await requestConfirmation(
					"Replace file?",
					`${result.item.relativePath} already exists. Overwrite it with the uploaded file?`,
					"Overwrite",
				);
				if (!overwrite) continue;
				try {
					await uploadFile(joinPath(destination, result.item.relativePath), result.item.file, true, controller.signal);
					uploaded += 1;
				} catch (requestError) {
					failures.push(errorMessage(requestError, `Could not overwrite ${result.item.relativePath}`));
				}
			}

			if (failures.length > 0) setError(failures.join(" "));
			setStatus(`Upload complete: ${uploaded} file${uploaded === 1 ? "" : "s"} uploaded.`);
			refresh();
		} finally {
			uploadActive.current = false;
			uploadController.current = null;
			setUploading(false);
		}
	}

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Editor modes share conflict handling and refresh behavior.
	async function submitEditor(): Promise<void> {
		if (!editor) return;
		setError(null);
		try {
			if (editor.kind === "new-folder") {
				if (!validName(editor.value)) {
					setError("Enter a folder name without slashes.");
					return;
				}
				await createDirectory(joinPath(currentPath, editor.value.trim()));
				setStatus(`Created folder ${editor.value.trim()}.`);
			} else {
				const destination =
					editor.kind === "rename"
						? validName(editor.value)
							? joinPath(parentPath(editor.entry.path), editor.value.trim())
							: ""
						: joinPath(editor.destination, editor.entry.name);
				if (!destination) {
					setError("Enter a valid name or destination.");
					return;
				}
				try {
					await moveEntry(editor.entry.path, destination);
				} catch (requestError) {
					if (!isConflict(requestError)) throw requestError;
					if (editor.entry.kind !== "file") {
						throw new Error("A folder already exists at that destination.");
					}
					const overwrite = await requestConfirmation(
						"Replace existing item?",
						`${destination} already exists. Replace it?`,
						"Replace",
						false,
					);
					if (!overwrite) return;
					await moveEntry(editor.entry.path, destination, true);
				}
				setStatus(editor.kind === "rename" ? `Renamed ${editor.entry.name}.` : `Moved ${editor.entry.name}.`);
			}
			setEditor(null);
			refresh();
		} catch (requestError) {
			setError(errorMessage(requestError, "File operation failed"));
		}
	}

	async function removeEntry(entry: FileEntry): Promise<void> {
		const confirmed = await requestConfirmation(
			`Delete ${entry.name}?`,
			`Delete ${entry.name}? This action cannot be undone.`,
			"Delete",
			true,
		);
		if (!confirmed) return;
		setError(null);
		try {
			await deleteEntry(entry.path, false);
		} catch (requestError) {
			if (!(entry.kind === "directory" && isConflict(requestError))) {
				setError(errorMessage(requestError, `Could not delete ${entry.name}`));
				return;
			}
			const recursive = await requestConfirmation(
				"Delete non-empty folder?",
				`${entry.name} is not empty. Delete the folder and everything inside it?`,
				"Delete all",
				true,
			);
			if (!recursive) return;
			try {
				await deleteEntry(entry.path, true);
			} catch (retryError) {
				setError(errorMessage(retryError, `Could not delete ${entry.name}`));
				return;
			}
		}
		setStatus(`Deleted ${entry.name}.`);
		refresh();
	}

	async function startDownload(entry: FileEntry): Promise<void> {
		setError(null);
		try {
			const picker = (window as unknown as SaveFilePickerWindow).showSaveFilePicker;
			if (picker) {
				const handle = await picker({ suggestedName: entry.name });
				const response = await downloadFile(entry.path);
				if (!response.body) throw new Error("The download response did not contain a file stream.");
				const writable = await handle.createWritable();
				await response.body.pipeTo(writable);
				return;
			}
			const response = await downloadFile(entry.path);
			const blob = await bufferDownload(response, MAX_BUFFERED_BROWSER_DOWNLOAD_BYTES);
			const url = URL.createObjectURL(blob);
			const link = document.createElement("a");
			link.className = "hidden";
			link.href = url;
			link.download = entry.name;
			document.body.append(link);
			link.click();
			link.remove();
			setTimeout(() => URL.revokeObjectURL(url), 0);
		} catch (requestError) {
			if (requestError instanceof Error && requestError.name === "AbortError") return;
			setError(errorMessage(requestError, `Could not download ${entry.name}`));
		}
	}

	function onDragEnter(event: DragEvent): void {
		event.preventDefault();
		dragDepth.current += 1;
		setDragging(true);
	}

	function onDragLeave(event: DragEvent): void {
		event.preventDefault();
		dragDepth.current = Math.max(0, dragDepth.current - 1);
		if (dragDepth.current === 0) setDragging(false);
	}

	async function onDrop(event: DragEvent): Promise<void> {
		event.preventDefault();
		dragDepth.current = 0;
		setDragging(false);
		const destination = currentPath;
		if (!event.dataTransfer || uploadActive.current) return;
		try {
			await uploadContents(await collectDroppedContents(event.dataTransfer), destination);
		} catch (requestError) {
			setError(errorMessage(requestError, "Could not read dropped files"));
		}
	}

	const displayedEntries = sortedEntries(entries, sort);
	const sortIndicator = (key: SortKey): string => {
		if (sort.key !== key) return "";
		return sort.direction === "ascending" ? " ^" : " v";
	};
	const ariaSort = (key: SortKey): SortDirection | undefined => (sort.key === key ? sort.direction : undefined);

	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: The full file browser is intentionally a drag-and-drop target.
		<div
			className={`relative flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto p-4 ${dragging ? "bg-[var(--accent-subtle)]" : ""}`}
			onDragEnter={onDragEnter}
			onDragOver={(event) => event.preventDefault()}
			onDragLeave={onDragLeave}
			onDrop={onDrop}
		>
			<SectionHeading title="Files" subtitle="Manage files shared with Moltis and its sandboxes." />
			<p className="max-w-3xl text-xs leading-relaxed text-[var(--muted)]">
				Files are available locally through <code>MOLTIS_FILES_DIR</code> and inside sandboxes at{" "}
				<code>/home/sandbox/files</code>. This browser shows relative paths only and never exposes the host path.
			</p>

			<div className="flex flex-wrap items-center gap-2" role="toolbar" aria-label="File actions">
				<button
					type="button"
					className="provider-btn"
					disabled={uploading}
					onClick={() => fileInputRef.current?.click()}
				>
					Upload Files
				</button>
				<button
					type="button"
					className="provider-btn provider-btn-secondary"
					disabled={uploading}
					onClick={() => folderInputRef.current?.click()}
				>
					Upload Folder
				</button>
				<button
					type="button"
					className="provider-btn provider-btn-secondary"
					onClick={() => setEditor({ kind: "new-folder", value: "" })}
				>
					New Folder
				</button>
				<button type="button" className="provider-btn provider-btn-secondary" onClick={refresh}>
					Refresh
				</button>
			</div>
			<input
				ref={fileInputRef}
				type="file"
				multiple
				className="hidden"
				aria-label="Choose files to upload"
				onChange={(event) => {
					const input = event.currentTarget;
					const destination = currentPath;
					if (input.files) void uploadContents(contentsFromFileList(input.files), destination);
					input.value = "";
				}}
			/>
			<input
				ref={folderInputRef}
				type="file"
				multiple
				className="hidden"
				aria-label="Choose folder to upload"
				onChange={(event) => {
					const input = event.currentTarget;
					const destination = currentPath;
					if (input.files) void uploadContents(contentsFromFileList(input.files), destination);
					input.value = "";
				}}
			/>

			<nav className="flex flex-wrap items-center gap-1 text-sm" aria-label="File path">
				<button type="button" className="text-[var(--accent)] hover:underline" onClick={() => setCurrentPath("")}>
					Files
				</button>
				{breadcrumbParts(currentPath).map((part) => (
					<span key={part.path} className="flex items-center gap-1">
						<span className="text-[var(--muted)]" aria-hidden="true">
							/
						</span>
						<button
							type="button"
							className="text-[var(--accent)] hover:underline"
							onClick={() => setCurrentPath(part.path)}
						>
							{part.label}
						</button>
					</span>
				))}
			</nav>

			{error ? (
				<div className="alert-error-text" role="alert" aria-live="assertive">
					{error}
				</div>
			) : null}
			{status ? (
				<div className="text-xs text-[var(--accent)]" role="status" aria-live="polite">
					{status}
				</div>
			) : null}

			<section
				className={`min-h-48 overflow-x-auto rounded-lg border border-[var(--border)] bg-[var(--surface)] ${dragging ? "border-[var(--accent)]" : ""}`}
				aria-label="Files table"
			>
				{dragging ? (
					<div className="pointer-events-none absolute inset-4 z-10 flex items-center justify-center rounded-lg border-2 border-dashed border-[var(--accent)] bg-[var(--surface)]/90 text-sm font-medium text-[var(--accent)]">
						Drop files and folders here
					</div>
				) : null}
				<table className="w-full min-w-[720px] border-collapse text-sm">
					<thead>
						<tr className="border-b border-[var(--border)] bg-[var(--bg)] text-left text-xs text-[var(--muted)]">
							<th className="px-4 py-3 font-medium" aria-sort={ariaSort("name")}>
								<button
									type="button"
									className="font-medium hover:text-[var(--text)]"
									onClick={() => setSortKey("name")}
								>
									Filename<span aria-hidden="true">{sortIndicator("name")}</span>
									<span className="sr-only">Sort by filename</span>
								</button>
							</th>
							<th className="px-4 py-3 font-medium" aria-sort={ariaSort("modifiedAt")}>
								<button
									type="button"
									className="font-medium hover:text-[var(--text)]"
									onClick={() => setSortKey("modifiedAt")}
								>
									Date Modified<span aria-hidden="true">{sortIndicator("modifiedAt")}</span>
									<span className="sr-only">Sort by date modified</span>
								</button>
							</th>
							<th className="px-4 py-3 font-medium" aria-sort={ariaSort("sizeBytes")}>
								<button
									type="button"
									className="font-medium hover:text-[var(--text)]"
									onClick={() => setSortKey("sizeBytes")}
								>
									Size<span aria-hidden="true">{sortIndicator("sizeBytes")}</span>
									<span className="sr-only">Sort by size</span>
								</button>
							</th>
							<th className="px-4 py-3 text-right font-medium">Actions</th>
						</tr>
					</thead>
					<tbody>
						{loading ? (
							<tr>
								<td colSpan={4} className="px-4 py-10 text-center text-sm text-[var(--muted)]">
									Loading files...
								</td>
							</tr>
						) : null}
						{!loading && displayedEntries.length === 0 ? (
							<tr>
								<td colSpan={4} className="px-4 py-10 text-center text-sm text-[var(--muted)]">
									This folder is empty. Upload files or drop them here.
								</td>
							</tr>
						) : null}
						{loading
							? null
							: displayedEntries.map(
									// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: Row controls vary intentionally by entry kind.
									(entry) => (
										<tr
											key={entry.path}
											className="border-b border-[var(--border)] last:border-0 hover:bg-[var(--bg-hover)]"
										>
											<td className="px-4 py-3 font-medium text-[var(--text-strong)]">
												<div className="flex items-center gap-2">
													<span
														className={`icon ${entry.kind === "directory" ? "icon-folder" : "icon-document"}`}
														aria-hidden="true"
													/>
													{entry.kind === "directory" ? (
														<button
															type="button"
															className="text-left hover:text-[var(--accent)] hover:underline"
															onClick={() => setCurrentPath(joinPath(currentPath, entry.name))}
														>
															{entry.name}
														</button>
													) : entry.kind === "file" ? (
														<button
															type="button"
															className="text-left hover:text-[var(--accent)] hover:underline"
															onClick={() => void startDownload(entry)}
														>
															{entry.name}
														</button>
													) : (
														<span>{entry.name}</span>
													)}
												</div>
											</td>
											<td className="whitespace-nowrap px-4 py-3 text-xs text-[var(--muted)]">
												{formatDate(entry.modifiedAt)}
											</td>
											<td className="whitespace-nowrap px-4 py-3 text-xs text-[var(--muted)]">
												{entry.kind === "directory" ? "-" : formatSize(entry.sizeBytes)}
											</td>
											<td className="px-4 py-3">
												<div className="flex justify-end gap-1">
													{entry.kind === "file" ? (
														<button
															type="button"
															className="provider-btn provider-btn-secondary provider-btn-sm"
															onClick={() => void startDownload(entry)}
															aria-label={`Download ${entry.name}`}
														>
															Download
														</button>
													) : null}
													{entry.kind === "other" ? null : (
														<>
															<button
																type="button"
																className="provider-btn provider-btn-secondary provider-btn-sm"
																onClick={() => setEditor({ kind: "rename", entry, value: entry.name })}
																aria-label={`Rename ${entry.name}`}
															>
																Rename
															</button>
															<button
																type="button"
																className="provider-btn provider-btn-secondary provider-btn-sm"
																onClick={() =>
																	setEditor({ kind: "move", entry, destination: currentPath, browsePath: currentPath })
																}
																aria-label={`Move ${entry.name}`}
															>
																Move
															</button>
														</>
													)}
													<button
														type="button"
														className="provider-btn provider-btn-danger provider-btn-sm"
														onClick={() => void removeEntry(entry)}
														aria-label={`Delete ${entry.name}`}
													>
														Delete
													</button>
												</div>
											</td>
										</tr>
									),
								)}
					</tbody>
				</table>
			</section>

			{editor ? (
				<EditorDialog
					state={editor}
					browseFolders={browseFolders}
					browseLoading={browseLoading}
					browseError={browseError}
					onChange={setEditor}
					onClose={() => setEditor(null)}
					onSubmit={() => void submitEditor()}
				/>
			) : null}
			{confirmation ? <ConfirmationDialog state={confirmation} onClose={closeConfirmation} /> : null}
		</div>
	);
}

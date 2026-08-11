import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import {
	Badge,
	EmptyState,
	Loading,
	SaveButton,
	SettingsCard,
	StatusMessage,
	TextAreaField,
} from "../../components/forms";
import { useTranslation } from "../../i18n";
import type {
	ConnectorAccount,
	ConnectorDataset,
	ConnectorDatasetCompileResponse,
	ConnectorDatasetDraft,
	ConnectorItem,
	JsonValue,
} from "../../types/connector";
import { ConfirmDialog, Modal, requestConfirm, showToast } from "../../ui";
import { connectorRpc } from "./rpc";

const PREVIEW_LIMIT = 25;
const PREVIEW_TEXT_LIMIT = 12_000;

interface DatasetsTabProps {
	accounts: ConnectorAccount[];
	datasets: ConnectorDataset[];
	onChanged: () => Promise<void>;
}

interface DatasetFormModalProps {
	accounts: ConnectorAccount[];
	dataset: ConnectorDataset | null;
	onClose: () => void;
	onSaved: () => Promise<void>;
}

function formatTimestamp(value?: string): string {
	if (!value) return "";
	const date = new Date(value);
	return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function currentDraft(dataset: ConnectorDataset): ConnectorDatasetDraft {
	return {
		name: dataset.name,
		config: dataset.config,
		...(dataset.scheduleMinutes === undefined ? {} : { scheduleMinutes: dataset.scheduleMinutes }),
		projections: dataset.projections,
		enabled: dataset.enabled,
	};
}

function parseOverrides(value: string): JsonValue | undefined {
	if (!value.trim()) return undefined;
	const parsed: unknown = JSON.parse(value);
	if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
		throw new Error("overrides-must-be-object");
	}
	return parsed as JsonValue;
}

interface DraftPreviewProps {
	preview: ConnectorDatasetCompileResponse;
	compiled: boolean;
}

function DraftPreview({ preview, compiled }: DraftPreviewProps): VNode {
	const { t } = useTranslation("connectors");
	const { draft } = preview;
	const selection = draft.config.selection;
	const filters = draft.config.filters;
	const calendars =
		selection.mode === "all" ? t("datasets.all") : selection.calendarHrefs.join(", ") || t("datasets.none");
	const dates = filters.startDate
		? filters.endDate
			? t("datasets.dateRange", { start: filters.startDate, end: filters.endDate })
			: t("datasets.startingDate", { date: filters.startDate })
		: filters.endDate
			? t("datasets.endingDate", { date: filters.endDate })
			: t("datasets.anyDate");
	const projections = [
		draft.projections.jsonl ? t("datasets.jsonl") : null,
		draft.projections.markdown ? t("datasets.markdown") : null,
	].filter((projection): projection is string => projection !== null);

	return (
		<section
			className="mb-3 rounded border border-[var(--border)] bg-[var(--surface)] p-3"
			aria-label={t("datasets.previewDraft")}
		>
			<div className="mb-2 flex flex-wrap items-center justify-between gap-2">
				<div className="text-sm font-medium text-[var(--text-strong)]">{t("datasets.previewDraft")}</div>
				{compiled ? <Badge label={t("datasets.readyToSave")} variant="configured" /> : null}
			</div>
			<p className="mb-3 text-sm text-[var(--text)]">{preview.summary}</p>
			<dl className="grid gap-x-4 gap-y-2 text-xs sm:grid-cols-[9rem_1fr]">
				<dt className="text-[var(--muted)]">{t("datasets.previewName")}</dt>
				<dd className="break-words text-[var(--text)]">{draft.name}</dd>
				<dt className="text-[var(--muted)]">{t("datasets.previewCalendars")}</dt>
				<dd className="break-words text-[var(--text)]">{calendars}</dd>
				<dt className="text-[var(--muted)]">{t("datasets.previewDates")}</dt>
				<dd className="text-[var(--text)]">{dates}</dd>
				<dt className="text-[var(--muted)]">{t("datasets.previewAccepted")}</dt>
				<dd className="text-[var(--text)]">
					{filters.acceptedByAccount ? t("datasets.acceptedOnly") : t("datasets.anyAcceptance")}
				</dd>
				<dt className="text-[var(--muted)]">{t("datasets.previewSchedule")}</dt>
				<dd className="text-[var(--text)]">
					{draft.scheduleMinutes ? t("datasets.everyMinutes", { count: draft.scheduleMinutes }) : t("datasets.manual")}
				</dd>
				<dt className="text-[var(--muted)]">{t("datasets.previewProjections")}</dt>
				<dd className="text-[var(--text)]">{projections.join(", ") || t("datasets.none")}</dd>
				<dt className="text-[var(--muted)]">{t("datasets.previewEnabled")}</dt>
				<dd className="text-[var(--text)]">{draft.enabled ? t("status.enabled") : t("status.disabled")}</dd>
			</dl>
			{preview.warnings.length > 0 ? (
				<div className="mt-3 rounded border border-[var(--warning)] p-2 text-xs text-[var(--warning)]" role="status">
					<div className="mb-1 font-medium">{t("datasets.previewWarnings")}</div>
					<ul className="list-disc space-y-1 pl-4">
						{preview.warnings.map((warning) => (
							<li key={warning}>{warning}</li>
						))}
					</ul>
				</div>
			) : null}
			{compiled ? null : <div className="mt-3 text-xs text-[var(--warning)]">{t("datasets.compileRequired")}</div>}
		</section>
	);
}

function DatasetFormModal({ accounts, dataset, onClose, onSaved }: DatasetFormModalProps): VNode {
	const { t } = useTranslation("connectors");
	const [accountId, setAccountId] = useState(dataset?.accountId ?? accounts[0]?.id ?? "");
	const [instruction, setInstruction] = useState(dataset?.instruction ?? "");
	const [advancedJson, setAdvancedJson] = useState("");
	const [preview, setPreview] = useState<ConnectorDatasetCompileResponse | null>(() =>
		dataset
			? {
					draft: currentDraft(dataset),
					summary: t("datasets.currentDraftSummary"),
					warnings: [],
				}
			: null,
	);
	const [compiled, setCompiled] = useState(false);
	const [compiling, setCompiling] = useState(false);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const revision = useRef(0);

	function invalidate(): void {
		revision.current += 1;
		setCompiled(false);
		setCompiling(false);
		setError(null);
	}

	async function compile(): Promise<void> {
		if (!(accountId && instruction.trim())) {
			setError(t("datasets.instructionRequired"));
			return;
		}
		let overrides: JsonValue | undefined;
		try {
			overrides = parseOverrides(advancedJson);
		} catch {
			setError(t("datasets.overridesInvalid"));
			return;
		}

		const compileRevision = revision.current + 1;
		revision.current = compileRevision;
		setCompiling(true);
		setCompiled(false);
		setError(null);
		try {
			const result = await connectorRpc("connectors.datasets.compile", {
				accountId,
				...(dataset ? { datasetId: dataset.id } : {}),
				instruction: instruction.trim(),
				...(overrides === undefined ? {} : { overrides }),
			});
			if (revision.current !== compileRevision) return;
			setPreview(result);
			setCompiled(true);
		} catch (caught: unknown) {
			if (revision.current === compileRevision) {
				setError(caught instanceof Error ? caught.message : String(caught));
			}
		} finally {
			if (revision.current === compileRevision) setCompiling(false);
		}
	}

	async function submit(event: Event): Promise<void> {
		event.preventDefault();
		if (!(compiled && preview)) return;
		const values = { instruction: instruction.trim(), ...preview.draft };
		setSaving(true);
		setError(null);
		let saved = false;
		try {
			if (dataset) {
				await connectorRpc("connectors.datasets.update", { id: dataset.id, ...values });
			} else {
				await connectorRpc("connectors.datasets.add", { accountId, ...values });
			}
			saved = true;
			await onSaved();
			showToast(t("datasets.saved"), "success");
			onClose();
		} catch (caught: unknown) {
			if (saved) {
				showToast(t("refreshFailed"), "error");
				onClose();
			} else {
				setError(caught instanceof Error ? caught.message : String(caught));
			}
		} finally {
			setSaving(false);
		}
	}

	return (
		<Modal show={true} onClose={onClose} title={dataset ? t("datasets.editTitle") : t("datasets.addTitle")}>
			<form onSubmit={submit} className="flex flex-col gap-1">
				<fieldset className="mb-3" disabled={Boolean(dataset)}>
					<legend className="mb-2 text-xs text-[var(--muted)]">{t("datasets.account")}</legend>
					<div className="grid gap-2 sm:grid-cols-2">
						{accounts.map((account) => (
							<button
								key={account.id}
								type="button"
								aria-pressed={accountId === account.id}
								className={`backend-card ${accountId === account.id ? "selected" : ""} block w-full p-3 text-left`}
								onClick={() => {
									if (account.id === accountId) return;
									setAccountId(account.id);
									invalidate();
								}}
							>
								<div className="text-sm font-medium text-[var(--text)]">{account.name}</div>
								<div className="mt-1 text-xs text-[var(--muted)]">{account.username}</div>
							</button>
						))}
					</div>
				</fieldset>
				<TextAreaField
					id="connector-dataset-instruction"
					label={t("datasets.instruction")}
					value={instruction}
					onInput={(value) => {
						setInstruction(value);
						invalidate();
					}}
					placeholder={t("datasets.instructionPlaceholder")}
					help={t("datasets.instructionHelp")}
					rows={5}
					required
				/>
				<div className="mb-3 rounded border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-xs text-[var(--muted)]">
					<div className="mb-1 font-medium text-[var(--text)]">{t("datasets.examples")}</div>
					<ul className="list-disc space-y-1 pl-4">
						<li>{t("datasets.exampleAll")}</li>
						<li>{t("datasets.exampleFiltered")}</li>
					</ul>
				</div>
				<details className="mb-3 rounded border border-[var(--border)] bg-[var(--bg)] p-3">
					<summary className="cursor-pointer text-sm font-medium text-[var(--text)]">{t("datasets.advanced")}</summary>
					<TextAreaField
						id="connector-dataset-overrides"
						label={t("datasets.overrides")}
						value={advancedJson}
						onInput={(value) => {
							setAdvancedJson(value);
							invalidate();
						}}
						placeholder={t("datasets.overridesPlaceholder")}
						help={t("datasets.overridesHelp")}
						rows={5}
						className="mt-3"
						monospace
					/>
				</details>
				<div className="mb-3 flex justify-start">
					<button
						type="button"
						className="provider-btn"
						disabled={compiling || !accountId || !instruction.trim()}
						onClick={() => void compile()}
					>
						{compiling ? t("datasets.compiling") : t("datasets.compile")}
					</button>
				</div>
				{preview ? <DraftPreview preview={preview} compiled={compiled} /> : null}
				<StatusMessage error={error} />
				<div className="mt-2 flex justify-end gap-2">
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onClose}>
						{t("cancel")}
					</button>
					<SaveButton
						type="submit"
						saving={saving}
						disabled={!(compiled && preview)}
						label={dataset ? t("datasets.save") : t("datasets.create")}
					/>
				</div>
			</form>
		</Modal>
	);
}

interface PreviewModalProps {
	dataset: ConnectorDataset;
	onClose: () => void;
}

function previewJson(item: ConnectorItem): string {
	const text = JSON.stringify(item.bodyJson, null, 2);
	return text.length <= PREVIEW_TEXT_LIMIT ? text : `${text.slice(0, PREVIEW_TEXT_LIMIT)}\n...`;
}

function PreviewModal({ dataset, onClose }: PreviewModalProps): VNode {
	const { t } = useTranslation("connectors");
	const [items, setItems] = useState<ConnectorItem[] | null>(null);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		let active = true;
		connectorRpc("connectors.items.query", {
			datasetId: dataset.id,
			limit: PREVIEW_LIMIT,
			offset: 0,
			includeDeleted: false,
		})
			.then((payload) => {
				if (active) setItems(payload.items);
			})
			.catch((caught: unknown) => {
				if (active) setError(caught instanceof Error ? caught.message : String(caught));
			});
		return () => {
			active = false;
		};
	}, [dataset.id]);

	return (
		<Modal show={true} onClose={onClose} title={t("datasets.previewTitle")}>
			<div className="flex flex-col gap-3">
				<div className="text-xs text-[var(--muted)]">{dataset.name}</div>
				{items || error ? null : <Loading />}
				<StatusMessage error={error} />
				{items?.length === 0 ? <EmptyState message={t("datasets.previewEmpty")} /> : null}
				<div className="flex max-h-[55vh] flex-col gap-3 overflow-y-auto">
					{items?.map((item) => (
						<div key={item.id} className="rounded border border-[var(--border)] bg-[var(--bg)] p-3">
							<div className="mb-2 flex flex-wrap items-center gap-2 text-xs text-[var(--muted)]">
								<Badge label={item.kind} />
								<span>{item.occurredAt || item.updatedAt || item.storedAt}</span>
							</div>
							<pre className="overflow-x-auto whitespace-pre-wrap break-words font-mono text-xs text-[var(--text)]">
								{previewJson(item)}
							</pre>
						</div>
					))}
				</div>
				<div className="flex justify-end">
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onClose}>
						{t("close")}
					</button>
				</div>
			</div>
		</Modal>
	);
}

export function DatasetsTab({ accounts, datasets, onChanged }: DatasetsTabProps): VNode {
	const { t } = useTranslation("connectors");
	const [editing, setEditing] = useState<ConnectorDataset | "new" | null>(null);
	const [previewing, setPreviewing] = useState<ConnectorDataset | null>(null);
	const [syncingId, setSyncingId] = useState<string | null>(null);

	async function sync(dataset: ConnectorDataset): Promise<void> {
		setSyncingId(dataset.id);
		let synchronized = false;
		try {
			await connectorRpc("connectors.datasets.sync", { id: dataset.id });
			synchronized = true;
			await onChanged();
			showToast(t("datasets.syncComplete"), "success");
		} catch (caught: unknown) {
			showToast(synchronized ? t("refreshFailed") : caught instanceof Error ? caught.message : String(caught), "error");
		} finally {
			setSyncingId(null);
		}
	}

	async function remove(dataset: ConnectorDataset): Promise<void> {
		const confirmed = await requestConfirm(t("datasets.removeConfirm", { name: dataset.name }), {
			confirmLabel: t("datasets.remove"),
			danger: true,
		});
		if (!confirmed) return;
		let removed = false;
		try {
			await connectorRpc("connectors.datasets.remove", { id: dataset.id });
			removed = true;
			await onChanged();
			showToast(t("datasets.removed"), "success");
		} catch (caught: unknown) {
			showToast(removed ? t("refreshFailed") : caught instanceof Error ? caught.message : String(caught), "error");
		}
	}

	return (
		<div className="flex flex-col gap-3">
			<div className="flex items-center justify-between gap-3">
				{accounts.length === 0 ? (
					<div className="text-xs text-[var(--muted)]">{t("datasets.needsAccount")}</div>
				) : (
					<span />
				)}
				<button
					type="button"
					className="provider-btn"
					disabled={accounts.length === 0}
					onClick={() => setEditing("new")}
				>
					{t("datasets.add")}
				</button>
			</div>
			{datasets.length === 0 ? <EmptyState message={t("datasets.empty")} /> : null}
			{datasets.map((dataset) => {
				const account = accounts.find((candidate) => candidate.id === dataset.accountId);
				const selection = dataset.config.selection;
				return (
					<SettingsCard key={dataset.id} className="bg-[var(--surface)] border border-[var(--border)] rounded-lg p-4">
						<div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
							<div className="min-w-0">
								<div className="flex flex-wrap items-center gap-2">
									<h3 className="text-sm font-medium text-[var(--text-strong)]">{dataset.name}</h3>
									<Badge
										label={dataset.enabled ? t("status.enabled") : t("status.disabled")}
										variant={dataset.enabled ? "configured" : "muted"}
									/>
									<Badge label={t("datasets.items", { count: dataset.itemCount })} />
									{dataset.needsSync ? <Badge label={t("datasets.needsSync")} variant="warning" /> : null}
								</div>
								<div className="mt-2 text-xs text-[var(--muted)]">
									{account?.name || dataset.accountId} ·{" "}
									{dataset.scheduleMinutes
										? t("datasets.everyMinutes", { count: dataset.scheduleMinutes })
										: t("datasets.manual")}{" "}
									·{" "}
									{selection.mode === "all"
										? t("datasets.all")
										: t("datasets.selected", { count: selection.calendarHrefs.length })}
								</div>
								<div className="mt-1 text-xs text-[var(--muted)]">
									{t("datasets.lastSync", {
										time: dataset.lastSyncAt ? formatTimestamp(dataset.lastSyncAt) : t("datasets.never"),
									})}
									{dataset.nextSyncAt
										? ` · ${t("datasets.nextSync", { time: formatTimestamp(dataset.nextSyncAt) })}`
										: ""}
								</div>
								<div className="mt-2 flex flex-wrap gap-2">
									{dataset.projections.jsonl ? <Badge label={t("datasets.jsonl")} /> : null}
									{dataset.projections.markdown ? <Badge label={t("datasets.markdown")} /> : null}
								</div>
								{dataset.projectionPath ? (
									<div className="mt-2 break-all font-mono text-xs text-[var(--muted)]">
										{t("datasets.outputPath", { path: dataset.projectionPath })}
									</div>
								) : null}
								{dataset.lastError ? <StatusMessage error={dataset.lastError} className="mt-2 text-xs" /> : null}
							</div>
							<div className="flex flex-wrap gap-2">
								<button
									type="button"
									className="provider-btn"
									disabled={syncingId !== null || !dataset.enabled || !account?.enabled || !account.hasPassword}
									onClick={() => void sync(dataset)}
								>
									{syncingId === dataset.id ? t("datasets.running") : t("datasets.runNow")}
								</button>
								<button
									type="button"
									className="provider-btn provider-btn-secondary"
									onClick={() => setPreviewing(dataset)}
								>
									{t("datasets.preview")}
								</button>
								<button
									type="button"
									className="provider-btn provider-btn-secondary"
									onClick={() => setEditing(dataset)}
								>
									{t("datasets.edit")}
								</button>
								<button type="button" className="provider-btn provider-btn-danger" onClick={() => void remove(dataset)}>
									{t("datasets.remove")}
								</button>
							</div>
						</div>
					</SettingsCard>
				);
			})}
			{editing ? (
				<DatasetFormModal
					key={editing === "new" ? "new" : editing.id}
					accounts={accounts}
					dataset={editing === "new" ? null : editing}
					onClose={() => setEditing(null)}
					onSaved={onChanged}
				/>
			) : null}
			{previewing ? <PreviewModal dataset={previewing} onClose={() => setPreviewing(null)} /> : null}
			<ConfirmDialog />
		</div>
	);
}

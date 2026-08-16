import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import {
	Badge,
	CheckboxField,
	EmptyState,
	Loading,
	SaveButton,
	SettingsCard,
	StatusMessage,
	TextAreaField,
	TextField,
} from "../../components/forms";
import { useTranslation } from "../../i18n";
import type {
	CalDavConnectorDataset,
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

function currentDraft(dataset: CalDavConnectorDataset): ConnectorDatasetDraft {
	return {
		name: dataset.name,
		config: dataset.config,
		...(dataset.scheduleMinutes === undefined ? {} : { scheduleMinutes: dataset.scheduleMinutes }),
		projections: dataset.projections,
		enabled: dataset.enabled,
	};
}

interface AccountSelectorProps {
	accounts: ConnectorAccount[];
	accountId: string;
	disabled: boolean;
	onChange: (accountId: string) => void;
}

function AccountSelector({ accounts, accountId, disabled, onChange }: AccountSelectorProps): VNode {
	const { t } = useTranslation("connectors");
	return (
		<fieldset className="mb-3" disabled={disabled}>
			<legend className="mb-2 text-xs text-[var(--muted)]">{t("datasets.account")}</legend>
			<div className="grid gap-2 sm:grid-cols-2">
				{accounts.map((account) => (
					<button
						key={account.id}
						type="button"
						aria-pressed={accountId === account.id}
						className={`backend-card ${accountId === account.id ? "selected" : ""} block w-full p-3 text-left`}
						onClick={() => onChange(account.id)}
					>
						<div className="text-sm font-medium text-[var(--text)]">{account.name}</div>
						<div className="mt-1 text-xs text-[var(--muted)]">
							{account.kind === "caldav"
								? account.username
								: account.kind === "channel_history"
									? `${account.channelType} · ${account.channelAccountId}`
									: account.kind === "gmail"
										? "Google Workspace"
										: `${account.himalayaBackend} · ${account.himalayaAccountName}`}
						</div>
					</button>
				))}
			</div>
		</fieldset>
	);
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
	const [channelId, setChannelId] = useState(dataset?.kind === "channel_history" ? dataset.config.channelId : "");
	const [threadId, setThreadId] = useState(dataset?.kind === "channel_history" ? dataset.config.threadId : "");
	const [limit, setLimit] = useState(dataset?.kind === "channel_history" ? String(dataset.config.limit) : "100");
	const [emailQuery, setEmailQuery] = useState(
		dataset?.kind === "gmail" ? dataset.config.query : dataset?.kind === "himalaya" ? (dataset.config.query ?? "") : "",
	);
	const [mailboxes, setMailboxes] = useState(
		dataset?.kind === "himalaya" ? dataset.config.mailboxIds.join("\n") : "INBOX",
	);
	const [maxMessages, setMaxMessages] = useState(
		dataset?.kind === "gmail" || dataset?.kind === "himalaya" ? String(dataset.config.maxMessages) : "500",
	);
	const [includeBodies, setIncludeBodies] = useState(
		dataset?.kind === "gmail"
			? dataset.config.includeBody
			: dataset?.kind === "himalaya"
				? dataset.config.includeBodies
				: true,
	);
	const [name, setName] = useState(dataset?.name ?? "");
	const [schedule, setSchedule] = useState(
		dataset?.scheduleMinutes === null || dataset?.scheduleMinutes === undefined ? "" : String(dataset.scheduleMinutes),
	);
	const [jsonl, setJsonl] = useState(dataset?.projections.jsonl ?? true);
	const [markdown, setMarkdown] = useState(dataset?.projections.markdown ?? true);
	const [enabled, setEnabled] = useState(dataset?.enabled ?? true);
	const [advancedJson, setAdvancedJson] = useState("");
	const [preview, setPreview] = useState<ConnectorDatasetCompileResponse | null>(() =>
		dataset?.kind === "caldav"
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
	const selectedAccount = accounts.find((account) => account.id === accountId);
	const channelHistory = dataset?.kind === "channel_history" || selectedAccount?.kind === "channel_history";
	const emailDataset =
		dataset?.kind === "gmail" ||
		dataset?.kind === "himalaya" ||
		selectedAccount?.kind === "gmail" ||
		selectedAccount?.kind === "himalaya";

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
		if (channelHistory) {
			if (!(name.trim() && channelId.trim() && threadId.trim())) {
				setError(t("datasets.channelRequired"));
				return;
			}
			const parsedLimit = Number(limit);
			if (!(Number.isSafeInteger(parsedLimit) && parsedLimit >= 1 && parsedLimit <= 200)) {
				setError(t("datasets.limitInvalid"));
				return;
			}
			const scheduleMinutes = schedule.trim() ? Number(schedule) : null;
			if (scheduleMinutes !== null && !(Number.isSafeInteger(scheduleMinutes) && scheduleMinutes > 0)) {
				setError(t("datasets.scheduleInvalid"));
				return;
			}
			setSaving(true);
			setError(null);
			let saved = false;
			try {
				const values = {
					instruction: `Keep up to ${parsedLimit} messages from channel ${channelId.trim()} in thread ${threadId.trim()}.`,
					name: name.trim(),
					config: {
						schemaVersion: dataset?.kind === "channel_history" ? dataset.config.schemaVersion : 1,
						channelId: channelId.trim(),
						threadId: threadId.trim(),
						limit: parsedLimit,
					},
					scheduleMinutes,
					projections: { jsonl, markdown },
					enabled,
				};
				if (dataset) await connectorRpc("connectors.datasets.update", { id: dataset.id, ...values });
				else await connectorRpc("connectors.datasets.add", { accountId, ...values });
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
			return;
		}
		if (emailDataset) {
			if (
				!(name.trim() && selectedAccount && (selectedAccount.kind === "gmail" || selectedAccount.kind === "himalaya"))
			) {
				setError(t("datasets.emailRequired"));
				return;
			}
			const parsedMax = Number(maxMessages);
			if (!(Number.isSafeInteger(parsedMax) && parsedMax >= 1 && parsedMax <= 1000)) {
				setError(t("datasets.emailLimitInvalid"));
				return;
			}
			const scheduleMinutes = schedule.trim() ? Number(schedule) : null;
			if (scheduleMinutes !== null && !(Number.isSafeInteger(scheduleMinutes) && scheduleMinutes > 0)) {
				setError(t("datasets.scheduleInvalid"));
				return;
			}
			const mailboxIds = mailboxes
				.split(/\r?\n/)
				.map((value) => value.trim())
				.filter(Boolean);
			if (selectedAccount.kind === "himalaya" && (mailboxIds.length === 0 || mailboxIds.length > 32)) {
				setError(t("datasets.mailboxesInvalid"));
				return;
			}
			const supportsQuery =
				selectedAccount.kind !== "himalaya" ||
				(selectedAccount.himalayaBackend !== "gmail" && selectedAccount.himalayaBackend !== "msgraph");
			const config =
				selectedAccount.kind === "gmail"
					? { schemaVersion: 1, query: emailQuery.trim(), maxMessages: parsedMax, includeBody: includeBodies }
					: {
							schemaVersion: 1,
							mailboxIds,
							...(supportsQuery && emailQuery.trim() ? { query: emailQuery.trim() } : {}),
							maxMessages: parsedMax,
							includeBodies,
						};
			setSaving(true);
			setError(null);
			try {
				const values = {
					instruction:
						selectedAccount.kind === "gmail"
							? `Keep up to ${parsedMax} Gmail messages matching ${emailQuery.trim() || "all mail"}.`
							: `Keep up to ${parsedMax} messages from ${mailboxIds.join(", ")}.`,
					name: name.trim(),
					config,
					scheduleMinutes,
					projections: { jsonl, markdown },
					enabled,
				};
				if (dataset) await connectorRpc("connectors.datasets.update", { id: dataset.id, ...values });
				else await connectorRpc("connectors.datasets.add", { accountId, ...values });
				await onSaved();
				showToast(t("datasets.saved"), "success");
				onClose();
			} catch (caught: unknown) {
				setError(caught instanceof Error ? caught.message : String(caught));
			} finally {
				setSaving(false);
			}
			return;
		}
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
		<Modal
			show={true}
			onClose={onClose}
			title={
				channelHistory
					? dataset
						? t("datasets.editChannelTitle")
						: t("datasets.addChannelTitle")
					: emailDataset
						? dataset
							? t("datasets.editEmailTitle")
							: t("datasets.addEmailTitle")
						: dataset
							? t("datasets.editTitle")
							: t("datasets.addTitle")
			}
		>
			<form onSubmit={submit} className="flex flex-col gap-1">
				<AccountSelector
					accounts={accounts}
					accountId={accountId}
					disabled={Boolean(dataset)}
					onChange={(nextAccountId) => {
						if (nextAccountId === accountId) return;
						setAccountId(nextAccountId);
						invalidate();
					}}
				/>
				{channelHistory ? (
					<>
						<div className="mb-3 rounded border border-[var(--border)] bg-[var(--bg)] p-3 text-xs text-[var(--muted)]">
							{t("datasets.channelHelp")}
						</div>
						<TextField
							id="connector-channel-dataset-name"
							label={t("datasets.name")}
							value={name}
							onInput={setName}
							placeholder={t("datasets.channelNamePlaceholder")}
							required
						/>
						<TextField
							id="connector-channel-id"
							label={t("datasets.channelId")}
							value={channelId}
							onInput={setChannelId}
							placeholder={t("datasets.channelIdPlaceholder")}
							help={t("datasets.channelIdHelp")}
							required
						/>
						<TextField
							id="connector-thread-id"
							label={t("datasets.threadId")}
							value={threadId}
							onInput={setThreadId}
							placeholder={t("datasets.threadIdPlaceholder")}
							help={t("datasets.threadIdHelp")}
							required
						/>
						<div className="grid gap-3 sm:grid-cols-2">
							<TextField
								id="connector-message-limit"
								label={t("datasets.limit")}
								type="number"
								value={limit}
								onInput={setLimit}
								help={t("datasets.limitHelp")}
								required
							/>
							<TextField
								id="connector-channel-schedule"
								label={t("datasets.schedule")}
								type="number"
								value={schedule}
								onInput={setSchedule}
								help={t("datasets.scheduleHelp")}
							/>
						</div>
						<fieldset className="mb-2">
							<legend className="mb-2 text-xs text-[var(--muted)]">{t("datasets.projections")}</legend>
							<div className="flex flex-wrap gap-4">
								<CheckboxField label={t("datasets.jsonl")} checked={jsonl} onChange={setJsonl} />
								<CheckboxField label={t("datasets.markdown")} checked={markdown} onChange={setMarkdown} />
							</div>
						</fieldset>
						<CheckboxField label={t("datasets.enabled")} checked={enabled} onChange={setEnabled} />
					</>
				) : emailDataset ? (
					<>
						<div className="mb-3 rounded border border-[var(--border)] bg-[var(--bg)] p-3 text-xs text-[var(--muted)]">
							{t("datasets.emailHelp")}
						</div>
						<TextField
							id="connector-email-dataset-name"
							label={t("datasets.name")}
							value={name}
							onInput={setName}
							required
						/>
						{selectedAccount?.kind === "himalaya" ? (
							<TextAreaField
								id="connector-email-mailboxes"
								label={t("datasets.mailboxes")}
								value={mailboxes}
								onInput={setMailboxes}
								help={t("datasets.mailboxesHelp")}
								required
							/>
						) : null}
						{selectedAccount?.kind !== "himalaya" ||
						selectedAccount.himalayaBackend === "imap" ||
						selectedAccount.himalayaBackend === "jmap" ||
						selectedAccount.himalayaBackend === "maildir" ||
						selectedAccount.himalayaBackend === "m2dir" ? (
							<TextField
								id="connector-email-query"
								label={t("datasets.emailQuery")}
								value={emailQuery}
								onInput={setEmailQuery}
								help={t("datasets.emailQueryHelp")}
							/>
						) : null}
						<div className="grid gap-3 sm:grid-cols-2">
							<TextField
								id="connector-email-limit"
								label={t("datasets.emailLimit")}
								type="number"
								value={maxMessages}
								onInput={setMaxMessages}
								required
							/>
							<TextField
								id="connector-email-schedule"
								label={t("datasets.schedule")}
								type="number"
								value={schedule}
								onInput={setSchedule}
								help={t("datasets.scheduleHelp")}
							/>
						</div>
						<CheckboxField label={t("datasets.includeBodies")} checked={includeBodies} onChange={setIncludeBodies} />
						<fieldset className="mb-2">
							<legend className="mb-2 text-xs text-[var(--muted)]">{t("datasets.projections")}</legend>
							<div className="flex flex-wrap gap-4">
								<CheckboxField label={t("datasets.jsonl")} checked={jsonl} onChange={setJsonl} />
								<CheckboxField label={t("datasets.markdown")} checked={markdown} onChange={setMarkdown} />
							</div>
						</fieldset>
						<CheckboxField label={t("datasets.enabled")} checked={enabled} onChange={setEnabled} />
					</>
				) : (
					<>
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
							<summary className="cursor-pointer text-sm font-medium text-[var(--text)]">
								{t("datasets.advanced")}
							</summary>
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
					</>
				)}
				<StatusMessage error={error} />
				<div className="mt-2 flex justify-end gap-2">
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onClose}>
						{t("cancel")}
					</button>
					<SaveButton
						type="submit"
						saving={saving}
						disabled={channelHistory || emailDataset ? !accountId : !(compiled && preview)}
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
				const scope =
					dataset.kind === "caldav"
						? dataset.config.selection.mode === "all"
							? t("datasets.all")
							: t("datasets.selected", { count: dataset.config.selection.calendarHrefs.length })
						: dataset.kind === "channel_history"
							? t("datasets.channelScope", {
									channelId: dataset.config.channelId,
									threadId: dataset.config.threadId,
									limit: dataset.config.limit,
								})
							: dataset.kind === "gmail"
								? t("datasets.gmailScope", {
										query: dataset.config.query || t("datasets.allMail"),
										limit: dataset.config.maxMessages,
									})
								: t("datasets.himalayaScope", {
										mailboxes: dataset.config.mailboxIds.join(", "),
										limit: dataset.config.maxMessages,
									});
				const canRun = account?.enabled === true && (account.kind !== "caldav" || account.hasPassword);
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
									· {scope}
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
									disabled={syncingId !== null || !dataset.enabled || !canRun}
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

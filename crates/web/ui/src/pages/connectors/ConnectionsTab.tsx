import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import {
	Badge,
	CheckboxField,
	EmptyState,
	Loading,
	SaveButton,
	SelectField,
	SettingsCard,
	StatusMessage,
	TextField,
} from "../../components/forms";
import { useTranslation } from "../../i18n";
import type {
	CalDavConnectorAccount,
	ChannelHistoryConnectorAccount,
	ConnectorAccount,
	ConnectorCalendar,
	ConnectorChannelSource,
	GmailConnectorAccount,
	HimalayaBackend,
	HimalayaConnectorAccount,
} from "../../types/connector";
import { ConfirmDialog, Modal, requestConfirm, showToast } from "../../ui";
import { connectorRpc } from "./rpc";

interface ConnectionsTabProps {
	accounts: ConnectorAccount[];
	caldavAvailable: boolean;
	channelHistoryAvailable: boolean;
	gmailAvailable: boolean;
	himalayaAvailable: boolean;
	channelSources: ConnectorChannelSource[];
	onChanged: () => Promise<void>;
}

interface EmailConnectionFormModalProps {
	kind: "gmail" | "himalaya";
	account: GmailConnectorAccount | HimalayaConnectorAccount | null;
	onClose: () => void;
	onSaved: () => Promise<void>;
}

const HIMALAYA_BACKENDS: Array<{ value: HimalayaBackend; label: string }> = [
	{ value: "imap", label: "IMAP" },
	{ value: "jmap", label: "JMAP" },
	{ value: "gmail", label: "Gmail API" },
	{ value: "msgraph", label: "Microsoft Graph" },
	{ value: "maildir", label: "Maildir" },
	{ value: "m2dir", label: "M2dir" },
];

function EmailConnectionFormModal({ kind, account, onClose, onSaved }: EmailConnectionFormModalProps): VNode {
	const { t } = useTranslation("connectors");
	const [name, setName] = useState(account?.name ?? "");
	const himalaya = account?.kind === "himalaya" ? account : null;
	const [accountName, setAccountName] = useState(himalaya?.himalayaAccountName ?? "");
	const [backend, setBackend] = useState<HimalayaBackend>(himalaya?.himalayaBackend ?? "imap");
	const [enabled, setEnabled] = useState(account?.enabled ?? true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	async function submit(event: Event): Promise<void> {
		event.preventDefault();
		if (!name.trim() || (kind === "himalaya" && !accountName.trim())) {
			setError(t("connections.emailRequired"));
			return;
		}
		setSaving(true);
		setError(null);
		try {
			if (account) {
				await connectorRpc("connectors.accounts.update", {
					id: account.id,
					name: name.trim(),
					enabled,
				});
			} else if (kind === "gmail") {
				await connectorRpc("connectors.accounts.add", { kind, name: name.trim(), enabled });
			} else {
				await connectorRpc("connectors.accounts.add", {
					kind,
					name: name.trim(),
					himalayaAccountName: accountName.trim(),
					himalayaBackend: backend,
					enabled,
				});
			}
			await onSaved();
			showToast(t("connections.saved"), "success");
			onClose();
		} catch (caught: unknown) {
			setError(caught instanceof Error ? caught.message : String(caught));
		} finally {
			setSaving(false);
		}
	}

	return (
		<Modal
			show={true}
			onClose={onClose}
			title={t(kind === "gmail" ? "connections.addGmailTitle" : "connections.addHimalayaTitle")}
		>
			<form onSubmit={submit} className="flex flex-col gap-1">
				<TextField
					id="connector-email-connection-name"
					label={t("connections.name")}
					value={name}
					onInput={setName}
					placeholder={t("connections.emailNamePlaceholder")}
					required
				/>
				{kind === "himalaya" ? (
					<>
						<TextField
							id="connector-himalaya-account"
							label={t("connections.himalayaAccount")}
							value={accountName}
							onInput={setAccountName}
							required
							disabled={Boolean(account)}
						/>
						<SelectField
							id="connector-himalaya-backend"
							label={t("connections.himalayaBackend")}
							value={backend}
							onChange={(value) => setBackend(value as HimalayaBackend)}
							options={HIMALAYA_BACKENDS}
							disabled={Boolean(account)}
						/>
					</>
				) : (
					<div className="mb-3 rounded border border-[var(--border)] bg-[var(--bg)] p-3 text-xs text-[var(--muted)]">
						{t("connections.gmailCredentialHelp")}
					</div>
				)}
				<CheckboxField label={t("connections.enabled")} checked={enabled} onChange={setEnabled} />
				<StatusMessage error={error} />
				<div className="mt-2 flex justify-end gap-2">
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onClose}>
						{t("cancel")}
					</button>
					<SaveButton type="submit" saving={saving} label={account ? t("connections.save") : t("connections.create")} />
				</div>
			</form>
		</Modal>
	);
}

interface ConnectionFormModalProps {
	account: CalDavConnectorAccount | null;
	onClose: () => void;
	onSaved: () => Promise<void>;
}

enum HttpProtocol {
	Http = "http:",
	Https = "https:",
}

function parseHttpProtocol(protocol: string): HttpProtocol | null {
	switch (protocol) {
		case HttpProtocol.Http:
			return HttpProtocol.Http;
		case HttpProtocol.Https:
			return HttpProtocol.Https;
		default:
			return null;
	}
}

function safeServerUrl(value: string): string {
	try {
		const url = new URL(value);
		url.username = "";
		url.password = "";
		url.search = "";
		url.hash = "";
		return url.toString();
	} catch {
		return "Invalid server URL";
	}
}

function ConnectionFormModal({ account, onClose, onSaved }: ConnectionFormModalProps): VNode {
	const { t } = useTranslation("connectors");
	const [name, setName] = useState(account?.name ?? "");
	const [serverUrl, setServerUrl] = useState(account?.serverUrl ?? "https://");
	const [username, setUsername] = useState(account?.username ?? "");
	const [password, setPassword] = useState("");
	const [timeout, setTimeoutValue] = useState(String(account?.timeoutSeconds ?? 30));
	const [allowInsecureHttp, setAllowInsecureHttp] = useState(account?.allowInsecureHttp ?? false);
	const [allowPrivateNetwork, setAllowPrivateNetwork] = useState(account?.allowPrivateNetwork ?? false);
	const [enabled, setEnabled] = useState(account?.enabled ?? true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const managed = account?.managed === true;

	function validate(): number | null {
		if (!(name.trim() && serverUrl.trim() && username.trim() && (account || password))) {
			setError(t("connections.required"));
			return null;
		}
		let parsed: URL;
		try {
			parsed = new URL(serverUrl.trim());
		} catch {
			setError(t("connections.invalidUrl"));
			return null;
		}
		const protocol = parseHttpProtocol(parsed.protocol);
		if (protocol === null) {
			setError(t("connections.invalidUrl"));
			return null;
		}
		if (parsed.search || parsed.hash) {
			setError(t("connections.urlSuffixUnsupported"));
			return null;
		}
		if (protocol === HttpProtocol.Http && !allowInsecureHttp) {
			setError(t("connections.httpsRequired"));
			return null;
		}
		const timeoutSeconds = Number(timeout);
		if (!(Number.isSafeInteger(timeoutSeconds) && timeoutSeconds > 0 && timeoutSeconds <= 300)) {
			setError(t("connections.timeoutInvalid"));
			return null;
		}
		return timeoutSeconds;
	}

	async function submit(event: Event): Promise<void> {
		event.preventDefault();
		const timeoutSeconds = validate();
		if (timeoutSeconds === null) return;
		setSaving(true);
		setError(null);
		let saved = false;
		try {
			if (account) {
				await connectorRpc("connectors.accounts.update", {
					id: account.id,
					name: name.trim(),
					serverUrl: serverUrl.trim(),
					username: username.trim(),
					...(password ? { password } : {}),
					timeoutSeconds,
					allowInsecureHttp,
					allowPrivateNetwork,
					enabled,
				});
			} else {
				await connectorRpc("connectors.accounts.add", {
					kind: "caldav",
					name: name.trim(),
					serverUrl: serverUrl.trim(),
					username: username.trim(),
					password,
					timeoutSeconds,
					allowInsecureHttp,
					allowPrivateNetwork,
					enabled,
				});
			}
			saved = true;
			await onSaved();
			showToast(t("connections.saved"), "success");
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
		<Modal show={true} onClose={onClose} title={account ? t("connections.editTitle") : t("connections.addTitle")}>
			<form onSubmit={submit} className="flex flex-col gap-1">
				{managed ? (
					<div className="mb-3 rounded border border-[var(--border)] bg-[var(--bg)] p-3 text-xs text-[var(--muted)]">
						{t("connections.managedHelp")}
					</div>
				) : null}
				<TextField
					id="connector-connection-name"
					label={t("connections.name")}
					value={name}
					onInput={setName}
					placeholder={t("connections.namePlaceholder")}
					required
					disabled={managed}
				/>
				<TextField
					id="connector-server-url"
					label={t("connections.serverUrl")}
					value={serverUrl}
					onInput={setServerUrl}
					placeholder="https://caldav.example.com"
					help={t("connections.serverHelp")}
					autoComplete="url"
					required
					disabled={managed}
				/>
				<TextField
					id="connector-username"
					label={t("connections.username")}
					value={username}
					onInput={setUsername}
					autoComplete="username"
					required
					disabled={managed}
				/>
				<TextField
					id="connector-password"
					label={t("connections.password")}
					type="password"
					value={password}
					onInput={setPassword}
					autoComplete="new-password"
					help={account ? t("connections.passwordEditHelp") : undefined}
					required={!account}
					disabled={managed}
				/>
				<CheckboxField label={t("connections.enabled")} checked={enabled} onChange={setEnabled} disabled={managed} />
				<details className="mb-3 rounded border border-[var(--border)] bg-[var(--bg)] p-3">
					<summary className="cursor-pointer text-sm font-medium text-[var(--text)]">
						{t("connections.advanced")}
					</summary>
					<div className="mt-3">
						<TextField
							id="connector-timeout"
							label={t("connections.timeout")}
							type="number"
							value={timeout}
							onInput={setTimeoutValue}
							disabled={managed}
						/>
						<CheckboxField
							label={t("connections.allowHttp")}
							checked={allowInsecureHttp}
							onChange={setAllowInsecureHttp}
						/>
						<CheckboxField
							label={t("connections.allowPrivate")}
							checked={allowPrivateNetwork}
							onChange={setAllowPrivateNetwork}
						/>
					</div>
				</details>
				<StatusMessage error={error} />
				<div className="mt-2 flex justify-end gap-2">
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onClose}>
						{t("cancel")}
					</button>
					<SaveButton type="submit" saving={saving} label={account ? t("connections.save") : t("connections.create")} />
				</div>
			</form>
		</Modal>
	);
}

interface ChannelConnectionFormModalProps {
	account: ChannelHistoryConnectorAccount | null;
	sources: ConnectorChannelSource[];
	onClose: () => void;
	onSaved: () => Promise<void>;
}

function ChannelConnectionFormModal({ account, sources, onClose, onSaved }: ChannelConnectionFormModalProps): VNode {
	const { t } = useTranslation("connectors");
	const initialSource = account
		? sources.find(
				(source) => source.channelType === account.channelType && source.accountId === account.channelAccountId,
			)
		: sources[0];
	const [name, setName] = useState(account?.name ?? "");
	const [sourceKey, setSourceKey] = useState(
		initialSource ? `${initialSource.channelType}:${initialSource.accountId}` : "",
	);
	const [enabled, setEnabled] = useState(account?.enabled ?? true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	async function submit(event: Event): Promise<void> {
		event.preventDefault();
		const source = sources.find((candidate) => `${candidate.channelType}:${candidate.accountId}` === sourceKey);
		if (!(name.trim() && (account || source))) {
			setError(t("connections.channelRequired"));
			return;
		}
		setSaving(true);
		setError(null);
		let saved = false;
		try {
			if (account) {
				await connectorRpc("connectors.accounts.update", { id: account.id, name: name.trim(), enabled });
			} else if (source) {
				await connectorRpc("connectors.accounts.add", {
					kind: "channel_history",
					name: name.trim(),
					channelType: source.channelType,
					channelAccountId: source.accountId,
					enabled,
				});
			}
			saved = true;
			await onSaved();
			showToast(t("connections.saved"), "success");
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
			title={account ? t("connections.editChannelTitle") : t("connections.addChannelTitle")}
		>
			<form onSubmit={submit} className="flex flex-col gap-1">
				<TextField
					id="connector-channel-connection-name"
					label={t("connections.name")}
					value={name}
					onInput={setName}
					placeholder={t("connections.channelNamePlaceholder")}
					required
				/>
				<fieldset className="mb-3" disabled={Boolean(account)}>
					<legend className="mb-2 text-xs text-[var(--muted)]">{t("connections.channelSource")}</legend>
					<div className="grid gap-2 sm:grid-cols-2">
						{sources.map((source) => {
							const key = `${source.channelType}:${source.accountId}`;
							return (
								<button
									key={key}
									type="button"
									aria-pressed={sourceKey === key}
									className={`backend-card ${sourceKey === key ? "selected" : ""} block w-full p-3 text-left`}
									onClick={() => setSourceKey(key)}
								>
									<div className="text-sm font-medium text-[var(--text)]">{source.displayName}</div>
									<div className="mt-1 text-xs text-[var(--muted)]">
										{source.channelType} · {source.accountId}
									</div>
								</button>
							);
						})}
					</div>
					{sources.length === 0 ? <EmptyState message={t("connections.noChannelSources")} /> : null}
				</fieldset>
				<div className="mb-3 rounded border border-[var(--border)] bg-[var(--bg)] p-3 text-xs text-[var(--muted)]">
					{t("connections.channelSourceHelp")}
				</div>
				<CheckboxField label={t("connections.enabled")} checked={enabled} onChange={setEnabled} />
				<StatusMessage error={error} />
				<div className="mt-2 flex justify-end gap-2">
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onClose}>
						{t("cancel")}
					</button>
					<SaveButton
						type="submit"
						saving={saving}
						disabled={!account && sources.length === 0}
						label={account ? t("connections.save") : t("connections.create")}
					/>
				</div>
			</form>
		</Modal>
	);
}

interface TestConnectionModalProps {
	account: ConnectorAccount;
	onClose: () => void;
}

function TestConnectionModal({ account, onClose }: TestConnectionModalProps): VNode {
	const { t } = useTranslation("connectors");
	const [calendars, setCalendars] = useState<ConnectorCalendar[] | null>(null);
	const [channelReady, setChannelReady] = useState(false);
	const [emailReady, setEmailReady] = useState(false);
	const [emailAddress, setEmailAddress] = useState<string | undefined>();
	const [mailboxes, setMailboxes] = useState<Array<{ id: string; displayName?: string }>>([]);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		let active = true;
		connectorRpc("connectors.accounts.test", { id: account.id })
			.then((payload) => {
				if (!active) return;
				if ("calendars" in payload) setCalendars(payload.calendars);
				else if ("channelReady" in payload) setChannelReady(payload.channelReady);
				else {
					setEmailReady(payload.emailReady);
					setEmailAddress(payload.emailAddress);
					setMailboxes(payload.mailboxes ?? []);
				}
			})
			.catch((caught: unknown) => {
				if (active) setError(caught instanceof Error ? caught.message : String(caught));
			});
		return () => {
			active = false;
		};
	}, [account.id]);

	return (
		<Modal
			show={true}
			onClose={onClose}
			title={
				account.kind === "caldav"
					? t("connections.testTitle")
					: account.kind === "channel_history"
						? t("connections.testChannelTitle")
						: t("connections.testEmailTitle")
			}
		>
			<div className="flex flex-col gap-3">
				<div className="text-xs text-[var(--muted)]">{account.name}</div>
				{calendars || channelReady || emailReady || error ? null : <Loading message={t("connections.testing")} />}
				<StatusMessage error={error} />
				{channelReady ? <StatusMessage success={t("connections.channelReady")} /> : null}
				{emailReady ? (
					<StatusMessage
						success={
							emailAddress ? t("connections.emailReadyAddress", { email: emailAddress }) : t("connections.emailReady")
						}
					/>
				) : null}
				{mailboxes.map((mailbox) => (
					<div
						key={mailbox.id}
						className="rounded border border-[var(--border)] bg-[var(--bg)] p-3 text-sm text-[var(--text)]"
					>
						{mailbox.displayName || mailbox.id}
					</div>
				))}
				{calendars?.length === 0 ? <EmptyState message={t("connections.noCalendars")} /> : null}
				{calendars?.map((calendar) => (
					<div key={calendar.href} className="rounded border border-[var(--border)] bg-[var(--bg)] p-3">
						<div className="flex items-center justify-between gap-2">
							<div className="text-sm font-medium text-[var(--text)]">{calendar.displayName || calendar.href}</div>
							{calendar.supportsSync ? <Badge label={t("connections.supportsSync")} variant="configured" /> : null}
						</div>
						{calendar.description ? (
							<div className="mt-1 text-xs text-[var(--muted)]">{calendar.description}</div>
						) : null}
					</div>
				))}
				<div className="flex justify-end">
					<button type="button" className="provider-btn provider-btn-secondary" onClick={onClose}>
						{t("close")}
					</button>
				</div>
			</div>
		</Modal>
	);
}

export function ConnectionsTab({
	accounts,
	caldavAvailable,
	channelHistoryAvailable,
	gmailAvailable,
	himalayaAvailable,
	channelSources,
	onChanged,
}: ConnectionsTabProps): VNode {
	const { t } = useTranslation("connectors");
	const [editing, setEditing] = useState<ConnectorAccount | "caldav" | "channel_history" | "gmail" | "himalaya" | null>(
		null,
	);
	const [testing, setTesting] = useState<ConnectorAccount | null>(null);

	async function remove(account: ConnectorAccount): Promise<void> {
		const confirmed = await requestConfirm(t("connections.removeConfirm", { name: account.name }), {
			confirmLabel: t("connections.remove"),
			danger: true,
		});
		if (!confirmed) return;
		let removed = false;
		try {
			await connectorRpc("connectors.accounts.remove", { id: account.id });
			removed = true;
			await onChanged();
			showToast(t("connections.removed"), "success");
		} catch (caught: unknown) {
			showToast(removed ? t("refreshFailed") : caught instanceof Error ? caught.message : String(caught), "error");
		}
	}

	return (
		<div className="flex flex-col gap-3">
			<div className="flex flex-wrap justify-end gap-2">
				<button type="button" className="provider-btn" disabled={!caldavAvailable} onClick={() => setEditing("caldav")}>
					{t("connections.add")}
				</button>
				<button
					type="button"
					className="provider-btn"
					disabled={!channelHistoryAvailable || channelSources.length === 0}
					onClick={() => setEditing("channel_history")}
				>
					{t("connections.addChannel")}
				</button>
				<button type="button" className="provider-btn" disabled={!gmailAvailable} onClick={() => setEditing("gmail")}>
					{t("connections.addGmail")}
				</button>
				<button
					type="button"
					className="provider-btn"
					disabled={!himalayaAvailable}
					onClick={() => setEditing("himalaya")}
				>
					{t("connections.addHimalaya")}
				</button>
			</div>
			{caldavAvailable ? null : <StatusMessage error={t("connections.unavailable")} />}
			{channelHistoryAvailable && channelSources.length === 0 ? (
				<StatusMessage error={t("connections.noChannelSources")} />
			) : null}
			{accounts.length === 0 ? <EmptyState message={t("connections.empty")} /> : null}
			{accounts.map((account) => (
				<SettingsCard key={account.id} className="bg-[var(--surface)] border border-[var(--border)] rounded-lg p-4">
					<div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
						<div className="min-w-0">
							<div className="flex flex-wrap items-center gap-2">
								<h3 className="text-sm font-medium text-[var(--text-strong)]">{account.name}</h3>
								<Badge
									label={account.enabled ? t("status.enabled") : t("status.disabled")}
									variant={account.enabled ? "configured" : "muted"}
								/>
								{account.managed ? <Badge label={t("connections.managed")} variant="muted" /> : null}
								{account.kind === "caldav" ? (
									<Badge
										label={account.hasPassword ? t("connections.passwordConfigured") : t("connections.passwordMissing")}
										variant={account.hasPassword ? "configured" : "warning"}
									/>
								) : account.kind === "channel_history" ? (
									<Badge label={t("connections.channelHistory")} variant="configured" />
								) : (
									<Badge label={account.kind === "gmail" ? "Gmail" : "Himalaya"} variant="configured" />
								)}
							</div>
							{account.kind === "caldav" ? (
								<>
									<div className="mt-2 break-all text-xs text-[var(--muted)]">{safeServerUrl(account.serverUrl)}</div>
									<div className="mt-1 text-xs text-[var(--muted)]">
										{account.username} · {account.timeoutSeconds}s
									</div>
								</>
							) : account.kind === "channel_history" ? (
								<div className="mt-2 text-xs text-[var(--muted)]">
									{account.channelType} · {account.channelAccountId}
								</div>
							) : (
								<div className="mt-2 text-xs text-[var(--muted)]">
									{account.kind === "gmail"
										? t("connections.googleWorkspaceCredentials")
										: `${account.himalayaBackend} · ${account.himalayaAccountName}`}
								</div>
							)}
							{account.kind === "caldav" && (account.allowInsecureHttp || account.allowPrivateNetwork) ? (
								<div className="mt-2 flex flex-wrap gap-2">
									{account.allowInsecureHttp ? <Badge label={t("connections.allowHttp")} variant="warning" /> : null}
									{account.allowPrivateNetwork ? (
										<Badge label={t("connections.allowPrivate")} variant="warning" />
									) : null}
								</div>
							) : null}
						</div>
						<div className="flex flex-wrap gap-2">
							<button
								type="button"
								className="provider-btn provider-btn-secondary"
								disabled={!(account.enabled && (account.kind !== "caldav" || account.hasPassword))}
								onClick={() => setTesting(account)}
							>
								{t("connections.test")}
							</button>
							<button type="button" className="provider-btn provider-btn-secondary" onClick={() => setEditing(account)}>
								{t("connections.edit")}
							</button>
							<button
								type="button"
								className="provider-btn provider-btn-danger"
								disabled={account.managed}
								title={account.managed ? t("connections.managedRemove") : undefined}
								onClick={() => void remove(account)}
							>
								{t("connections.remove")}
							</button>
						</div>
					</div>
				</SettingsCard>
			))}
			{editing && (editing === "caldav" || (typeof editing === "object" && editing.kind === "caldav")) ? (
				<ConnectionFormModal
					key={editing === "caldav" ? "new-caldav" : editing.id}
					account={editing === "caldav" ? null : editing}
					onClose={() => setEditing(null)}
					onSaved={onChanged}
				/>
			) : null}
			{editing &&
			(editing === "gmail" ||
				editing === "himalaya" ||
				(typeof editing === "object" && (editing.kind === "gmail" || editing.kind === "himalaya"))) ? (
				<EmailConnectionFormModal
					key={typeof editing === "string" ? `new-${editing}` : editing.id}
					kind={typeof editing === "string" ? editing : editing.kind}
					account={typeof editing === "string" ? null : editing}
					onClose={() => setEditing(null)}
					onSaved={onChanged}
				/>
			) : null}
			{editing &&
			(editing === "channel_history" || (typeof editing === "object" && editing.kind === "channel_history")) ? (
				<ChannelConnectionFormModal
					key={editing === "channel_history" ? "new-channel" : editing.id}
					account={editing === "channel_history" ? null : editing}
					sources={channelSources}
					onClose={() => setEditing(null)}
					onSaved={onChanged}
				/>
			) : null}
			{testing ? <TestConnectionModal account={testing} onClose={() => setTesting(null)} /> : null}
			<ConfirmDialog />
		</div>
	);
}

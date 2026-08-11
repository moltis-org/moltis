import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import {
	Badge,
	CheckboxField,
	EmptyState,
	Loading,
	SaveButton,
	SettingsCard,
	StatusMessage,
	TextField,
} from "../../components/forms";
import { useTranslation } from "../../i18n";
import type { ConnectorAccount, ConnectorCalendar } from "../../types/connector";
import { ConfirmDialog, Modal, requestConfirm, showToast } from "../../ui";
import { connectorRpc } from "./rpc";

interface ConnectionsTabProps {
	accounts: ConnectorAccount[];
	caldavAvailable: boolean;
	onChanged: () => Promise<void>;
}

interface ConnectionFormModalProps {
	account: ConnectorAccount | null;
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

interface TestConnectionModalProps {
	account: ConnectorAccount;
	onClose: () => void;
}

function TestConnectionModal({ account, onClose }: TestConnectionModalProps): VNode {
	const { t } = useTranslation("connectors");
	const [calendars, setCalendars] = useState<ConnectorCalendar[] | null>(null);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		let active = true;
		connectorRpc("connectors.accounts.test", { id: account.id })
			.then((payload) => {
				if (active) setCalendars(payload.calendars);
			})
			.catch((caught: unknown) => {
				if (active) setError(caught instanceof Error ? caught.message : String(caught));
			});
		return () => {
			active = false;
		};
	}, [account.id]);

	return (
		<Modal show={true} onClose={onClose} title={t("connections.testTitle")}>
			<div className="flex flex-col gap-3">
				<div className="text-xs text-[var(--muted)]">{account.name}</div>
				{calendars || error ? null : <Loading message={t("connections.testing")} />}
				<StatusMessage error={error} />
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

export function ConnectionsTab({ accounts, caldavAvailable, onChanged }: ConnectionsTabProps): VNode {
	const { t } = useTranslation("connectors");
	const [editing, setEditing] = useState<ConnectorAccount | "new" | null>(null);
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
			<div className="flex justify-end">
				<button type="button" className="provider-btn" disabled={!caldavAvailable} onClick={() => setEditing("new")}>
					{t("connections.add")}
				</button>
			</div>
			{caldavAvailable ? null : <StatusMessage error={t("connections.unavailable")} />}
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
								<Badge
									label={account.hasPassword ? t("connections.passwordConfigured") : t("connections.passwordMissing")}
									variant={account.hasPassword ? "configured" : "warning"}
								/>
							</div>
							<div className="mt-2 break-all text-xs text-[var(--muted)]">{safeServerUrl(account.serverUrl)}</div>
							<div className="mt-1 text-xs text-[var(--muted)]">
								{account.username} · {account.timeoutSeconds}s
							</div>
							{account.allowInsecureHttp || account.allowPrivateNetwork ? (
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
								disabled={!(account.enabled && account.hasPassword)}
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
			{editing ? (
				<ConnectionFormModal
					key={editing === "new" ? "new" : editing.id}
					account={editing === "new" ? null : editing}
					onClose={() => setEditing(null)}
					onSaved={onChanged}
				/>
			) : null}
			{testing ? <TestConnectionModal account={testing} onClose={() => setTesting(null)} /> : null}
			<ConfirmDialog />
		</div>
	);
}

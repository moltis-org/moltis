// ── Remote access section ────────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { TabBar } from "../../components/forms/Tabs";
import { navigate } from "../../router";
import { settingsPath } from "../../routes";
import { targetValue } from "../../typed-events";
import { rerender } from "./_shared";

export function renderLinkedText(text: string): (string | VNode)[] {
	return String(text || "")
		.split(/(https?:\/\/[^\s]+)/g)
		.filter(Boolean)
		.map((part, index) =>
			/^https?:\/\//.test(part) ? (
				<a key={index} href={part} target="_blank" rel="noopener" className="underline break-all">
					{part}
				</a>
			) : (
				part
			),
		);
}

/** Clone a hidden element from index.html by ID. */
export function cloneHidden(id: string): HTMLElement | null {
	const el = document.getElementById(id);
	if (!el) return null;
	const clone = el.cloneNode(true) as HTMLElement;
	clone.removeAttribute("id");
	clone.style.display = "";
	return clone;
}

interface TailscaleStatus {
	installed?: boolean;
	version?: string;
	tailnet?: string;
	login_name?: string;
	tailscale_ip?: string;
	tailscale_up?: boolean;
	mode?: string;
	hostname?: string;
	url?: string;
	passkey_warning?: string;
}

interface NgrokStatus {
	enabled?: boolean;
	authtoken_source?: string;
	domain?: string;
	public_url?: string;
	passkey_warning?: string;
	error?: string;
}

interface NgrokForm {
	enabled: boolean;
	authtoken: string;
	clearAuthtoken: boolean;
	domain: string;
}

export function RemoteAccessSection(): VNode {
	const [tsStatus, setTsStatus] = useState<TailscaleStatus | null>(null);
	const [tsError, setTsError] = useState<string | null>(null);
	const [tsWarning, setTsWarning] = useState<string | null>(null);
	const [tsLoading, setTsLoading] = useState(true);
	const [configuring, setConfiguring] = useState(false);
	const [configuringMode, setConfiguringMode] = useState<string | null>(null);
	const [ngStatus, setNgStatus] = useState<NgrokStatus | null>(null);
	const [ngError, setNgError] = useState<string | null>(null);
	const [ngLoading, setNgLoading] = useState(true);
	const [ngSaving, setNgSaving] = useState(false);
	const [ngMsg, setNgMsg] = useState<string | null>(null);
	const [ngForm, setNgForm] = useState<NgrokForm>({
		enabled: false,
		authtoken: "",
		clearAuthtoken: false,
		domain: "",
	});
	const [authReady, setAuthReady] = useState(false);

	function fetchTsStatus(): void {
		setTsLoading(true);
		rerender();
		fetch("/api/tailscale/status")
			.then((r) => {
				const ct = r.headers.get("content-type") || "";
				if (r.status === 404 || !ct.includes("application/json")) {
					setTsError("Tailscale feature is not enabled. Rebuild with --features tailscale.");
					setTsLoading(false);
					rerender();
					return null;
				}
				return r.json();
			})
			.then((data: TailscaleStatus | null) => {
				if (!data) return;
				if ((data as { error?: string }).error) {
					setTsError((data as { error?: string }).error || null);
				} else {
					setTsStatus(data);
					setTsError(null);
					setTsWarning(data.passkey_warning || null);
				}
				setTsLoading(false);
				rerender();
			})
			.catch((e: Error) => {
				setTsError(e.message);
				setTsLoading(false);
				rerender();
			});
	}

	function fetchNgrokStatus(): void {
		setNgLoading(true);
		rerender();
		fetch("/api/ngrok/status")
			.then((r) => {
				const ct = r.headers.get("content-type") || "";
				if (r.status === 404 || !ct.includes("application/json")) {
					setNgError("ngrok feature is not enabled. Rebuild with --features ngrok.");
					setNgStatus(null);
					setNgLoading(false);
					rerender();
					return null;
				}
				return r.json();
			})
			.then((data: NgrokStatus | null) => {
				if (!data) return;
				setNgStatus(data);
				setNgError(data.error || null);
				setNgLoading(false);
				setNgForm({
					enabled: Boolean(data.enabled),
					authtoken: "",
					clearAuthtoken: false,
					domain: data.domain || "",
				});
				rerender();
			})
			.catch((e: Error) => {
				setNgError(e.message);
				setNgLoading(false);
				rerender();
			});
	}

	function setMode(mode: string): void {
		setConfiguring(true);
		setTsError(null);
		setTsWarning(null);
		setConfiguringMode(mode);
		rerender();
		fetch("/api/tailscale/configure", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ mode }),
		})
			.then((r) => r.json())
			.then((data: { error?: string; passkey_warning?: string }) => {
				if (data.error) {
					setTsError(data.error);
				} else {
					setTsWarning(data.passkey_warning || null);
					fetchTsStatus();
				}
				setConfiguring(false);
				setConfiguringMode(null);
				rerender();
			})
			.catch((e: Error) => {
				setTsError(e.message);
				setConfiguring(false);
				setConfiguringMode(null);
				rerender();
			});
	}

	function persistNgrokConfig(nextForm: NgrokForm, successMessage: string): void {
		setNgSaving(true);
		setNgError(null);
		setNgMsg(null);
		rerender();

		fetch("/api/ngrok/config", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({
				enabled: nextForm.enabled,
				authtoken: nextForm.authtoken,
				clear_authtoken: nextForm.clearAuthtoken,
				domain: nextForm.domain,
			}),
		})
			.then((r) =>
				r
					.json()
					.catch(() => ({}))
					.then((data: { error?: string; status?: NgrokStatus }) => ({ ok: r.ok, data })),
			)
			.then(({ ok, data }: { ok: boolean; data: { error?: string; status?: NgrokStatus } }) => {
				setNgSaving(false);
				if (!ok || data.error) {
					setNgError(data.error || null);
				} else {
					setNgMsg(successMessage);
					if (data.status) {
						setNgStatus(data.status);
						setNgForm({
							enabled: Boolean(data.status.enabled),
							authtoken: "",
							clearAuthtoken: false,
							domain: data.status.domain || "",
						});
					} else {
						fetchNgrokStatus();
					}
				}
				rerender();
			})
			.catch((e: Error) => {
				setNgSaving(false);
				setNgError(e.message);
				rerender();
			});
	}

	function saveNgrokConfig(e: Event): void {
		e.preventDefault();
		persistNgrokConfig(ngForm, "ngrok settings applied.");
	}

	function toggleNgrokEnabled(): void {
		const nextForm = {
			...ngForm,
			enabled: !ngForm.enabled,
		};
		setNgForm(nextForm);
		persistNgrokConfig(nextForm, `ngrok ${nextForm.enabled ? "enabled" : "disabled"}.`);
	}

	function toggleNgrokTokenDeletion(): void {
		if (ngForm.clearAuthtoken) {
			setNgForm({
				...ngForm,
				clearAuthtoken: false,
			});
			return;
		}

		if (!window.confirm("Delete the current ngrok token from config when you save?")) {
			return;
		}

		setNgForm({
			...ngForm,
			authtoken: "",
			clearAuthtoken: true,
		});
	}

	useEffect(() => {
		fetchTsStatus();
		fetchNgrokStatus();
		fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((d: { auth_disabled?: boolean; has_password?: boolean } | null) => {
				if (!d) return;
				const ready = d.auth_disabled ? false : d.has_password === true;
				setAuthReady(ready);
				rerender();
			})
			.catch(() => {
				/* ignore auth status fetch errors */
			});
	}, []);

	function renderTailscaleModeButton(mode: string, currentMode: string): VNode {
		const active = currentMode === mode && !configuring;
		const classes = active
			? "ts-mode-active"
			: "text-[var(--muted)] border-[var(--border)] bg-transparent hover:text-[var(--text)] hover:border-[var(--border-strong)]";
		return (
			<button
				type="button"
				className={`text-xs border px-3 py-1.5 rounded-md cursor-pointer transition-colors font-medium ${classes}${
					configuringMode === mode ? " ts-mode-configuring" : ""
				}`}
				disabled={configuring}
				onClick={() => setMode(mode)}
			>
				{configuringMode === mode ? <span className="ts-spinner" /> : null}
				{mode}
			</button>
		);
	}

	function renderTailscaleCard(): VNode {
		const currentMode = tsStatus?.mode || "off";
		const tsVaultBlocked = tsError === "vault is sealed";
		return (
			<section className="rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)] p-4 flex flex-col gap-4">
				<div className="flex flex-col gap-1">
					<h3 className="text-base font-medium text-[var(--text-strong)]">Tailscale</h3>
					<p className="text-xs text-[var(--muted)] leading-relaxed">
						Expose the gateway via Tailscale Serve (tailnet-only HTTPS) or Funnel (public HTTPS). The gateway stays
						bound to localhost while Tailscale proxies traffic to it.
					</p>
				</div>

				{tsLoading ? (
					<div className="text-xs text-[var(--muted)]">Loading{"\u2026"} this can take a few seconds.</div>
				) : null}
				{tsStatus?.installed ? (
					<div className="info-bar">
						<span className="info-field">
							<span className="status-dot connected" />
							<span className="info-label">Installed</span>
							{tsStatus.version ? <span className="info-version">v{tsStatus.version.split("-")[0]}</span> : null}
						</span>
						{tsStatus.tailnet ? (
							<span className="info-field">
								<span className="info-label">Tailnet:</span>
								<span className="info-value-strong">{tsStatus.tailnet}</span>
							</span>
						) : null}
						{tsStatus.login_name ? (
							<span className="info-field">
								<span className="info-label">Account:</span>
								<span className="info-value">{tsStatus.login_name}</span>
							</span>
						) : null}
						{tsStatus.tailscale_ip ? (
							<span className="info-field">
								<span className="info-label">IP:</span>
								<span className="info-value-mono">{tsStatus.tailscale_ip}</span>
							</span>
						) : null}
					</div>
				) : null}
				{tsError ? (
					<div className="settings-alert-error whitespace-pre-line max-w-form">
						<span className="icon icon-lg icon-warn-triangle shrink-0 mt-0.5" />
						<span>{renderLinkedText(tsError)}</span>
					</div>
				) : null}
				{tsVaultBlocked ? (
					<button type="button" className="provider-btn self-start" onClick={() => navigate(settingsPath("vault"))}>
						Unlock in Encryption settings
					</button>
				) : null}
				{tsWarning ? <div className="alert-warning-text max-w-form">{tsWarning}</div> : null}

				{tsStatus?.installed === false ? (
					<div
						className="info-bar"
						style={{ justifyContent: "center", flexDirection: "column", gap: "12px", textAlign: "center" }}
					>
						<p className="text-sm text-[var(--text)]">
							The <code className="font-mono text-sm">tailscale</code> CLI was not found on this machine.
						</p>
						<div className="flex items-center justify-center gap-2 flex-wrap">
							<a
								href="https://tailscale.com/download"
								target="_blank"
								rel="noopener"
								className="provider-btn"
								style={{ display: "inline-block", textDecoration: "none" }}
							>
								Install Tailscale
							</a>
							<button type="button" className="provider-btn provider-btn-secondary" onClick={fetchTsStatus}>
								Re-check
							</button>
						</div>
					</div>
				) : null}

				{!tsLoading && tsStatus?.installed !== false ? (
					<div className="flex flex-col gap-4">
						{tsStatus?.tailscale_up === false ? (
							<div className="alert-warning-text max-w-form">
								<span className="alert-label-warn">Warning:</span> Tailscale is not running. Start it with{" "}
								<code className="font-mono">tailscale up</code> or open the Tailscale app.
							</div>
						) : null}

						<div className="max-w-form flex flex-col gap-2">
							<h4 className="text-sm font-medium text-[var(--text-strong)]">Mode</h4>
							<div className="flex gap-2 flex-wrap">
								{(["off", "serve", "funnel"] as const).map((mode) => renderTailscaleModeButton(mode, currentMode))}
							</div>
							{configuring ? (
								<div className="text-xs text-[var(--muted)]">
									Configuring tailscale {configuringMode}
									{"\u2026"} This can take up to 10 seconds.
								</div>
							) : null}
						</div>

						<div className="alert-warning-text max-w-form">
							<span className="alert-label-warn">Warning:</span> Enabling Funnel exposes moltis to the public internet.
							This code has not been security-audited. Use at your own risk.
						</div>
						{authReady ? null : (
							<div className="flex flex-col gap-2 max-w-form">
								<div className="alert-warning-text">
									<span className="alert-label-warn">Warning:</span> Funnel can be enabled now, but remote visitors will
									see the setup-required page until authentication is configured.
								</div>
								<button
									type="button"
									className="provider-btn self-start"
									onClick={() => navigate(settingsPath("security"))}
								>
									Set up authentication
								</button>
							</div>
						)}

						{tsStatus?.hostname ? (
							<div className="max-w-form">
								<h4 className="text-sm font-medium text-[var(--text-strong)] mb-1">Hostname</h4>
								{tsStatus.url && currentMode !== "off" ? (
									<a
										href={tsStatus.url}
										target="_blank"
										rel="noopener"
										className="font-mono text-sm text-[var(--accent)] no-underline"
									>
										{tsStatus.hostname}
									</a>
								) : (
									<div className="font-mono text-sm">{tsStatus.hostname}</div>
								)}
							</div>
						) : null}
						{tsStatus?.url && currentMode !== "off" ? (
							<div className="max-w-form">
								<h4 className="text-sm font-medium text-[var(--text-strong)] mb-1">URL</h4>
								<a
									href={tsStatus.url}
									target="_blank"
									rel="noopener"
									className="font-mono text-sm text-[var(--accent)] no-underline break-all"
								>
									{tsStatus.url}
								</a>
							</div>
						) : null}
						{currentMode === "funnel" ? (
							<div className="alert-warning-text max-w-form">
								<span className="alert-label-warn">Warning:</span> Funnel exposes your gateway to the public internet.
								Make sure password authentication is configured.
							</div>
						) : null}
					</div>
				) : null}
			</section>
		);
	}

	function renderNgrokCard(): VNode {
		const authSourceLabel =
			ngStatus?.authtoken_source === "config"
				? "Stored in config"
				: ngStatus?.authtoken_source === "env"
					? "Using NGROK_AUTHTOKEN from the environment"
					: "No authtoken configured yet";
		const ngVaultBlocked = ngError === "vault is sealed";

		return (
			<section className="rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)] p-4 flex flex-col gap-4">
				<div className="flex flex-col gap-1">
					<h3 className="text-base font-medium text-[var(--text-strong)]">ngrok</h3>
					<p className="text-xs text-[var(--muted)] leading-relaxed">
						Create a public HTTPS endpoint without installing an external binary. Changes apply immediately.
					</p>
				</div>

				{ngLoading ? (
					<div className="text-xs text-[var(--muted)]">Loading{"\u2026"} this can take a few seconds.</div>
				) : null}
				{ngError ? (
					<div className="settings-alert-error whitespace-pre-line max-w-form">
						<span className="icon icon-lg icon-warn-triangle shrink-0 mt-0.5" />
						<span>{renderLinkedText(ngError)}</span>
					</div>
				) : null}
				{ngVaultBlocked ? (
					<button type="button" className="provider-btn self-start" onClick={() => navigate(settingsPath("vault"))}>
						Unlock in Encryption settings
					</button>
				) : null}

				{ngLoading || ngError ? null : (
					<form className="flex flex-col gap-4" onSubmit={saveNgrokConfig}>
						<div className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2.5 flex items-center justify-between gap-3">
							<div>
								<div className="text-sm font-medium text-[var(--text-strong)]">
									ngrok is {ngForm.enabled ? "enabled" : "disabled"}
								</div>
								<div className="text-xs text-[var(--muted)]">
									Public HTTPS endpoint for demos, shared testing, and team access.
								</div>
							</div>
							<button type="button" className="provider-btn" disabled={ngSaving} onClick={toggleNgrokEnabled}>
								{ngSaving ? "Saving\u2026" : ngForm.enabled ? "Disable ngrok" : "Enable ngrok"}
							</button>
						</div>

						<div className="flex flex-col gap-1">
							<label className="text-sm font-medium text-[var(--text-strong)]" htmlFor="ngrok-authtoken">
								Authtoken
							</label>
							<input
								id="ngrok-authtoken"
								type="password"
								className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-sm text-[var(--text)]"
								placeholder={
									ngStatus?.authtoken_source ? "Leave blank to keep the current token" : "Paste your ngrok authtoken"
								}
								value={ngForm.authtoken}
								onInput={(e: Event) => setNgForm({ ...ngForm, authtoken: targetValue(e) })}
							/>
							<div className="text-xs text-[var(--muted)]">{authSourceLabel}</div>
							<div className="text-xs text-[var(--muted)]">
								Create or copy an authtoken from{" "}
								<a
									href="https://dashboard.ngrok.com/get-started/your-authtoken"
									target="_blank"
									rel="noopener"
									className="text-[var(--accent)] no-underline hover:underline"
								>
									ngrok dashboard
								</a>
								.
							</div>
							{ngStatus?.authtoken_source === "config" ? (
								<div className="flex flex-col gap-1">
									<button
										type="button"
										className="text-xs text-[var(--accent)] self-start bg-transparent border-0 p-0 cursor-pointer hover:underline"
										onClick={toggleNgrokTokenDeletion}
									>
										{ngForm.clearAuthtoken ? "Keep current token" : "Delete current token"}
									</button>
									{ngForm.clearAuthtoken ? (
										<div className="text-xs text-[var(--muted)]">
											The saved config token will be deleted when you save.
										</div>
									) : null}
								</div>
							) : null}
						</div>

						<div className="flex flex-col gap-1">
							<label className="text-sm font-medium text-[var(--text-strong)]" htmlFor="ngrok-domain">
								Reserved domain
							</label>
							<input
								id="ngrok-domain"
								type="text"
								className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-sm text-[var(--text)]"
								placeholder="team-gateway.ngrok.app"
								value={ngForm.domain}
								onInput={(e: Event) => setNgForm({ ...ngForm, domain: targetValue(e) })}
							/>
							<div className="text-xs text-[var(--muted)]">
								Optional. Use a reserved domain if you want a stable passkey origin across restarts.
							</div>
						</div>

						{ngStatus?.public_url ? (
							<div className="flex flex-col gap-1">
								<h4 className="text-sm font-medium text-[var(--text-strong)]">Active public URL</h4>
								<a
									href={ngStatus.public_url}
									target="_blank"
									rel="noopener"
									className="font-mono text-sm text-[var(--accent)] no-underline break-all"
								>
									{ngStatus.public_url}
								</a>
							</div>
						) : null}
						{ngStatus?.passkey_warning ? (
							<div className="alert-warning-text max-w-form">{ngStatus.passkey_warning}</div>
						) : null}
						{ngForm.enabled && !authReady ? (
							<div className="alert-warning-text max-w-form">
								<span className="alert-label-warn">Warning:</span> ngrok can be enabled now, but remote visitors will
								see the setup-required page until authentication is configured.
							</div>
						) : null}
						{ngMsg ? <div className="text-xs text-[var(--ok)]">{ngMsg}</div> : null}

						<div className="flex flex-wrap gap-2">
							<button type="submit" className="provider-btn" disabled={ngSaving}>
								{ngSaving ? "Saving\u2026" : "Save ngrok settings"}
							</button>
						</div>
					</form>
				)}
			</section>
		);
	}

	const [activeTab, setActiveTab] = useState("tailscale");

	const tsBadge = tsLoading ? undefined : tsStatus?.mode && tsStatus.mode !== "off" ? tsStatus.mode : undefined;
	const ngBadge = ngLoading ? undefined : ngStatus?.enabled ? "on" : undefined;

	const tabs = [
		{ id: "tailscale", label: "Tailscale", badge: tsBadge },
		{ id: "ngrok", label: "ngrok", badge: ngBadge },
	];

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Remote Access</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed max-w-[60rem]" style={{ margin: 0 }}>
				Choose how moltis is exposed beyond localhost. Tailscale is the safer default for tailnet access and optional
				public Funnel, while ngrok gives you a managed public HTTPS URL for teams, demos, and shared endpoints.
			</p>
			<TabBar tabs={tabs} active={activeTab} onChange={setActiveTab} />
			{activeTab === "tailscale" && renderTailscaleCard()}
			{activeTab === "ngrok" && renderNgrokCard()}
		</div>
	);
}

// ── Remote access step ────────────────────────────────────────

import type { VNode } from "preact";
import { useEffect, useState } from "preact/hooks";
import { TabBar } from "../../components/forms/Tabs";
import { t } from "../../i18n";
import { targetValue } from "../../typed-events";
import { ErrorPanel } from "../shared";

// ── Types ───────────────────────────────────────────────────

export interface TailscaleStatus {
	installed?: boolean;
	tailscale_up?: boolean;
	mode?: string;
	url?: string;
	passkey_warning?: string;
	error?: string;
	[key: string]: unknown;
}

export interface NgrokStatus {
	enabled?: boolean;
	public_url?: string;
	domain?: string;
	authtoken_source?: string;
	passkey_warning?: string;
	[key: string]: unknown;
}

export interface NetbirdStatus {
	installed?: boolean;
	netbird_up?: boolean;
	mode?: string;
	url?: string;
	peer_ip?: string;
	dns_name?: string;
	error?: string;
	[key: string]: unknown;
}

export interface CloudflareTunnelStatus {
	enabled?: boolean;
	public_url?: string;
	hostname?: string;
	token_source?: string;
	passkey_warning?: string;
	error?: string;
	[key: string]: unknown;
}

interface NgrokForm {
	enabled: boolean;
	authtoken: string;
	domain: string;
}

interface CloudflareTunnelForm {
	enabled: boolean;
	token: string;
	hostname: string;
}

// ── Helpers ─────────────────────────────────────────────────

export function fetchRemoteAccessStatus(
	path: string,
	featureDisabledMessage: string,
): Promise<{ error?: string; feature_disabled?: boolean; [key: string]: unknown }> {
	return fetch(path)
		.then((response) => {
			const contentType = response.headers.get("content-type") || "";
			if (response.status === 404 || !contentType.includes("application/json")) {
				return {
					error: featureDisabledMessage,
					feature_disabled: true,
				};
			}
			return response.json() as Promise<Record<string, unknown>>;
		})
		.catch((err: Error) => ({
			error: err.message,
		}));
}

export function preferredPublicBaseUrl({
	cloudflareStatus,
	ngrokStatus,
	tailscaleStatus,
}: {
	cloudflareStatus?: CloudflareTunnelStatus | null;
	ngrokStatus: NgrokStatus | null;
	tailscaleStatus: TailscaleStatus | null;
}): string {
	const cloudflareUrl =
		cloudflareStatus?.enabled && typeof cloudflareStatus?.public_url === "string"
			? cloudflareStatus.public_url.trim()
			: "";
	if (cloudflareUrl) return cloudflareUrl;

	const ngrokUrl = typeof ngrokStatus?.public_url === "string" ? ngrokStatus.public_url.trim() : "";
	if (ngrokUrl) return ngrokUrl;

	const tailscaleUrl = typeof tailscaleStatus?.url === "string" ? tailscaleStatus.url.trim() : "";
	if (tailscaleStatus?.mode === "funnel" && tailscaleUrl) return tailscaleUrl;

	return "";
}

// ── RemoteAccessStep ────────────────────────────────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: onboarding remote access manages multiple endpoint integrations
export function RemoteAccessStep({ onNext, onBack }: { onNext: () => void; onBack: () => void }): VNode {
	const [remoteTab, setRemoteTab] = useState("tailscale");
	const [authReady, setAuthReady] = useState(false);
	const [tsStatus, setTsStatus] = useState<TailscaleStatus | null>(null);
	const [tsError, setTsError] = useState<string | null>(null);
	const [tsWarning, setTsWarning] = useState<string | null>(null);
	const [tsLoading, setTsLoading] = useState(true);
	const [configuringTailscale, setConfiguringTailscale] = useState(false);
	const [ngStatus, setNgStatus] = useState<NgrokStatus | null>(null);
	const [ngError, setNgError] = useState<string | null>(null);
	const [ngLoading, setNgLoading] = useState(true);
	const [ngSaving, setNgSaving] = useState(false);
	const [ngMsg, setNgMsg] = useState<string | null>(null);
	const [nbStatus, setNbStatus] = useState<NetbirdStatus | null>(null);
	const [nbError, setNbError] = useState<string | null>(null);
	const [nbLoading, setNbLoading] = useState(true);
	const [nbConfiguring, setNbConfiguring] = useState(false);
	const [cfStatus, setCfStatus] = useState<CloudflareTunnelStatus | null>(null);
	const [cfError, setCfError] = useState<string | null>(null);
	const [cfLoading, setCfLoading] = useState(true);
	const [cfSaving, setCfSaving] = useState(false);
	const [cfMsg, setCfMsg] = useState<string | null>(null);
	const [ngForm, setNgForm] = useState<NgrokForm>({
		enabled: false,
		authtoken: "",
		domain: "",
	});
	const [cfForm, setCfForm] = useState<CloudflareTunnelForm>({
		enabled: false,
		token: "",
		hostname: "",
	});

	function loadAuthStatus(): Promise<void> {
		return fetch("/api/auth/status")
			.then((response) => (response.ok ? (response.json() as Promise<Record<string, unknown>>) : null))
			.then((data) => {
				const ready = data?.auth_disabled ? false : data?.has_password === true;
				setAuthReady(ready);
			})
			.catch(() => {
				setAuthReady(false);
			});
	}

	function loadTailscaleStatus(): Promise<void> {
		setTsLoading(true);
		return fetchRemoteAccessStatus("/api/tailscale/status", "Tailscale feature is not enabled in this build.")
			.then((data) => {
				setTsStatus(data?.feature_disabled ? null : (data as TailscaleStatus));
				setTsError(data?.error || null);
				setTsWarning((data as TailscaleStatus)?.passkey_warning || null);
				setTsLoading(false);
			})
			.catch((err: Error) => {
				setTsError(err.message);
				setTsLoading(false);
			});
	}

	function loadNgrokStatus(): Promise<void> {
		setNgLoading(true);
		return fetchRemoteAccessStatus("/api/ngrok/status", "ngrok feature is not enabled in this build.")
			.then((data) => {
				setNgStatus(data?.feature_disabled ? null : (data as NgrokStatus));
				setNgError(data?.error || null);
				setNgLoading(false);
				setNgForm((current) => ({
					enabled: Boolean(data?.enabled),
					authtoken: current.authtoken,
					domain: current.domain || (data?.domain as string) || "",
				}));
			})
			.catch((err: Error) => {
				setNgError(err.message);
				setNgLoading(false);
			});
	}

	function loadNetbirdStatus(): Promise<void> {
		setNbLoading(true);
		return fetchRemoteAccessStatus("/api/netbird/status", "NetBird feature is not enabled in this build.")
			.then((data) => {
				setNbStatus(data?.feature_disabled ? null : (data as NetbirdStatus));
				setNbError(data?.error || null);
				setNbLoading(false);
			})
			.catch((err: Error) => {
				setNbError(err.message);
				setNbLoading(false);
			});
	}

	function loadCloudflareTunnelStatus(): Promise<void> {
		setCfLoading(true);
		return fetchRemoteAccessStatus(
			"/api/cloudflare-tunnel/status",
			"Cloudflare Tunnel feature is not enabled in this build.",
		)
			.then((data) => {
				setCfStatus(data?.feature_disabled ? null : (data as CloudflareTunnelStatus));
				setCfError(data?.error || null);
				setCfLoading(false);
				setCfForm((current) => ({
					enabled: Boolean(data?.enabled),
					token: current.token,
					hostname: current.hostname || (data?.hostname as string) || "",
				}));
			})
			.catch((err: Error) => {
				setCfError(err.message);
				setCfLoading(false);
			});
	}

	useEffect(() => {
		loadAuthStatus();
		loadTailscaleStatus();
		loadNgrokStatus();
		loadNetbirdStatus();
		loadCloudflareTunnelStatus();
	}, []);

	function setTailscaleMode(mode: string): void {
		setConfiguringTailscale(true);
		setTsError(null);
		setTsWarning(null);
		fetch("/api/tailscale/configure", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ mode }),
		})
			.then((response) =>
				(response.json() as Promise<Record<string, unknown>>)
					.catch((): Record<string, unknown> => ({}))
					.then((data) => ({ ok: response.ok, data })),
			)
			.then(({ ok, data }) => {
				if (!ok || data.error) {
					setTsError((data.error as string) || "Failed to configure Tailscale.");
				} else {
					setTsWarning((data.passkey_warning as string) || null);
					loadTailscaleStatus();
				}
				setConfiguringTailscale(false);
			})
			.catch((err: Error) => {
				setTsError(err.message);
				setConfiguringTailscale(false);
			});
	}

	function toggleTailscaleFunnel(): void {
		const nextMode = tsStatus?.mode === "funnel" ? "off" : "funnel";
		setTailscaleMode(nextMode);
	}

	function applyNgrokConfig(nextForm: NgrokForm, successMessage: string): void {
		setNgSaving(true);
		setNgError(null);
		setNgMsg(null);
		fetch("/api/ngrok/config", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({
				enabled: nextForm.enabled,
				authtoken: nextForm.authtoken,
				clear_authtoken: false,
				domain: nextForm.domain,
			}),
		})
			.then((response) =>
				(response.json() as Promise<Record<string, unknown>>)
					.catch((): Record<string, unknown> => ({}))
					.then((data) => ({ ok: response.ok, data })),
			)
			.then(({ ok, data }) => {
				setNgSaving(false);
				if (!ok || data.error) {
					setNgError((data.error as string) || "Failed to apply ngrok settings.");
					return;
				}

				const status = (data.status as NgrokStatus) || null;
				setNgMsg(successMessage);
				setNgStatus(status);
				setNgForm({
					enabled: Boolean(status?.enabled),
					authtoken: "",
					domain: status?.domain || nextForm.domain || "",
				});
			})
			.catch((err: Error) => {
				setNgSaving(false);
				setNgError(err.message);
			});
	}

	function toggleNgrokEnabled(): void {
		const nextForm = {
			...ngForm,
			enabled: !ngForm.enabled,
		};
		setNgForm(nextForm);
		applyNgrokConfig(nextForm, `ngrok ${nextForm.enabled ? "enabled" : "disabled"}.`);
	}

	function setNetbirdMode(mode: string): void {
		setNbConfiguring(true);
		setNbError(null);
		fetch("/api/netbird/configure", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({ mode }),
		})
			.then((response) =>
				(response.json() as Promise<Record<string, unknown>>)
					.catch((): Record<string, unknown> => ({}))
					.then((data) => ({ ok: response.ok, data })),
			)
			.then(({ ok, data }) => {
				setNbConfiguring(false);
				if (!ok || data.error) {
					setNbError((data.error as string) || "Failed to configure NetBird.");
					return;
				}

				loadNetbirdStatus();
			})
			.catch((err: Error) => {
				setNbConfiguring(false);
				setNbError(err.message);
			});
	}

	function toggleNetbirdServe(): void {
		setNetbirdMode(nbStatus?.mode === "serve" ? "off" : "serve");
	}

	function applyCloudflareTunnelConfig(nextForm: CloudflareTunnelForm, successMessage: string): void {
		setCfSaving(true);
		setCfError(null);
		setCfMsg(null);
		fetch("/api/cloudflare-tunnel/config", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({
				enabled: nextForm.enabled,
				token: nextForm.token,
				clear_token: false,
				hostname: nextForm.hostname,
			}),
		})
			.then((response) =>
				(response.json() as Promise<Record<string, unknown>>)
					.catch((): Record<string, unknown> => ({}))
					.then((data) => ({ ok: response.ok, data })),
			)
			.then(({ ok, data }) => {
				setCfSaving(false);
				if (!ok || data.error) {
					setCfError((data.error as string) || "Failed to apply Cloudflare Tunnel settings.");
					return;
				}

				const status = (data.status as CloudflareTunnelStatus) || null;
				setCfMsg(successMessage);
				setCfStatus(status);
				setCfForm({
					enabled: Boolean(status?.enabled),
					token: "",
					hostname: status?.hostname || nextForm.hostname || "",
				});
			})
			.catch((err: Error) => {
				setCfSaving(false);
				setCfError(err.message);
			});
	}

	function toggleCloudflareTunnelEnabled(): void {
		const nextForm = {
			...cfForm,
			enabled: !cfForm.enabled,
		};
		setCfForm(nextForm);
		applyCloudflareTunnelConfig(nextForm, `Cloudflare Tunnel ${nextForm.enabled ? "enabled" : "disabled"}.`);
	}

	const tailscaleAvailable = tsStatus !== null;
	const tailscaleFunnelEnabled = tsStatus?.mode === "funnel";
	const tailscaleInstalled = tsStatus?.installed !== false;
	const tailscaleBlocked = !(tailscaleAvailable && tailscaleInstalled) || tsStatus?.tailscale_up === false;
	const ngrokAvailable = ngStatus !== null;
	const netbirdAvailable = nbStatus !== null;
	const netbirdServeEnabled = nbStatus?.mode === "serve";
	const netbirdInstalled = nbStatus?.installed !== false;
	const netbirdBlocked = !(netbirdAvailable && netbirdInstalled) || nbStatus?.netbird_up === false;
	const cloudflareAvailable = cfStatus !== null;
	const activePublicUrl = preferredPublicBaseUrl({
		cloudflareStatus: cfStatus,
		ngrokStatus: ngStatus,
		tailscaleStatus: tsStatus,
	});

	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Remote Access</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed">
				Public endpoints are optional for most channels, but Microsoft Teams needs one. Enable Tailscale Funnel, ngrok,
				Cloudflare Tunnel, or configure private NetBird access before connecting team channels.
			</p>
			{activePublicUrl ? (
				<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)] flex flex-col gap-1">
					<span className="font-medium text-[var(--text-strong)]">Active public URL</span>
					<a href={activePublicUrl} target="_blank" rel="noopener" className="text-[var(--accent)] underline break-all">
						{activePublicUrl}
					</a>
					<span>The Teams webhook step will prefill this URL.</span>
				</div>
			) : (
				<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 text-xs text-[var(--muted)]">
					Teams webhooks need a public URL. If you skip this step, you can still configure remote access later in
					Settings.
				</div>
			)}

			<TabBar
				tabs={[
					{
						id: "tailscale",
						label: "Tailscale",
						badge: tsLoading ? undefined : tailscaleFunnelEnabled ? "funnel" : undefined,
					},
					{ id: "ngrok", label: "ngrok", badge: ngLoading ? undefined : ngForm.enabled ? "on" : undefined },
					{ id: "netbird", label: "NetBird", badge: nbLoading ? undefined : netbirdServeEnabled ? "serve" : undefined },
					{
						id: "cloudflare",
						label: "Cloudflare",
						badge: cfLoading ? undefined : cfForm.enabled ? "on" : undefined,
					},
				]}
				active={remoteTab}
				onChange={setRemoteTab}
			/>

			{remoteTab === "tailscale" && (
				<div className="flex flex-col gap-4">
					<p className="text-xs text-[var(--muted)] leading-relaxed">
						Public HTTPS through Tailscale. Tailscale Serve is tailnet-only, so Teams webhooks need Funnel instead.
					</p>
					{tsLoading ? (
						<div className="text-xs text-[var(--muted)]">Loading Tailscale status&hellip;</div>
					) : (
						<div className="text-sm text-[var(--text-strong)]">
							Tailscale Funnel is {tailscaleFunnelEnabled ? "enabled" : "disabled"}.
						</div>
					)}
					{tsStatus?.url && tailscaleFunnelEnabled ? (
						<a
							href={tsStatus.url}
							target="_blank"
							rel="noopener"
							className="text-sm text-[var(--accent)] underline break-all"
						>
							{tsStatus.url}
						</a>
					) : null}
					{tsError ? <ErrorPanel message={tsError} /> : null}
					{tsWarning ? <div className="alert-warning-text max-w-form">{tsWarning}</div> : null}
					{tsStatus?.installed === false ? (
						<a
							href="https://tailscale.com/download"
							target="_blank"
							rel="noopener"
							className="provider-btn self-start no-underline"
						>
							Install Tailscale
						</a>
					) : null}
					{tsStatus?.tailscale_up === false ? (
						<div className="alert-warning-text max-w-form">
							<span className="alert-label-warn">Warning:</span> Start Tailscale before enabling Funnel.
						</div>
					) : null}
					{authReady ? null : (
						<div className="alert-warning-text max-w-form">
							<span className="alert-label-warn">Warning:</span> Funnel can be enabled now, but remote visitors will see
							the setup-required page until authentication is configured.
						</div>
					)}
					<button
						type="button"
						className="provider-btn self-start"
						disabled={tsLoading || configuringTailscale || tailscaleBlocked}
						onClick={toggleTailscaleFunnel}
					>
						{configuringTailscale ? "Applying\u2026" : tailscaleFunnelEnabled ? "Disable Funnel" : "Enable Funnel"}
					</button>
				</div>
			)}

			{remoteTab === "ngrok" && (
				<div className="flex flex-col gap-4">
					<p className="text-xs text-[var(--muted)] leading-relaxed">
						Public HTTPS without installing an external binary. This is useful for demos, shared testing, and Teams.
					</p>
					{ngLoading ? (
						<div className="text-xs text-[var(--muted)]">Loading ngrok status&hellip;</div>
					) : (
						<div className="text-sm text-[var(--text-strong)]">ngrok is {ngForm.enabled ? "enabled" : "disabled"}.</div>
					)}
					{ngStatus?.public_url ? (
						<a
							href={ngStatus.public_url}
							target="_blank"
							rel="noopener"
							className="text-sm text-[var(--accent)] underline break-all"
						>
							{ngStatus.public_url}
						</a>
					) : null}
					{ngError ? <ErrorPanel message={ngError} /> : null}
					{ngStatus?.passkey_warning ? (
						<div className="alert-warning-text max-w-form">{ngStatus.passkey_warning}</div>
					) : null}
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]" htmlFor="onboarding-ngrok-authtoken">
							Authtoken
						</label>
						<input
							id="onboarding-ngrok-authtoken"
							type="password"
							className="provider-key-input w-full"
							placeholder={
								ngStatus?.authtoken_source ? "Leave blank to keep the current token" : "Paste your ngrok authtoken"
							}
							value={ngForm.authtoken}
							onInput={(e) => setNgForm({ ...ngForm, authtoken: targetValue(e) })}
						/>
						<div className="text-xs text-[var(--muted)]">
							Create or copy an authtoken from{" "}
							<a
								href="https://dashboard.ngrok.com/get-started/your-authtoken"
								target="_blank"
								rel="noopener"
								className="text-[var(--accent)] underline"
							>
								ngrok dashboard
							</a>
							.
						</div>
					</div>
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]" htmlFor="onboarding-ngrok-domain">
							Reserved domain (optional)
						</label>
						<input
							id="onboarding-ngrok-domain"
							type="text"
							className="provider-key-input w-full"
							placeholder="team-gateway.ngrok.app"
							value={ngForm.domain}
							onInput={(e) => setNgForm({ ...ngForm, domain: targetValue(e) })}
						/>
						<div className="text-xs text-[var(--muted)]">
							Use a reserved domain if you want a stable public hostname.
						</div>
					</div>
					{ngMsg ? <div className="text-xs text-[var(--ok)]">{ngMsg}</div> : null}
					<button
						type="button"
						className="provider-btn self-start"
						disabled={!ngrokAvailable || ngLoading || ngSaving}
						onClick={toggleNgrokEnabled}
					>
						{ngSaving ? "Applying\u2026" : ngForm.enabled ? "Disable ngrok" : "Enable ngrok"}
					</button>
				</div>
			)}

			{remoteTab === "netbird" && (
				<div className="flex flex-col gap-4">
					<p className="text-xs text-[var(--muted)] leading-relaxed">
						Private mesh access through NetBird. Use this when remote clients are members of your NetBird network.
					</p>
					{nbLoading ? (
						<div className="text-xs text-[var(--muted)]">Loading NetBird status&hellip;</div>
					) : (
						<div className="text-sm text-[var(--text-strong)]">
							NetBird serve is {netbirdServeEnabled ? "enabled" : "disabled"}.
						</div>
					)}
					{nbStatus?.url ? (
						<a
							href={nbStatus.url}
							target="_blank"
							rel="noopener"
							className="text-sm text-[var(--accent)] underline break-all"
						>
							{nbStatus.url}
						</a>
					) : null}
					{nbError ? <ErrorPanel message={nbError} /> : null}
					{nbStatus?.installed === false ? (
						<a
							href="https://docs.netbird.io/how-to/installation"
							target="_blank"
							rel="noopener"
							className="provider-btn self-start no-underline"
						>
							Install NetBird
						</a>
					) : null}
					{nbStatus?.netbird_up === false ? (
						<div className="alert-warning-text max-w-form">
							<span className="alert-label-warn">Warning:</span> Install NetBird and connect this peer with{" "}
							<code className="font-mono">netbird up</code> or the NetBird app before enabling serve.
						</div>
					) : null}
					<button
						type="button"
						className="provider-btn self-start"
						disabled={nbLoading || nbConfiguring || netbirdBlocked}
						onClick={toggleNetbirdServe}
					>
						{nbConfiguring ? "Applying\u2026" : netbirdServeEnabled ? "Disable NetBird Serve" : "Enable NetBird Serve"}
					</button>
				</div>
			)}

			{remoteTab === "cloudflare" && (
				<div className="flex flex-col gap-4">
					<p className="text-xs text-[var(--muted)] leading-relaxed">
						Public HTTPS through Cloudflare Tunnel. This is useful when your public DNS is managed by Cloudflare.
					</p>
					{cfLoading ? (
						<div className="text-xs text-[var(--muted)]">Loading Cloudflare Tunnel status&hellip;</div>
					) : (
						<div className="text-sm text-[var(--text-strong)]">
							Cloudflare Tunnel is {cfForm.enabled ? "enabled" : "disabled"}.
						</div>
					)}
					{cfStatus?.public_url ? (
						<a
							href={cfStatus.public_url}
							target="_blank"
							rel="noopener"
							className="text-sm text-[var(--accent)] underline break-all"
						>
							{cfStatus.public_url}
						</a>
					) : null}
					{cfError ? <ErrorPanel message={cfError} /> : null}
					{cfStatus?.passkey_warning ? (
						<div className="alert-warning-text max-w-form">{cfStatus.passkey_warning}</div>
					) : null}
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]" htmlFor="onboarding-cloudflare-token">
							Tunnel token
						</label>
						<input
							id="onboarding-cloudflare-token"
							type="password"
							className="provider-key-input w-full"
							placeholder={
								cfStatus?.token_source ? "Leave blank to keep the current token" : "Paste your Cloudflare Tunnel token"
							}
							value={cfForm.token}
							onInput={(e) => setCfForm({ ...cfForm, token: targetValue(e) })}
						/>
					</div>
					<div className="flex flex-col gap-1">
						<label className="text-xs text-[var(--muted)]" htmlFor="onboarding-cloudflare-hostname">
							Public hostname (optional)
						</label>
						<input
							id="onboarding-cloudflare-hostname"
							type="text"
							className="provider-key-input w-full"
							placeholder="moltis.example.com"
							value={cfForm.hostname}
							onInput={(e) => setCfForm({ ...cfForm, hostname: targetValue(e) })}
						/>
						<div className="text-xs text-[var(--muted)]">
							Use a hostname if you want the UI to display your stable tunnel URL immediately.
						</div>
					</div>
					{authReady ? null : (
						<div className="alert-warning-text max-w-form">
							<span className="alert-label-warn">Warning:</span> Remote visitors will see the setup-required page until
							authentication is configured.
						</div>
					)}
					{cfMsg ? <div className="text-xs text-[var(--ok)]">{cfMsg}</div> : null}
					<button
						type="button"
						className="provider-btn self-start"
						disabled={!cloudflareAvailable || cfLoading || cfSaving}
						onClick={toggleCloudflareTunnelEnabled}
					>
						{cfSaving ? "Applying\u2026" : cfForm.enabled ? "Disable Cloudflare" : "Enable Cloudflare"}
					</button>
				</div>
			)}

			<div className="flex flex-wrap items-center gap-3 mt-1">
				<button type="button" className="provider-btn provider-btn-secondary" onClick={onBack}>
					{t("common:actions.back")}
				</button>
				<button type="button" className="provider-btn" onClick={onNext}>
					{t("common:actions.continue")}
				</button>
				<button
					type="button"
					className="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
					onClick={onNext}
				>
					Skip for now
				</button>
			</div>
		</div>
	);
}

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { Badge, SaveButton, StatusMessage, TextField } from "../../components/forms";
import { localizedApiErrorMessage } from "../../helpers";

export type RemoteBackendId = "vercel" | "daytona" | "coder";

export const REMOTE_BACKEND_TABS: Array<{ id: RemoteBackendId; label: string }> = [
	{ id: "vercel", label: "Vercel" },
	{ id: "daytona", label: "Daytona" },
	{ id: "coder", label: "Coder" },
];

interface RemoteBackendsConfig {
	vercel: {
		configured: boolean;
		from_env?: boolean;
		project_id?: string;
		team_id?: string;
		runtime: string;
		timeout_ms: number;
		vcpus: number;
	};
	daytona: {
		configured: boolean;
		from_env?: boolean;
		api_url: string;
		target?: string;
	};
	coder?: {
		configured: boolean;
		url_configured: boolean;
		url_from_env: boolean;
		url?: string;
		token_configured: boolean;
		token_from_env: boolean;
		template_configured: boolean;
		organization?: string;
		user: string;
		template_id?: string;
		template_name?: string;
		workspace_prefix: string;
		ttl_ms?: number;
		size?: string;
	};
}

const remoteConfig = signal<RemoteBackendsConfig | null>(null);
const remoteLoading = signal(false);
const remoteSaving = signal("");
const remoteMsg = signal("");
const remoteErr = signal("");
const vercelToken = signal("");
const vercelProjectId = signal("");
const vercelTeamId = signal("");
const daytonaApiKey = signal("");
const daytonaApiUrl = signal("");
const coderToken = signal("");
const coderUrl = signal("");
const coderOrganization = signal("");
const coderUser = signal("me");
const coderTemplateId = signal("");
const coderTemplateName = signal("");
const coderWorkspacePrefix = signal("moltis");
const coderTtlMs = signal("");
const coderSize = signal("");

async function responseErrorMessage(response: Response, fallback: string): Promise<string> {
	try {
		const payload = (await response.json()) as {
			code?: string;
			error?: string | { code?: string; message?: string };
			message?: string;
		};
		if (typeof payload.error === "string" && payload.error.trim()) return payload.error;
		return localizedApiErrorMessage(payload, fallback);
	} catch {
		return fallback;
	}
}

function applyRemoteConfig(data: RemoteBackendsConfig): void {
	remoteConfig.value = data;
	vercelProjectId.value = data.vercel?.project_id || "";
	vercelTeamId.value = data.vercel?.team_id || "";
	daytonaApiUrl.value = data.daytona?.api_url || "https://app.daytona.io/api";
	if (data.coder) {
		coderUrl.value = data.coder.url || "";
		coderOrganization.value = data.coder.organization || "";
		coderUser.value = data.coder.user || "me";
		coderTemplateId.value = data.coder.template_id || "";
		coderTemplateName.value = data.coder.template_name || "";
		coderWorkspacePrefix.value = data.coder.workspace_prefix || "moltis";
		coderTtlMs.value = data.coder.ttl_ms == null ? "" : String(data.coder.ttl_ms);
		coderSize.value = data.coder.size || "";
	}
}

export function fetchRemoteBackends(): void {
	remoteLoading.value = true;
	remoteErr.value = "";
	fetch("/api/sandbox/remote-backends")
		.then(async (response) => {
			if (!response.ok) {
				throw new Error(await responseErrorMessage(response, "Failed to load remote backend config."));
			}
			return response.json() as Promise<RemoteBackendsConfig>;
		})
		.then(applyRemoteConfig)
		.catch((error: Error) => {
			remoteErr.value = error.message;
		})
		.finally(() => {
			remoteLoading.value = false;
		});
}

export function resetRemoteBackends(): void {
	remoteConfig.value = null;
	remoteLoading.value = false;
	remoteSaving.value = "";
	remoteMsg.value = "";
	remoteErr.value = "";
	vercelToken.value = "";
	vercelProjectId.value = "";
	vercelTeamId.value = "";
	daytonaApiKey.value = "";
	daytonaApiUrl.value = "";
	coderToken.value = "";
	coderUrl.value = "";
	coderOrganization.value = "";
	coderUser.value = "me";
	coderTemplateId.value = "";
	coderTemplateName.value = "";
	coderWorkspacePrefix.value = "moltis";
	coderTtlMs.value = "";
	coderSize.value = "";
}

function saveRemoteBackend(backend: string, config: Record<string, unknown>): void {
	remoteSaving.value = backend;
	remoteErr.value = "";
	remoteMsg.value = "";
	fetch("/api/sandbox/remote-backends", {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ backend, config }),
	})
		.then(async (response) => {
			if (!response.ok) {
				throw new Error(await responseErrorMessage(response, "Failed to save remote backend config."));
			}
			return response.json();
		})
		.then((data) => {
			if (data?.config) applyRemoteConfig(data.config as RemoteBackendsConfig);
			if (backend !== "_global") {
				remoteMsg.value = `${backend} configuration saved. Restart Moltis to apply.`;
				vercelToken.value = "";
				daytonaApiKey.value = "";
				coderToken.value = "";
			}
		})
		.catch((error: Error) => {
			remoteErr.value = error.message;
		})
		.finally(() => {
			remoteSaving.value = "";
		});
}

export function saveDefaultBackend(backend: string): void {
	saveRemoteBackend("_global", { backend });
}

interface BackendHeaderProps {
	title: string;
	configured: boolean;
	description: string;
}

function BackendHeader({ title, configured, description }: BackendHeaderProps): VNode {
	return (
		<>
			<div className="mb-2 flex items-center gap-2">
				<h3 className="text-sm font-medium text-[var(--text-strong)]">{title}</h3>
				<Badge label={configured ? "configured" : "not configured"} variant={configured ? "configured" : "muted"} />
			</div>
			<p className="mb-3 text-xs leading-relaxed text-[var(--muted)]">{description}</p>
		</>
	);
}

function BackendStatus({ backend }: { backend: RemoteBackendId }): VNode {
	return (
		<StatusMessage
			error={remoteErr.value || null}
			success={remoteMsg.value.includes(backend) ? remoteMsg.value : null}
		/>
	);
}

function VercelTabContent(): VNode {
	const config = remoteConfig.value;
	const save = (): void => {
		const update: Record<string, unknown> = {};
		if (vercelToken.value.trim()) update.token = vercelToken.value.trim();
		if (vercelProjectId.value.trim()) update.project_id = vercelProjectId.value.trim();
		if (vercelTeamId.value.trim()) update.team_id = vercelTeamId.value.trim();
		saveRemoteBackend("vercel", update);
	};

	return (
		<div className="max-w-form">
			<BackendHeader
				title="Vercel Sandbox"
				configured={config?.vercel?.configured ?? false}
				description="Firecracker microVMs via the Vercel API. Each session gets an ephemeral isolated VM with millisecond boot times."
			/>
			<TextField
				label="Vercel token"
				type="password"
				value={vercelToken.value}
				onInput={(value) => {
					vercelToken.value = value;
				}}
				placeholder={
					config?.vercel?.from_env
						? "•••••••• (set via VERCEL_TOKEN env var)"
						: config?.vercel?.configured
							? "•••••••• (set in config)"
							: "Vercel token (VERCEL_TOKEN)"
				}
				disabled={config?.vercel?.from_env}
				monospace
				help={
					config?.vercel?.from_env
						? "Managed by VERCEL_TOKEN. Remove it from the environment to configure here."
						: undefined
				}
			/>
			<div className="grid gap-3 sm:grid-cols-2">
				<TextField
					label="Project ID"
					value={vercelProjectId.value}
					onInput={(value) => {
						vercelProjectId.value = value;
					}}
					required
					monospace
				/>
				<TextField
					label="Team ID"
					value={vercelTeamId.value}
					onInput={(value) => {
						vercelTeamId.value = value;
					}}
					monospace
				/>
			</div>
			<SaveButton
				saving={remoteSaving.value === "vercel"}
				disabled={!(vercelToken.value.trim() && vercelProjectId.value.trim())}
				onClick={save}
			/>
			<BackendStatus backend="vercel" />
		</div>
	);
}

function DaytonaTabContent(): VNode {
	const config = remoteConfig.value;
	const save = (): void => {
		const update: Record<string, unknown> = {};
		if (daytonaApiKey.value.trim()) update.api_key = daytonaApiKey.value.trim();
		if (daytonaApiUrl.value.trim()) update.api_url = daytonaApiUrl.value.trim();
		saveRemoteBackend("daytona", update);
	};

	return (
		<div className="max-w-form">
			<BackendHeader
				title="Daytona"
				configured={config?.daytona?.configured ?? false}
				description="Open-source cloud sandboxes. Self-host on your own infrastructure or use the managed Daytona service."
			/>
			<TextField
				label="Daytona API key"
				type="password"
				value={daytonaApiKey.value}
				onInput={(value) => {
					daytonaApiKey.value = value;
				}}
				placeholder={
					config?.daytona?.from_env
						? "•••••••• (set via DAYTONA_API_KEY env var)"
						: config?.daytona?.configured
							? "•••••••• (set in config)"
							: "Daytona API key (DAYTONA_API_KEY)"
				}
				disabled={config?.daytona?.from_env}
				monospace
				help={
					config?.daytona?.from_env
						? "Managed by DAYTONA_API_KEY. Remove it from the environment to configure here."
						: undefined
				}
			/>
			<TextField
				label="API URL"
				value={daytonaApiUrl.value}
				onInput={(value) => {
					daytonaApiUrl.value = value;
				}}
				placeholder="https://app.daytona.io/api"
				monospace
			/>
			<SaveButton saving={remoteSaving.value === "daytona"} disabled={!daytonaApiKey.value.trim()} onClick={save} />
			<BackendStatus backend="daytona" />
		</div>
	);
}

function CoderTabContent(): VNode {
	const config = remoteConfig.value?.coder;
	const ttl = coderTtlMs.value.trim();
	const ttlValid = !ttl || (Number.isSafeInteger(Number(ttl)) && Number(ttl) >= 0);
	const hasTemplate = Boolean(coderTemplateId.value.trim() || coderTemplateName.value.trim());
	const hasToken = Boolean(coderToken.value.trim() || config?.token_configured);
	const canSave = Boolean(coderUrl.value.trim() && hasToken && hasTemplate && ttlValid);
	const configured = Boolean(config?.url_configured && config.token_configured && config.template_configured);

	const save = (): void => {
		const update: Record<string, unknown> = {
			organization: coderOrganization.value.trim() || null,
			user: coderUser.value.trim() || null,
			template_id: coderTemplateId.value.trim() || null,
			template_name: coderTemplateName.value.trim() || null,
			workspace_prefix: coderWorkspacePrefix.value.trim() || null,
			ttl_ms: ttl ? Number(ttl) : null,
			size: coderSize.value.trim() || null,
		};
		if (!config?.url_from_env) update.url = coderUrl.value.trim();
		if (!config?.token_from_env && coderToken.value.trim()) update.token = coderToken.value.trim();
		saveRemoteBackend("coder", update);
	};

	return (
		<div className="max-w-form">
			<BackendHeader
				title="Coder"
				configured={configured}
				description="Ephemeral Coder workspaces created from your template, with commands and files transported through the workspace agent."
			/>
			<TextField
				label="Coder URL"
				type="url"
				value={coderUrl.value}
				onInput={(value) => {
					coderUrl.value = value;
				}}
				placeholder="https://coder.example.com"
				disabled={config?.url_from_env}
				required
				monospace
				help={
					config?.url_from_env
						? "Managed by CODER_URL. Remove it from the environment to configure here."
						: "HTTPS is required except for localhost or a literal loopback address."
				}
			/>
			<TextField
				label="Coder session token"
				type="password"
				value={coderToken.value}
				onInput={(value) => {
					coderToken.value = value;
				}}
				placeholder={config?.token_configured ? "•••••••• (already configured)" : "CODER_SESSION_TOKEN"}
				disabled={config?.token_from_env}
				required
				monospace
				help={
					config?.token_from_env
						? "Managed by CODER_SESSION_TOKEN. Remove it from the environment to configure here."
						: undefined
				}
			/>
			<fieldset className="mb-3 rounded border border-[var(--border)] p-3">
				<legend className="px-1 text-xs text-[var(--muted)]">Template (one required)</legend>
				<div className="grid gap-3 sm:grid-cols-2">
					<TextField
						label="Template ID"
						value={coderTemplateId.value}
						onInput={(value) => {
							coderTemplateId.value = value;
						}}
						monospace
						className="mb-0"
					/>
					<TextField
						label="Template name"
						value={coderTemplateName.value}
						onInput={(value) => {
							coderTemplateName.value = value;
						}}
						monospace
						className="mb-0"
					/>
				</div>
			</fieldset>
			<div className="grid gap-3 sm:grid-cols-2">
				<TextField
					label="Organization"
					value={coderOrganization.value}
					onInput={(value) => {
						coderOrganization.value = value;
					}}
				/>
				<TextField
					label="User"
					value={coderUser.value}
					onInput={(value) => {
						coderUser.value = value;
					}}
					placeholder="me"
				/>
				<TextField
					label="Workspace prefix"
					value={coderWorkspacePrefix.value}
					onInput={(value) => {
						coderWorkspacePrefix.value = value;
					}}
					placeholder="moltis"
				/>
				<TextField
					label="TTL (milliseconds)"
					type="number"
					value={coderTtlMs.value}
					onInput={(value) => {
						coderTtlMs.value = value;
					}}
					help={ttlValid ? "Zero disables automatic workspace shutdown." : "TTL must be a non-negative whole number."}
				/>
				<TextField
					label="Size or preset"
					value={coderSize.value}
					onInput={(value) => {
						coderSize.value = value;
					}}
					placeholder="medium"
				/>
			</div>
			<SaveButton saving={remoteSaving.value === "coder"} disabled={!canSave} onClick={save} />
			<BackendStatus backend="coder" />
		</div>
	);
}

export function RemoteBackendTabContent({ backend }: { backend: RemoteBackendId }): VNode {
	if (remoteLoading.value && !remoteConfig.value) {
		return <div className="text-xs text-[var(--muted)]">Loading remote backend settings…</div>;
	}
	if (backend === "vercel") return <VercelTabContent />;
	if (backend === "daytona") return <DaytonaTabContent />;
	return <CoderTabContent />;
}

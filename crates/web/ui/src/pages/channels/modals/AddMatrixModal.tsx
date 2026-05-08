// ── Add Matrix modal ─────────────────────────────────────────

import { useSignal } from "@preact/signals";
import type { VNode } from "preact";
import { useRef } from "preact/hooks";

import {
	addChannel,
	deriveMatrixAccountId,
	fetchChannelStatus,
	MATRIX_DEFAULT_HOMESERVER,
	MATRIX_DOCS_URL,
	MATRIX_ENCRYPTION_GUIDANCE,
	matrixAuthModeGuidance,
	matrixCredentialLabel,
	matrixCredentialPlaceholder,
	matrixOwnershipModeGuidance,
	normalizeMatrixAuthMode,
	normalizeMatrixOtpCooldown,
	normalizeMatrixOwnershipMode,
	parseChannelConfigPatch,
	validateChannelFields,
} from "../../../channel-utils";
import { sendRpc } from "../../../helpers";
import { models as modelsSig } from "../../../stores/model-store";
import { targetChecked, targetValue } from "../../../typed-events";
import { ChannelType } from "../../../types";
import { Modal, ModelSelect } from "../../../ui";
import { type ChannelConfig, ConnectionModeHint, loadChannels, showAddMatrix } from "../../ChannelsPage";
import { AdvancedConfigPatchField, AllowlistInput } from "../ChannelFields";

export function AddMatrixModal(): VNode {
	const error = useSignal("");
	const saving = useSignal(false);
	const addModel = useSignal("");
	const userAllowlistItems = useSignal<string[]>([]);
	const roomAllowlistItems = useSignal<string[]>([]);
	const homeserverDraft = useSignal(MATRIX_DEFAULT_HOMESERVER);
	const authModeDraft = useSignal("oidc");
	const userIdDraft = useSignal("");
	const credentialDraft = useSignal("");
	const deviceDisplayNameDraft = useSignal("");
	const ownershipModeDraft = useSignal("moltis_owned");
	const oidcWaiting = useSignal(false);
	const oidcPollRef = useRef<ReturnType<typeof setInterval> | null>(null);
	const otpSelfApprovalDraft = useSignal(true);
	const otpCooldownDraft = useSignal("300");
	const advancedConfigPatch = useSignal("");

	function resetForm(): void {
		if (oidcPollRef.current) {
			clearInterval(oidcPollRef.current);
			oidcPollRef.current = null;
		}
		addModel.value = "";
		userAllowlistItems.value = [];
		roomAllowlistItems.value = [];
		homeserverDraft.value = MATRIX_DEFAULT_HOMESERVER;
		authModeDraft.value = "oidc";
		userIdDraft.value = "";
		credentialDraft.value = "";
		deviceDisplayNameDraft.value = "";
		ownershipModeDraft.value = "moltis_owned";
		otpSelfApprovalDraft.value = true;
		otpCooldownDraft.value = "300";
		advancedConfigPatch.value = "";
		oidcWaiting.value = false;
	}

	function onSubmit(e: Event): void {
		e.preventDefault();
		const form = (e.target as HTMLElement).closest(".channel-form") as HTMLElement;
		const authMode = normalizeMatrixAuthMode(authModeDraft.value);
		const credential = credentialDraft.value.trim();
		const homeserver = homeserverDraft.value.trim();
		const userId = userIdDraft.value.trim();
		const accountId = deriveMatrixAccountId({ userId, homeserver });
		const v = validateChannelFields(ChannelType.Matrix, accountId, credential, {
			matrixAuthMode: authMode,
			matrixUserId: userId,
		});
		if (!v.valid) {
			error.value = v.error;
			return;
		}
		if (!homeserver) {
			error.value = "Homeserver URL is required.";
			return;
		}
		const advancedPatch = parseChannelConfigPatch(advancedConfigPatch.value);
		if (!advancedPatch.ok) {
			error.value = advancedPatch.error;
			return;
		}
		error.value = "";
		saving.value = true;

		if (authMode === "oidc") {
			const redirectUri = `${window.location.origin}/auth/callback`;
			const oidcConfig: ChannelConfig = {
				homeserver,
				ownership_mode: normalizeMatrixOwnershipMode(ownershipModeDraft.value),
				dm_policy: (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement).value,
				room_policy: (form.querySelector("[data-field=roomPolicy]") as HTMLSelectElement).value,
				mention_mode: (form.querySelector("[data-field=mentionMode]") as HTMLSelectElement).value,
				auto_join: (form.querySelector("[data-field=autoJoin]") as HTMLSelectElement).value,
				user_allowlist: userAllowlistItems.value,
				room_allowlist: roomAllowlistItems.value,
				otp_self_approval: otpSelfApprovalDraft.value,
				otp_cooldown_secs: normalizeMatrixOtpCooldown(otpCooldownDraft.value),
			};
			if (deviceDisplayNameDraft.value.trim()) oidcConfig.device_display_name = deviceDisplayNameDraft.value.trim();
			if (addModel.value) {
				oidcConfig.model = addModel.value;
				const oidcModel = modelsSig.value.find((x) => x.id === addModel.value);
				if (oidcModel?.provider) oidcConfig.model_provider = oidcModel.provider;
			}
			Object.assign(oidcConfig, advancedPatch.value);
			sendRpc("channels.oauth_start", {
				account_id: accountId,
				homeserver,
				redirect_uri: redirectUri,
				config: oidcConfig,
			}).then((res) => {
				const r = res as {
					ok?: boolean;
					payload?: { auth_url?: string };
					error?: { message?: string; detail?: string };
				};
				if (r?.ok && r.payload?.auth_url) {
					oidcWaiting.value = true;
					saving.value = false;
					window.open(r.payload.auth_url, "_blank", "noopener");
					let pollCount = 0;
					oidcPollRef.current = setInterval(() => {
						pollCount++;
						if (pollCount > 120) {
							clearInterval(oidcPollRef.current!);
							oidcPollRef.current = null;
							oidcWaiting.value = false;
							error.value = "OIDC authentication timed out. Please try again.";
							return;
						}
						fetchChannelStatus().then((statusRes: unknown) => {
							const sr = statusRes as {
								ok?: boolean;
								payload?: { channels?: Array<{ account_id?: string; status?: string }> };
							};
							if (!sr?.ok) return;
							const channels = sr.payload?.channels || [];
							if (channels.some((ch) => ch.account_id === accountId && ch.status === "connected")) {
								clearInterval(oidcPollRef.current!);
								oidcPollRef.current = null;
								oidcWaiting.value = false;
								showAddMatrix.value = false;
								resetForm();
								loadChannels();
							}
						});
					}, 1000);
				} else {
					saving.value = false;
					error.value = r?.error?.message || r?.error?.detail || "Failed to start OIDC login.";
				}
			});
			return;
		}

		const addConfig: ChannelConfig = {
			homeserver,
			ownership_mode: authMode === "password" ? normalizeMatrixOwnershipMode(ownershipModeDraft.value) : "user_managed",
			dm_policy: (form.querySelector("[data-field=dmPolicy]") as HTMLSelectElement).value,
			room_policy: (form.querySelector("[data-field=roomPolicy]") as HTMLSelectElement).value,
			mention_mode: (form.querySelector("[data-field=mentionMode]") as HTMLSelectElement).value,
			auto_join: (form.querySelector("[data-field=autoJoin]") as HTMLSelectElement).value,
			user_allowlist: userAllowlistItems.value,
			room_allowlist: roomAllowlistItems.value,
			otp_self_approval: otpSelfApprovalDraft.value,
			otp_cooldown_secs: normalizeMatrixOtpCooldown(otpCooldownDraft.value),
		};
		if (authMode === "password") {
			addConfig.password = credential;
		} else {
			addConfig.access_token = credential;
		}
		if (userId) addConfig.user_id = userId;
		if (deviceDisplayNameDraft.value.trim()) addConfig.device_display_name = deviceDisplayNameDraft.value.trim();
		if (addModel.value) {
			addConfig.model = addModel.value;
			const found = modelsSig.value.find((x) => x.id === addModel.value);
			if (found?.provider) addConfig.model_provider = found.provider;
		}
		Object.assign(addConfig, advancedPatch.value);
		addChannel(ChannelType.Matrix, accountId, addConfig).then((res: unknown) => {
			saving.value = false;
			const r = res as { ok?: boolean; error?: { message?: string; detail?: string } } | undefined;
			if (r?.ok) {
				showAddMatrix.value = false;
				resetForm();
				loadChannels();
			} else {
				error.value = r?.error?.message || r?.error?.detail || "Failed to connect Matrix.";
			}
		});
	}

	const defaultPlaceholder =
		modelsSig.value.length > 0
			? `(default: ${modelsSig.value[0].displayName || modelsSig.value[0].id})`
			: "(server default)";

	return (
		<Modal
			show={showAddMatrix.value}
			onClose={() => {
				showAddMatrix.value = false;
			}}
			title="Connect Matrix"
		>
			<div className="channel-form">
				<div className="channel-card">
					<div>
						<span className="text-xs font-medium text-[var(--text-strong)]">Connect a Matrix bot user</span>
						<div className="text-xs text-[var(--muted)] channel-help">
							1. Leave the homeserver as <span className="font-mono">{MATRIX_DEFAULT_HOMESERVER}</span> for matrix.org
							accounts
						</div>
						<div className="text-xs text-[var(--muted)]">
							2. OIDC is the default because it is the simplest and supports encrypted Matrix chats. Password also
							supports encryption. Access token auth is only for plain Matrix traffic
						</div>
						<div className="text-xs text-[var(--muted)]">
							3. Moltis generates the local account ID automatically from the Matrix user or homeserver
						</div>
					</div>
				</div>
				<div className="rounded-md border border-emerald-600/30 bg-emerald-50 px-3 py-2 text-xs text-emerald-900">
					<div className="font-medium text-emerald-800">Encrypted chats require OIDC or Password auth</div>
					<div>{MATRIX_ENCRYPTION_GUIDANCE}</div>
				</div>
				<ConnectionModeHint type={ChannelType.Matrix} />
				<label className="text-xs text-[var(--muted)]">Homeserver URL</label>
				<input
					data-field="homeserver"
					type="text"
					placeholder={MATRIX_DEFAULT_HOMESERVER}
					value={homeserverDraft.value}
					onInput={(e) => {
						homeserverDraft.value = targetValue(e);
					}}
					className="channel-input"
					autoComplete="off"
					autoCapitalize="none"
					autoCorrect="off"
					spellcheck={false}
				/>
				<label className="text-xs text-[var(--muted)]">Authentication</label>
				<select
					data-field="authMode"
					className="channel-select"
					value={authModeDraft.value}
					onChange={(e) => {
						authModeDraft.value = normalizeMatrixAuthMode(targetValue(e));
					}}
				>
					<option value="oidc">OIDC (recommended)</option>
					<option value="password">Password</option>
					<option value="access_token">Access token</option>
				</select>
				<div className="text-xs text-[var(--muted)]">{matrixAuthModeGuidance(authModeDraft.value)}</div>
				{authModeDraft.value === "password" || authModeDraft.value === "oidc" ? (
					<label className="flex items-start gap-2 rounded-md border border-[var(--border)] bg-[var(--surface2)] px-3 py-2">
						<input
							type="checkbox"
							aria-label="Let Moltis own this Matrix account"
							checked={normalizeMatrixOwnershipMode(ownershipModeDraft.value) === "moltis_owned"}
							onChange={(e) => {
								ownershipModeDraft.value = targetChecked(e) ? "moltis_owned" : "user_managed";
							}}
						/>
						<span className="flex flex-col gap-1">
							<span className="text-xs font-medium text-[var(--text-strong)]">Let Moltis own this Matrix account</span>
							<span className="text-xs text-[var(--muted)]">
								{matrixOwnershipModeGuidance(authModeDraft.value, ownershipModeDraft.value)}
							</span>
						</span>
					</label>
				) : (
					<div className="text-xs text-[var(--muted)]">
						{matrixOwnershipModeGuidance(authModeDraft.value, "user_managed")}
					</div>
				)}
				{authModeDraft.value !== "oidc" && (
					<>
						<label className="text-xs text-[var(--muted)]">
							Matrix User ID{authModeDraft.value === "password" ? " (required)" : " (optional)"}
						</label>
						<input
							data-field="userId"
							type="text"
							placeholder="@bot:example.com"
							value={userIdDraft.value}
							onInput={(e) => {
								userIdDraft.value = targetValue(e);
							}}
							className="channel-input"
						/>
						<label className="text-xs text-[var(--muted)]">{matrixCredentialLabel(authModeDraft.value)}</label>
						<input
							data-field="credential"
							type="password"
							placeholder={matrixCredentialPlaceholder(authModeDraft.value)}
							value={credentialDraft.value}
							onInput={(e) => {
								credentialDraft.value = targetValue(e);
							}}
							className="channel-input"
							autoComplete="new-password"
							autoCapitalize="none"
							autoCorrect="off"
							spellcheck={false}
						/>
						<div className="text-xs text-[var(--muted)]">
							{authModeDraft.value === "password" ? (
								"Use the password for the dedicated Matrix bot account. This is the required mode for encrypted Matrix chats because Moltis needs to create and persist its own Matrix device keys."
							) : (
								<>
									Get the access token in Element:{" "}
									<span className="font-mono">Settings -&gt; Help & About -&gt; Advanced -&gt; Access Token</span>.
									Access token mode does <span className="font-medium">not</span> support encrypted Matrix chats because
									Moltis cannot import that existing device's private encryption keys.
								</>
							)}{" "}
							<a href={MATRIX_DOCS_URL} target="_blank" rel="noreferrer" className="text-[var(--accent)] underline">
								Matrix setup docs
							</a>
						</div>
					</>
				)}
				<label className="text-xs text-[var(--muted)]">Device Display Name (optional)</label>
				<input
					data-field="deviceDisplayName"
					type="text"
					placeholder="Moltis Matrix Bot"
					value={deviceDisplayNameDraft.value}
					onInput={(e) => {
						deviceDisplayNameDraft.value = targetValue(e);
					}}
					className="channel-input"
				/>
				<label className="text-xs text-[var(--muted)]">DM Policy</label>
				<select data-field="dmPolicy" className="channel-select">
					<option value="allowlist">Allowlist only</option>
					<option value="open">Open (anyone)</option>
					<option value="disabled">Disabled</option>
				</select>
				<label className="text-xs text-[var(--muted)]">Room Policy</label>
				<select data-field="roomPolicy" className="channel-select">
					<option value="allowlist">Room allowlist only</option>
					<option value="open">Open (any joined room)</option>
					<option value="disabled">Disabled</option>
				</select>
				<label className="text-xs text-[var(--muted)]">Room Mention Mode</label>
				<select data-field="mentionMode" className="channel-select">
					<option value="mention">Must mention bot</option>
					<option value="always">Always respond</option>
					<option value="none">Never respond in rooms</option>
				</select>
				<label className="text-xs text-[var(--muted)]">Invite Auto-Join</label>
				<select data-field="autoJoin" className="channel-select">
					<option value="always">Always join invites</option>
					<option value="allowlist">Only when inviter or room is allowlisted</option>
					<option value="off">Do not auto-join</option>
				</select>
				<label className="text-xs text-[var(--muted)]">Unknown DM Approval</label>
				<select
					data-field="otpSelfApproval"
					className="channel-select"
					value={otpSelfApprovalDraft.value ? "on" : "off"}
					onChange={(e) => {
						otpSelfApprovalDraft.value = targetValue(e) !== "off";
					}}
				>
					<option value="on">PIN challenge enabled (recommended)</option>
					<option value="off">Reject unknown DMs without a PIN</option>
				</select>
				<label className="text-xs text-[var(--muted)]">PIN Cooldown Seconds</label>
				<input
					data-field="otpCooldown"
					type="number"
					min={1}
					step={1}
					className="channel-input"
					value={otpCooldownDraft.value}
					onInput={(e) => {
						otpCooldownDraft.value = targetValue(e);
					}}
				/>
				<div className="text-xs text-[var(--muted)]">
					With DM policy on allowlist, unknown users get a 6-digit PIN challenge by default.
				</div>
				<label className="text-xs text-[var(--muted)]">Default Model</label>
				<ModelSelect
					models={modelsSig.value}
					value={addModel.value}
					onChange={(v: string) => {
						addModel.value = v;
					}}
					placeholder={defaultPlaceholder}
				/>
				<label className="text-xs text-[var(--muted)]">DM Allowlist (Matrix user IDs)</label>
				<AllowlistInput
					value={userAllowlistItems.value}
					preserveAt={true}
					onChange={(items) => {
						userAllowlistItems.value = items;
					}}
				/>
				<label className="text-xs text-[var(--muted)]">Room Allowlist (room IDs or aliases)</label>
				<AllowlistInput
					value={roomAllowlistItems.value}
					preserveAt={true}
					onChange={(items) => {
						roomAllowlistItems.value = items;
					}}
				/>
				<AdvancedConfigPatchField
					value={advancedConfigPatch.value}
					onInput={(value) => {
						advancedConfigPatch.value = value;
					}}
				/>
				{error.value && <div className="text-xs text-[var(--error)] py-1">{error.value}</div>}
				<button className="provider-btn" onClick={onSubmit} disabled={saving.value || oidcWaiting.value}>
					{saving.value
						? "Connecting\u2026"
						: oidcWaiting.value
							? "Waiting for OIDC\u2026"
							: authModeDraft.value === "oidc"
								? "Authenticate with OIDC"
								: "Connect Matrix"}
				</button>
			</div>
		</Modal>
	);
}

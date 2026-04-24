// ── Onboarding wizard ──────────────────────────────────────
//
// Multi-step setup page shown to first-time users.
// Steps: Auth (conditional) → Import (conditional) → Provider →
// Voice (conditional) → Skills → Remote Access → Channel → Identity → Summary
// No new Rust code — all existing RPC methods and REST endpoints.

import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { fetchChannelStatus } from "./channel-utils";
import { get as getGon, refresh as refreshGon } from "./gon";
import { sendRpc } from "./helpers";
import { t } from "./i18n";
// ── Sub-module imports ──────────────────────────────────────
import { ensureWsConnected, preferredChatPath } from "./onboarding/shared";
import { AuthStep } from "./onboarding/steps/AuthStep";
import { ChannelStep } from "./onboarding/steps/ChannelStep";
import { IdentityStep } from "./onboarding/steps/IdentityStep";
import { OpenClawImportStep } from "./onboarding/steps/OpenClawImportStep";
import { ProviderStep } from "./onboarding/steps/ProviderStep";
import { RemoteAccessStep } from "./onboarding/steps/RemoteAccessStep";
import { SkillsStep } from "./onboarding/steps/SkillsStep";
import { VoiceStep } from "./onboarding/steps/VoiceStep";
import type { IdentityInfo } from "./onboarding/types";
import { fetchVoiceProviders } from "./voice-utils";

// ── Step indicator ──────────────────────────────────────────

interface StepIndicatorProps {
	steps: string[];
	current: number;
}

function StepIndicator({ steps, current }: StepIndicatorProps): VNode {
	const ref = useRef<HTMLDivElement>(null);
	useEffect(() => {
		if (!ref.current) return;
		const active = ref.current.querySelector(".onboarding-step.active");
		if (active) active.scrollIntoView({ inline: "center", block: "nearest", behavior: "smooth" });
	}, [current]);
	return (
		<div className="onboarding-steps" ref={ref}>
			{steps.map((label, i) => {
				const state = i < current ? "completed" : i === current ? "active" : "";
				const isLast = i === steps.length - 1;
				return (
					<>
						<StepDot key={i} index={i} label={label} state={state} />
						{!isLast && <div className={`onboarding-step-line ${i < current ? "completed" : ""}`} />}
					</>
				);
			})}
		</div>
	);
}

function StepDot({ index, label, state }: { index: number; label: string; state: string }): VNode {
	return (
		<div className={`onboarding-step ${state}`}>
			<div className={`onboarding-step-dot ${state}`}>
				{state === "completed" ? <span className="icon icon-md icon-checkmark" /> : index + 1}
			</div>
			<div className="onboarding-step-label">{label}</div>
		</div>
	);
}

// ── Summary step helpers ─────────────────────────────────────

const LOW_MEMORY_THRESHOLD = 2 * 1024 * 1024 * 1024;

function formatMemBytes(bytes: number | null | undefined): string {
	if (bytes == null) return "?";
	const gb = bytes / (1024 * 1024 * 1024);
	return `${gb.toFixed(1)} GB`;
}

function CheckIcon(): VNode {
	return <span className="icon icon-check-circle shrink-0" style="color:var(--ok)" />;
}

function WarnIcon(): VNode {
	return <span className="icon icon-warn-triangle shrink-0" style="color:var(--warn)" />;
}

function ErrorIcon(): VNode {
	return <span className="icon icon-x-circle shrink-0" style="color:var(--error)" />;
}

function InfoIcon(): VNode {
	return <span className="icon icon-info-circle shrink-0" style="color:var(--muted)" />;
}

function SummaryRow({
	icon,
	label,
	children,
}: {
	icon: VNode;
	label: string;
	children: preact.ComponentChildren;
}): VNode {
	return (
		<div className="rounded-md border border-[var(--border)] bg-[var(--surface)] p-3 flex gap-3 items-start">
			<div className="mt-0.5">{icon}</div>
			<div className="flex-1 min-w-0">
				<div className="text-sm font-medium text-[var(--text-strong)]">{label}</div>
				<div className="text-xs text-[var(--muted)] mt-1">{children}</div>
			</div>
		</div>
	);
}

// ── Summary step types ──────────────────────────────────────

interface SummaryProvider {
	name: string;
	displayName: string;
	configured: boolean;
}

interface SummaryChannel {
	type: string;
	account_id: string;
	name?: string;
	status: string;
}

interface SummaryVoiceProvider {
	name: string;
	enabled: boolean;
}

interface SummaryVoice {
	tts: SummaryVoiceProvider[];
	stt: SummaryVoiceProvider[];
}

interface SummarySkills {
	enabledCategories: number;
	totalCategories: number;
	enabledSkills: number;
	totalSkills: number;
}

interface SummaryData {
	identity: IdentityInfo | null;
	mem: { total?: number; available?: number } | null;
	update: { available?: boolean; latest_version?: string; release_url?: string } | null;
	voiceEnabled: boolean;
	providers: SummaryProvider[];
	channels: SummaryChannel[];
	tailscale: { tailscale_up?: boolean; installed?: boolean } | null;
	voice: SummaryVoice | null;
	sandbox: { backend?: string } | null;
	skills: SummarySkills | null;
}

// ── SummaryStep ─────────────────────────────────────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: summary step fetches multiple data sources and renders conditional sections
function SummaryStep({ onBack, onFinish }: { onBack: () => void; onFinish: () => void }): VNode {
	const [loading, setLoading] = useState(true);
	const [data, setData] = useState<SummaryData | null>(null);

	useEffect(() => {
		let cancelled = false;

		// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: parallel data fetches and conditional gon reads
		async function load(): Promise<void> {
			await refreshGon();

			const identity = getGon("identity") as IdentityInfo | null;
			const mem = getGon("mem") as { total?: number; available?: number } | null;
			const update = getGon("update") as {
				available?: boolean;
				latest_version?: string;
				release_url?: string;
			} | null;
			const voiceEnabled = getGon("voice_enabled") === true;

			const [providersRes, channelsRes, tailscaleRes, voiceRes, bootstrapRes, skillsRes] = await Promise.all([
				(
					sendRpc("providers.available", {}) as Promise<{
						ok?: boolean;
						payload?: SummaryProvider[];
					}>
				).catch(() => null),
				(
					fetchChannelStatus() as Promise<{
						ok?: boolean;
						payload?: { channels?: SummaryChannel[] };
					}>
				).catch(() => null),
				fetch("/api/tailscale/status")
					.then((r) =>
						r.ok
							? (r.json() as Promise<{
									tailscale_up?: boolean;
									installed?: boolean;
								}>)
							: null,
					)
					.catch(() => null),
				voiceEnabled
					? (
							fetchVoiceProviders() as Promise<{
								ok?: boolean;
								payload?: SummaryVoice;
							}>
						).catch(() => null)
					: Promise.resolve(null),
				fetch(
					"/api/bootstrap?include_channels=false&include_sessions=false&include_models=false&include_projects=false&include_counts=false&include_identity=false",
				)
					.then((r) =>
						r.ok
							? (r.json() as Promise<{
									sandbox?: { backend?: string };
								}>)
							: null,
					)
					.catch(() => null),
				(
					sendRpc("skills.bundled.categories", {}) as Promise<{
						ok?: boolean;
						payload?: { categories?: { name: string; count: number; enabled: boolean }[]; total_skills?: number };
					}>
				).catch(() => null),
			]);

			if (cancelled) return;

			const skillsCats = skillsRes?.ok ? skillsRes.payload?.categories || [] : [];
			const skillsTotal = skillsRes?.ok ? skillsRes.payload?.total_skills || 0 : 0;
			const skillsEnabledCats = skillsCats.filter((c) => c.enabled);

			setData({
				identity,
				mem,
				update,
				voiceEnabled,
				providers: providersRes?.ok ? providersRes.payload || [] : [],
				channels: channelsRes?.ok ? channelsRes.payload?.channels || [] : [],
				tailscale: tailscaleRes,
				voice: voiceRes?.ok ? voiceRes.payload || { tts: [], stt: [] } : null,
				sandbox: bootstrapRes?.sandbox || null,
				skills: skillsCats.length
					? {
							enabledCategories: skillsEnabledCats.length,
							totalCategories: skillsCats.length,
							enabledSkills: skillsEnabledCats.reduce((sum, c) => sum + c.count, 0),
							totalSkills: skillsTotal,
						}
					: null,
			});
			setLoading(false);
		}

		load();
		return () => {
			cancelled = true;
		};
	}, []);

	if (loading || !data) {
		return (
			<div className="flex flex-col items-center justify-center gap-3 min-h-[200px]">
				<div className="inline-block w-8 h-8 border-2 border-[var(--border)] border-t-[var(--accent)] rounded-full animate-spin" />
				<div className="text-sm text-[var(--muted)]">{t("onboarding:summary.loadingSummary")}</div>
			</div>
		);
	}

	const activeModel = localStorage.getItem("moltis-model");
	const configuredProviders = data.providers.filter((p) => p.configured);

	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">{t("onboarding:summary.title")}</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed">
				Overview of your configuration. You can change any of these later in Settings.
			</p>

			<div className="flex flex-col gap-2">
				{/* Identity */}
				<SummaryRow
					icon={data.identity?.user_name && data.identity?.name ? <CheckIcon /> : <WarnIcon />}
					label="Identity"
				>
					{data.identity?.user_name && data.identity?.name ? (
						<>
							You: <span className="font-medium text-[var(--text)]">{data.identity.user_name}</span> Agent:{" "}
							<span className="font-medium text-[var(--text)]">
								{data.identity.emoji || ""} {data.identity.name}
							</span>
						</>
					) : (
						<span className="text-[var(--warn)]">Identity not fully configured</span>
					)}
				</SummaryRow>

				{/* LLMs */}
				<SummaryRow icon={configuredProviders.length > 0 ? <CheckIcon /> : <ErrorIcon />} label="LLMs">
					{configuredProviders.length > 0 ? (
						<div className="flex flex-col gap-1">
							<div className="flex flex-wrap gap-1">
								{configuredProviders.map((p) => (
									<span key={p.name} className="provider-item-badge configured">
										{p.displayName}
									</span>
								))}
							</div>
							{activeModel ? (
								<div>
									Active model: <span className="font-mono font-medium text-[var(--text)]">{activeModel}</span>
								</div>
							) : null}
						</div>
					) : (
						<span className="text-[var(--error)]">No LLM providers configured</span>
					)}
				</SummaryRow>

				{/* Channels */}
				<SummaryRow
					icon={
						data.channels.length > 0 ? (
							data.channels.some((c) => c.status === "error") ? (
								<ErrorIcon />
							) : data.channels.some((c) => c.status === "disconnected") ? (
								<WarnIcon />
							) : (
								<CheckIcon />
							)
						) : (
							<InfoIcon />
						)
					}
					label="Channels"
				>
					{data.channels.length > 0 ? (
						<div className="flex flex-col gap-1">
							{data.channels.map((ch) => {
								const statusColor =
									ch.status === "connected" ? "var(--ok)" : ch.status === "error" ? "var(--error)" : "var(--warn)";
								return (
									<div key={ch.account_id} className="flex items-center gap-1">
										<span style={`color:${statusColor}`}>{"\u25CF"}</span>
										<span className="font-medium text-[var(--text)]">{ch.type}</span>: {ch.name || ch.account_id}
										<span>({ch.status})</span>
									</div>
								);
							})}
						</div>
					) : (
						<>No channels configured</>
					)}
				</SummaryRow>

				{/* Skills */}
				{data.skills && (
					<SummaryRow icon={data.skills.enabledCategories > 0 ? <CheckIcon /> : <InfoIcon />} label="Skills">
						<span className="font-medium text-[var(--text)]">{data.skills.enabledSkills}</span> skills enabled across{" "}
						<span className="font-medium text-[var(--text)]">
							{data.skills.enabledCategories}/{data.skills.totalCategories}
						</span>{" "}
						categories
					</SummaryRow>
				)}

				{/* System Memory */}
				<SummaryRow
					icon={data.mem?.total && data.mem.total < LOW_MEMORY_THRESHOLD ? <WarnIcon /> : <CheckIcon />}
					label="System Memory"
				>
					{data.mem ? (
						<>
							Total: <span className="font-medium text-[var(--text)]">{formatMemBytes(data.mem.total)}</span> Available:{" "}
							<span className="font-medium text-[var(--text)]">{formatMemBytes(data.mem.available)}</span>
							{data.mem.total && data.mem.total < LOW_MEMORY_THRESHOLD ? (
								<div className="text-[var(--warn)] mt-1">
									Low memory detected. Consider upgrading to an instance with more RAM.
								</div>
							) : null}
						</>
					) : (
						<>Memory info unavailable</>
					)}
				</SummaryRow>

				{/* Sandbox */}
				<SummaryRow
					icon={data.sandbox?.backend && data.sandbox.backend !== "none" ? <CheckIcon /> : <InfoIcon />}
					label="Sandbox"
				>
					{data.sandbox?.backend && data.sandbox.backend !== "none" ? (
						<>
							Backend: <span className="font-medium text-[var(--text)]">{data.sandbox.backend}</span>
						</>
					) : (
						<>No container runtime detected</>
					)}
				</SummaryRow>

				{/* Version */}
				<SummaryRow icon={data.update?.available ? <WarnIcon /> : <CheckIcon />} label="Version">
					{data.update?.available ? (
						<>
							Update available:{" "}
							<a
								href={data.update.release_url || "#"}
								target="_blank"
								rel="noopener"
								className="text-[var(--accent)] underline font-medium"
							>
								{data.update.latest_version}
							</a>
						</>
					) : (
						<>You are running the latest version.</>
					)}
				</SummaryRow>

				{/* Tailscale (hidden if feature not compiled) */}
				{data.tailscale !== null ? (
					<SummaryRow
						icon={
							data.tailscale?.tailscale_up ? <CheckIcon /> : data.tailscale?.installed ? <WarnIcon /> : <InfoIcon />
						}
						label="Tailscale"
					>
						{data.tailscale?.tailscale_up ? (
							<>Connected</>
						) : data.tailscale?.installed ? (
							<>
								Installed but not connected &mdash;{" "}
								<a href="/settings/remote-access" className="text-[var(--accent)] underline">
									Configure in Settings
								</a>
							</>
						) : (
							<>Not installed. Install Tailscale for secure remote access.</>
						)}
					</SummaryRow>
				) : null}

				{/* Voice (hidden if not enabled) */}
				{data.voiceEnabled ? (
					<SummaryRow
						icon={
							data.voice && [...data.voice.tts, ...data.voice.stt].some((p) => p.enabled) ? <CheckIcon /> : <InfoIcon />
						}
						label="Voice"
					>
						{(() => {
							if (!data.voice) return <>Voice providers unavailable</>;
							const enabledStt = data.voice.stt.filter((p) => p.enabled).map((p) => p.name);
							const enabledTts = data.voice.tts.filter((p) => p.enabled).map((p) => p.name);
							if (enabledStt.length === 0 && enabledTts.length === 0) return <>No voice providers enabled</>;
							return (
								<div className="flex flex-col gap-0.5">
									{enabledStt.length > 0 ? (
										<div>
											STT: <span className="font-medium text-[var(--text)]">{enabledStt.join(", ")}</span>
										</div>
									) : null}
									{enabledTts.length > 0 ? (
										<div>
											TTS: <span className="font-medium text-[var(--text)]">{enabledTts.join(", ")}</span>
										</div>
									) : null}
								</div>
							);
						})()}
					</SummaryRow>
				) : null}
			</div>

			<div className="flex flex-wrap items-center gap-3 mt-1">
				<button type="button" className="provider-btn provider-btn-secondary" onClick={onBack}>
					{t("common:actions.back")}
				</button>
				<div className="flex-1" />
				<button type="button" className="provider-btn" onClick={onFinish}>
					{data.identity?.emoji || ""} {data.identity?.name || "Your agent"}, reporting for duty
				</button>
			</div>
		</div>
	);
}

// ── Main page component ─────────────────────────────────────

function OnboardingPage(): VNode {
	const [step, setStep] = useState(-1); // -1 = checking
	const [authNeeded, setAuthNeeded] = useState(false);
	const [authSkippable, setAuthSkippable] = useState(false);
	const [voiceAvailable] = useState(() => getGon("voice_enabled") === true);
	const headerRef = useRef<HTMLElement | null>(null);
	const navRef = useRef<HTMLElement | null>(null);
	const sessionsPanelRef = useRef<HTMLElement | null>(null);

	// Hide nav, header, and banners for standalone experience
	useEffect(() => {
		const header = document.querySelector("header") as HTMLElement | null;
		const nav = document.getElementById("navPanel");
		const sessions = document.getElementById("sessionsPanel");
		const burger = document.getElementById("burgerBtn");
		const toggle = document.getElementById("sessionsToggle");
		const authBanner = document.getElementById("authDisabledBanner");
		headerRef.current = header;
		navRef.current = nav;
		sessionsPanelRef.current = sessions;

		if (header) header.style.display = "none";
		if (nav) nav.style.display = "none";
		if (sessions) sessions.style.display = "none";
		if (burger) burger.style.display = "none";
		if (toggle) toggle.style.display = "none";
		if (authBanner) authBanner.style.display = "none";

		return () => {
			if (header) header.style.display = "";
			if (nav) nav.style.display = "";
			if (sessions) sessions.style.display = "";
			if (burger) burger.style.display = "";
			if (toggle) toggle.style.display = "";
		};
	}, []);

	// Check auth status to decide whether to show step 0
	useEffect(() => {
		fetch("/api/auth/status")
			.then((r) => (r.ok ? r.json() : null))
			.then((auth: { setup_required?: boolean; auth_disabled?: boolean; localhost_only?: boolean } | null) => {
				if (auth?.setup_required || (auth?.auth_disabled && !auth?.localhost_only)) {
					setAuthNeeded(true);
					setAuthSkippable(!auth.setup_required);
					setStep(0);
				} else {
					setAuthNeeded(false);
					ensureWsConnected();
					setStep(1);
				}
			})
			.catch(() => {
				setAuthNeeded(false);
				ensureWsConnected();
				setStep(1);
			});
	}, []);

	if (step === -1) {
		return (
			<div className="onboarding-card">
				<div className="text-sm text-[var(--muted)]">{t("common:status.loading")}</div>
			</div>
		);
	}

	// Build step list dynamically based on auth + voice + openclaw availability
	const openclawDetected = getGon("openclaw_detected") === true;
	const allLabels = [t("onboarding:steps.security")];
	if (openclawDetected) allLabels.push(t("onboarding:steps.import"));
	allLabels.push(t("onboarding:steps.llm"));
	if (voiceAvailable) allLabels.push(t("onboarding:steps.voice"));
	allLabels.push(
		t("onboarding:steps.skills"),
		t("onboarding:steps.remoteAccess"),
		t("onboarding:steps.channel"),
		t("onboarding:steps.identity"),
		t("onboarding:steps.summary"),
	);
	const steps = authNeeded ? allLabels : allLabels.slice(1);
	const stepIndex = authNeeded ? step : step - 1;

	// Compute dynamic step indices
	let nextIdx = 1;
	const importStep = openclawDetected ? nextIdx++ : -1;
	const llmStep = nextIdx++;
	const voiceStep = voiceAvailable ? nextIdx++ : -1;
	const skillsStep = nextIdx++;
	const remoteAccessStep = nextIdx++;
	const channelStep = nextIdx++;
	const identityStep = nextIdx++;
	const summaryStep = nextIdx;
	const lastStep = summaryStep;

	function goNext(): void {
		if (step === lastStep) window.location.assign(preferredChatPath());
		else setStep(step + 1);
	}

	function goFinish(): void {
		window.location.assign(preferredChatPath());
	}

	function goBack(): void {
		if (authNeeded) setStep(Math.max(0, step - 1));
		else setStep(Math.max(1, step - 1));
	}

	const startedAt = getGon("started_at") as number | null;
	const version = String(getGon("version") || "").trim();

	return (
		<div className="onboarding-card">
			<StepIndicator steps={steps} current={stepIndex} />
			<div className="mt-6">
				{step === 0 && <AuthStep onNext={goNext} skippable={authSkippable} />}
				{step === importStep && <OpenClawImportStep onNext={goNext} onBack={authNeeded ? goBack : null} />}
				{step === llmStep && <ProviderStep onNext={goNext} onBack={authNeeded || openclawDetected ? goBack : null} />}
				{step === voiceStep && <VoiceStep onNext={goNext} onBack={goBack} />}
				{step === skillsStep && <SkillsStep onNext={goNext} onBack={goBack} />}
				{step === remoteAccessStep && <RemoteAccessStep onNext={goNext} onBack={goBack} />}
				{step === channelStep && <ChannelStep onNext={goNext} onBack={goBack} />}
				{step === identityStep && <IdentityStep onNext={goNext} onBack={goBack} />}
				{step === summaryStep && <SummaryStep onBack={goBack} onFinish={goFinish} />}
			</div>
			{startedAt || version ? (
				<div className="text-xs text-[var(--muted)] text-center mt-4 pt-3 border-t border-[var(--border)]">
					{startedAt ? (
						<span>
							Server started <time data-epoch-ms={startedAt} />
						</span>
					) : null}
					{startedAt && version ? <span> {"\u00b7"} </span> : null}
					{version ? (
						<span>
							{t("onboarding:summary.versionLabel")} v{version}
						</span>
					) : null}
				</div>
			) : null}
		</div>
	);
}

// ── Page registration ───────────────────────────────────────

let containerRef: HTMLElement | null = null;

export function mountOnboarding(container: HTMLElement): void {
	containerRef = container;
	container.style.cssText =
		"display:flex;align-items:flex-start;justify-content:center;min-height:100vh;padding:max(0.75rem, env(safe-area-inset-top)) max(0.75rem, env(safe-area-inset-right)) max(0.75rem, env(safe-area-inset-bottom)) max(0.75rem, env(safe-area-inset-left));box-sizing:border-box;width:100%;max-width:100vw;overflow-x:hidden;overflow-y:auto;";
	render(<OnboardingPage />, container);
}

export function unmountOnboarding(): void {
	if (containerRef) render(null, containerRef);
	containerRef = null;
}

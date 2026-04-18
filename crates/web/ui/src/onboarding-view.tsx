// ── Onboarding wizard ──────────────────────────────────────
//
// Multi-step setup page shown to first-time users.
// Steps: Auth (conditional) → Identity → Provider → Voice (conditional) →
// Remote Access → Channel → Summary
// No new Rust code — all existing RPC methods and REST endpoints.

import type { VNode } from "preact";
import { render } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { get as getGon } from "./gon";
import { t } from "./i18n";

// ── Sub-module imports ──────────────────────────────────────
import { ensureWsConnected, preferredChatPath } from "./onboarding/shared";
import { AuthStep } from "./onboarding/steps/AuthStep";
import { IdentityStep } from "./onboarding/steps/IdentityStep";
import { ProviderStep } from "./onboarding/steps/ProviderStep";
import type { IdentityInfo } from "./onboarding/types";

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

// ── Placeholder steps ───────────────────────────────────────

function VoiceStep({ onNext, onBack }: { onNext: () => void; onBack: () => void }): VNode {
	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Voice (optional)</h2>
			<p className="text-xs text-[var(--muted)]">Voice configuration step — full TSX conversion pending.</p>
			<div className="flex flex-wrap items-center gap-3 mt-1">
				<button className="provider-btn provider-btn-secondary" onClick={onBack}>
					{t("common:actions.back")}
				</button>
				<button className="provider-btn" onClick={onNext}>
					{t("common:actions.continue")}
				</button>
			</div>
		</div>
	);
}

function RemoteAccessStep({ onNext, onBack }: { onNext: () => void; onBack: () => void }): VNode {
	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Remote Access</h2>
			<p className="text-xs text-[var(--muted)]">Remote access configuration step — full TSX conversion pending.</p>
			<div className="flex flex-wrap items-center gap-3 mt-1">
				<button className="provider-btn provider-btn-secondary" onClick={onBack}>
					{t("common:actions.back")}
				</button>
				<button className="provider-btn" onClick={onNext}>
					{t("common:actions.continue")}
				</button>
			</div>
		</div>
	);
}

function ChannelStep({ onNext, onBack }: { onNext: () => void; onBack: () => void }): VNode {
	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Connect a Channel</h2>
			<p className="text-xs text-[var(--muted)]">Channel configuration step — full TSX conversion pending.</p>
			<div className="flex flex-wrap items-center gap-3 mt-1">
				<button className="provider-btn provider-btn-secondary" onClick={onBack}>
					{t("common:actions.back")}
				</button>
				<button className="provider-btn" onClick={onNext}>
					{t("common:actions.continue")}
				</button>
				<button
					className="text-xs text-[var(--muted)] cursor-pointer bg-transparent border-none underline"
					onClick={onNext}
				>
					{t("common:actions.skip")}
				</button>
			</div>
		</div>
	);
}

function OpenClawImportStep({ onNext, onBack }: { onNext: () => void; onBack?: (() => void) | null }): VNode {
	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Import from OpenClaw</h2>
			<p className="text-xs text-[var(--muted)]">Import step — full TSX conversion pending.</p>
			<div className="flex flex-wrap items-center gap-3 mt-1">
				{onBack ? (
					<button className="provider-btn provider-btn-secondary" onClick={onBack}>
						Back
					</button>
				) : null}
				<button className="provider-btn" onClick={onNext}>
					Skip
				</button>
			</div>
		</div>
	);
}

function SummaryStep({ onBack, onFinish }: { onBack: () => void; onFinish: () => void }): VNode {
	const identity = (getGon("identity") as IdentityInfo) || {};
	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">{t("onboarding:summary.title")}</h2>
			<p className="text-xs text-[var(--muted)]">
				Overview of your configuration. You can change any of these later in Settings.
			</p>
			<div className="flex flex-wrap items-center gap-3 mt-1">
				<button className="provider-btn provider-btn-secondary" onClick={onBack}>
					{t("common:actions.back")}
				</button>
				<div className="flex-1" />
				<button className="provider-btn" onClick={onFinish}>
					{identity.emoji || ""} {identity.name || "Your agent"}, reporting for duty
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

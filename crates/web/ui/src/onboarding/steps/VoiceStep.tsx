// ── Voice step (TTS/STT configuration) ───────────────────────

import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import { get as getGon } from "../../gon";
import { t } from "../../i18n";
import { activeSessionKey } from "../../state";
import { fetchPhrase } from "../../tts-phrases";
import { targetValue } from "../../typed-events";
import {
	decodeBase64Safe,
	fetchVoiceProviders,
	saveVoiceKey,
	saveVoiceSettings,
	testTts,
	toggleVoiceProvider,
	transcribeAudio,
	VOICE_COUNTERPART_IDS,
} from "../../voice-utils";
import { ErrorPanel, ensureWsConnected } from "../shared";
import type { IdentityInfo } from "../types";

// ── Constants ───────────────────────────────────────────────

const WS_RETRY_LIMIT = 75;
const WS_RETRY_DELAY_MS = 200;

// ── Types ───────────────────────────────────────────────────

interface VoiceProvider {
	id: string;
	name: string;
	description?: string;
	category: string;
	available: boolean;
	enabled: boolean;
	keySource?: string;
	keyUrl?: string;
	keyUrlLabel?: string;
	keyPlaceholder?: string;
	hint?: string;
	capabilities?: { baseUrl?: boolean };
	settings?: { baseUrl?: string };
	[key: string]: unknown;
}

interface VoiceProviders {
	tts: VoiceProvider[];
	stt: VoiceProvider[];
}

interface VoiceTesting {
	id: string;
	type: string;
	phase: string;
}

interface VoiceTestResult {
	success?: boolean;
	text?: string | null;
	error?: string | null;
}

// ── OnboardingVoiceRow ──────────────────────────────────────

interface OnboardingVoiceRowProps {
	provider: VoiceProvider;
	type: string;
	configuring: string | null;
	apiKey: string;
	setApiKey: (v: string) => void;
	baseUrl: string;
	setBaseUrl: (v: string) => void;
	saving: boolean;
	error: string | null;
	onSaveKey: (e: Event) => void;
	onStartConfigure: (id: string) => void;
	onCancelConfigure: () => void;
	onTest: () => void;
	voiceTesting: VoiceTesting | null;
	voiceTestResult: VoiceTestResult | null;
}

function OnboardingVoiceRow({
	provider,
	type,
	configuring,
	apiKey,
	setApiKey,
	baseUrl,
	setBaseUrl,
	saving,
	error,
	onSaveKey,
	onStartConfigure,
	onCancelConfigure,
	onTest,
	voiceTesting,
	voiceTestResult,
}: OnboardingVoiceRowProps): VNode {
	const isConfiguring = configuring === provider.id;
	const keyInputRef = useRef<HTMLInputElement>(null);

	useEffect(() => {
		if (isConfiguring && keyInputRef.current) {
			keyInputRef.current.focus();
		}
	}, [isConfiguring]);

	const supportsBaseUrl = provider.capabilities?.baseUrl === true;
	const keySourceLabel =
		provider.keySource === "env" ? "(from env)" : provider.keySource === "llm_provider" ? "(from LLM provider)" : "";

	// Test button state
	const testState = voiceTesting?.id === provider.id && voiceTesting?.type === type ? voiceTesting : null;
	const showTestBtn = provider.available;
	let testBtnText = "Test";
	let testBtnDisabled = false;
	if (testState) {
		if (testState.phase === "recording") {
			testBtnText = "Stop";
		} else {
			testBtnText = "Testing\u2026";
			testBtnDisabled = true;
		}
	}

	return (
		<div className="rounded-md border border-[var(--border)] bg-[var(--surface)] p-3">
			<div className="flex items-center gap-3">
				<div className="flex-1 min-w-0 flex flex-col gap-0.5">
					<div className="flex items-center gap-2 flex-wrap">
						<span className="text-sm font-medium text-[var(--text-strong)]">{provider.name}</span>
						{provider.available ? (
							<span className="provider-item-badge configured">configured</span>
						) : (
							<span className="provider-item-badge needs-key">needs key</span>
						)}
						{keySourceLabel ? <span className="text-xs text-[var(--muted)]">{keySourceLabel}</span> : null}
					</div>
					{provider.description ? (
						<span className="text-xs text-[var(--muted)]">
							{provider.description}
							{!isConfiguring && provider.keyUrl ? (
								<>
									{" \u2014 "}get your key at{" "}
									<a href={provider.keyUrl} target="_blank" rel="noopener" className="text-[var(--accent)] underline">
										{provider.keyUrlLabel || provider.keyUrl}
									</a>
								</>
							) : null}
						</span>
					) : null}
				</div>
				<div className="shrink-0 flex items-center gap-2">
					{isConfiguring ? null : (
						<button
							type="button"
							className="provider-btn provider-btn-secondary provider-btn-sm"
							onClick={() => onStartConfigure(provider.id)}
						>
							Configure
						</button>
					)}
					{showTestBtn ? (
						<button
							type="button"
							className="provider-btn provider-btn-secondary provider-btn-sm"
							onClick={onTest}
							disabled={testBtnDisabled}
							title={type === "tts" ? "Test voice output" : "Test voice input"}
						>
							{testBtnText}
						</button>
					) : null}
				</div>
			</div>
			{testState?.phase === "recording" ? (
				<div className="voice-recording-hint mt-2">
					<span className="voice-recording-dot" />
					<span>Speak now, then click Stop when finished</span>
				</div>
			) : null}
			{testState?.phase === "transcribing" ? (
				<span className="text-xs text-[var(--muted)] mt-1 block">Transcribing&hellip;</span>
			) : null}
			{testState?.phase === "testing" && type === "tts" ? (
				<span className="text-xs text-[var(--muted)] mt-1 block">Playing audio&hellip;</span>
			) : null}
			{voiceTestResult?.text ? (
				<div className="voice-transcription-result mt-2">
					<span className="voice-transcription-label">Transcribed:</span>
					<span className="voice-transcription-text">"{voiceTestResult.text}"</span>
				</div>
			) : null}
			{voiceTestResult?.success === true ? (
				<div className="voice-success-result mt-2">
					<span className="icon icon-md icon-check-circle" />
					<span>Audio played successfully</span>
				</div>
			) : null}
			{voiceTestResult?.error ? (
				<div className="voice-error-result">
					<span className="icon icon-md icon-x-circle" />
					<span>{voiceTestResult.error}</span>
				</div>
			) : null}
			{isConfiguring ? (
				<form onSubmit={onSaveKey} className="flex flex-col gap-2 mt-3 border-t border-[var(--border)] pt-3">
					<div>
						<label className="text-xs text-[var(--muted)] mb-1 block">API Key</label>
						<input
							type="password"
							className="provider-key-input w-full"
							ref={keyInputRef}
							value={apiKey}
							onInput={(e) => setApiKey(targetValue(e))}
							placeholder={provider.keyPlaceholder || "API key"}
						/>
					</div>
					{supportsBaseUrl ? (
						<div>
							<label className="text-xs text-[var(--muted)] mb-1 block">Base URL</label>
							<input
								type="text"
								className="provider-key-input w-full"
								data-field="baseUrl"
								value={baseUrl}
								onInput={(e) => setBaseUrl(targetValue(e))}
								placeholder="http://localhost:8000/v1"
							/>
							<div className="text-xs text-[var(--muted)] mt-1">
								Use this for a local or OpenAI-compatible server. Leave the API key blank if the endpoint does not
								require one.
							</div>
						</div>
					) : null}
					{provider.keyUrl ? (
						<div className="text-xs text-[var(--muted)]">
							Get your key at{" "}
							<a href={provider.keyUrl} target="_blank" rel="noopener" className="text-[var(--accent)] underline">
								{provider.keyUrlLabel || provider.keyUrl}
							</a>
						</div>
					) : null}
					{provider.hint ? <div className="text-xs text-[var(--accent)]">{provider.hint}</div> : null}
					{error ? <ErrorPanel message={error} /> : null}
					<div className="flex items-center gap-2 mt-1">
						<button type="submit" className="provider-btn provider-btn-sm" disabled={saving}>
							{saving ? "Saving\u2026" : "Save"}
						</button>
						<button
							type="button"
							className="provider-btn provider-btn-secondary provider-btn-sm"
							onClick={onCancelConfigure}
						>
							Cancel
						</button>
					</div>
				</form>
			) : null}
		</div>
	);
}

// ── VoiceStep ───────────────────────────────────────────────

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: voice step manages provider list, key config forms, TTS playback, and STT mic recording
export function VoiceStep({ onNext, onBack }: { onNext: () => void; onBack: () => void }): VNode {
	const [loading, setLoading] = useState(true);
	const [allProviders, setAllProviders] = useState<VoiceProviders>({ tts: [], stt: [] });
	const [configuring, setConfiguring] = useState<string | null>(null);
	const [apiKey, setApiKey] = useState("");
	const [baseUrl, setBaseUrl] = useState("");
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [voiceTesting, setVoiceTesting] = useState<VoiceTesting | null>(null);
	const [voiceTestResults, setVoiceTestResults] = useState<Record<string, VoiceTestResult>>({});
	const [activeRecorder, setActiveRecorder] = useState<MediaRecorder | null>(null);
	const [enableSaving, setEnableSaving] = useState(false);

	function fetchProviders(): Promise<unknown> {
		return (fetchVoiceProviders() as Promise<{ ok?: boolean; payload?: VoiceProviders }>).then((res) => {
			if (res?.ok) {
				setAllProviders(res.payload || { tts: [], stt: [] });
			}
			return res;
		});
	}

	useEffect(() => {
		let cancelled = false;
		let attempts = 0;

		function load(): void {
			if (cancelled) return;
			(
				fetchVoiceProviders() as Promise<{
					ok?: boolean;
					payload?: VoiceProviders;
					error?: { code?: string; message?: string };
				}>
			).then((res) => {
				if (cancelled) return;
				if (res?.ok) {
					setAllProviders(res.payload || { tts: [], stt: [] });
					setLoading(false);
					return;
				}
				if (
					(res?.error?.code === "UNAVAILABLE" || res?.error?.message === "WebSocket not connected") &&
					attempts < WS_RETRY_LIMIT
				) {
					attempts += 1;
					ensureWsConnected();
					window.setTimeout(load, WS_RETRY_DELAY_MS);
					return;
				}
				// Voice not compiled -> skip
				onNext();
			});
		}

		load();
		return () => {
			cancelled = true;
		};
	}, []);

	// Cloud providers only (filter out local for onboarding)
	const cloudStt = allProviders.stt.filter((p) => p.category === "cloud");
	const cloudTts = allProviders.tts.filter((p) => p.category === "cloud");

	// Auto-detected: available via LLM provider key, not yet enabled.
	const autoDetected = [...allProviders.stt, ...allProviders.tts].filter(
		(p) => p.available && p.keySource === "llm_provider" && !p.enabled && p.category === "cloud",
	);
	const hasAutoDetected = autoDetected.length > 0;

	function enableAutoDetected(): void {
		setEnableSaving(true);
		setError(null);
		const firstStt = allProviders.stt.find((p) => p.available && p.keySource === "llm_provider" && !p.enabled);
		const firstTts = allProviders.tts.find((p) => p.available && p.keySource === "llm_provider" && !p.enabled);
		const toggles: Promise<unknown>[] = [];
		if (firstStt) toggles.push(toggleVoiceProvider(firstStt.id, true, "stt"));
		if (firstTts) toggles.push(toggleVoiceProvider(firstTts.id, true, "tts"));
		if (toggles.length === 0) {
			setEnableSaving(false);
			return;
		}
		Promise.all(toggles).then((results) => {
			setEnableSaving(false);
			const failed = (results as Array<{ ok?: boolean; error?: { message?: string } }>).find((r) => !r?.ok);
			if (failed) {
				setError(failed?.error?.message || "Failed to enable voice provider");
				return;
			}
			fetchProviders();
		});
	}

	function onStartConfigure(providerId: string): void {
		const provider = [...allProviders.stt, ...allProviders.tts].find((candidate) => candidate.id === providerId);
		setConfiguring(providerId);
		setApiKey("");
		setBaseUrl(provider?.settings?.baseUrl || "");
		setError(null);
	}

	function onCancelConfigure(): void {
		setConfiguring(null);
		setApiKey("");
		setBaseUrl("");
		setError(null);
	}

	function onSaveKey(e: Event): void {
		e.preventDefault();
		const provider = [...allProviders.stt, ...allProviders.tts].find((candidate) => candidate.id === configuring);
		const trimmedApiKey = apiKey.trim();
		const trimmedBaseUrl = baseUrl.trim();
		const hadBaseUrl = typeof provider?.settings?.baseUrl === "string" && provider.settings.baseUrl.trim().length > 0;
		const shouldSaveBaseUrl = provider?.capabilities?.baseUrl === true && (trimmedBaseUrl.length > 0 || hadBaseUrl);
		if (!(trimmedApiKey || shouldSaveBaseUrl)) {
			setError("API key or base URL is required.");
			return;
		}
		setError(null);
		setSaving(true);
		const providerId = configuring as string;
		const req = trimmedApiKey
			? saveVoiceKey(providerId, trimmedApiKey, {
					baseUrl: shouldSaveBaseUrl ? trimmedBaseUrl : undefined,
				})
			: saveVoiceSettings(providerId, shouldSaveBaseUrl ? { baseUrl: trimmedBaseUrl } : undefined);
		(req as Promise<{ ok?: boolean; error?: { message?: string } }>).then(async (res) => {
			if (res?.ok) {
				// Auto-enable in onboarding: toggle on for each type this provider appears in.
				const counterId = VOICE_COUNTERPART_IDS[providerId];
				const toggles: Promise<unknown>[] = [];
				const sttMatch =
					allProviders.stt.find((p) => p.id === providerId) ||
					(counterId && allProviders.stt.find((p) => p.id === counterId));
				const ttsMatch =
					allProviders.tts.find((p) => p.id === providerId) ||
					(counterId && allProviders.tts.find((p) => p.id === counterId));
				if (sttMatch) {
					toggles.push(toggleVoiceProvider(sttMatch.id, true, "stt"));
				}
				if (ttsMatch) {
					toggles.push(toggleVoiceProvider(ttsMatch.id, true, "tts"));
				}
				await Promise.all(toggles);
				setSaving(false);
				setConfiguring(null);
				setApiKey("");
				setBaseUrl("");
				fetchProviders();
			} else {
				setSaving(false);
				setError(res?.error?.message || "Failed to save");
			}
		});
	}

	// Stop active STT recording
	function stopSttRecording(): void {
		if (activeRecorder) {
			activeRecorder.stop();
		}
	}

	// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: test function handles TTS playback and STT mic recording flows
	async function testVoiceProvider(providerId: string, type: string): Promise<void> {
		// If already recording for this provider, stop it
		if (voiceTesting?.id === providerId && voiceTesting?.type === "stt" && voiceTesting?.phase === "recording") {
			stopSttRecording();
			return;
		}

		setError(null);
		setVoiceTesting({ id: providerId, type, phase: "testing" });

		// Auto-enable the provider if it's available but not yet enabled
		const prov = (type === "stt" ? allProviders.stt : allProviders.tts).find((p) => p.id === providerId);
		if (prov?.available && !prov?.enabled) {
			const toggleRes = (await toggleVoiceProvider(providerId, true, type)) as {
				ok?: boolean;
				error?: { message?: string };
			};
			if (!toggleRes?.ok) {
				setVoiceTestResults((prev) => ({
					...prev,
					[providerId]: {
						success: false,
						error: toggleRes?.error?.message || "Failed to enable provider",
					},
				}));
				setVoiceTesting(null);
				return;
			}
			// ElevenLabs/Google share API keys - enable the counterpart too.
			const counterType = type === "stt" ? "tts" : "stt";
			const counterList = type === "stt" ? allProviders.tts : allProviders.stt;
			const counterId = VOICE_COUNTERPART_IDS[providerId] || providerId;
			const counterProv = counterList.find((p) => p.id === counterId);
			if (counterProv?.available && !counterProv?.enabled) {
				await toggleVoiceProvider(counterId, true, counterType);
			}
			// Refresh provider list in background
			fetchProviders();
		}

		if (type === "tts") {
			try {
				const identity = getGon("identity") as IdentityInfo | null;
				const user = identity?.user_name || "friend";
				const bot = identity?.name || "Moltis";
				const ttsText = await fetchPhrase("onboarding", user, bot);
				const res = (await testTts(ttsText, providerId)) as {
					ok?: boolean;
					payload?: { audio?: string; mimeType?: string; content_type?: string; format?: string };
					error?: { message?: string };
				};
				if (res?.ok && res.payload?.audio) {
					const bytes = decodeBase64Safe(res.payload.audio);
					const audioMime = res.payload.mimeType || res.payload.content_type || "audio/mpeg";
					const blob = new Blob([bytes.buffer as ArrayBuffer], { type: audioMime });
					const url = URL.createObjectURL(blob);
					const audio = new Audio(url);
					audio.onerror = (e) => {
						console.error("[TTS] audio element error:", audio.error?.message || e);
						URL.revokeObjectURL(url);
					};
					audio.onended = () => URL.revokeObjectURL(url);
					audio.play().catch((e) => console.error("[TTS] play() failed:", e));
					setVoiceTestResults((prev) => ({
						...prev,
						[providerId]: { success: true, error: null },
					}));
				} else {
					setVoiceTestResults((prev) => ({
						...prev,
						[providerId]: {
							success: false,
							error: res?.error?.message || "TTS test failed",
						},
					}));
				}
			} catch (err) {
				setVoiceTestResults((prev) => ({
					...prev,
					[providerId]: {
						success: false,
						error: (err as Error).message || "TTS test failed",
					},
				}));
			}
			setVoiceTesting(null);
		} else {
			// STT: record then transcribe
			try {
				const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
				const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
					? "audio/webm;codecs=opus"
					: "audio/webm";
				const mediaRecorder = new MediaRecorder(stream, { mimeType });
				const audioChunks: Blob[] = [];

				mediaRecorder.ondataavailable = (e) => {
					if (e.data.size > 0) audioChunks.push(e.data);
				};

				mediaRecorder.start();
				setActiveRecorder(mediaRecorder);
				setVoiceTesting({ id: providerId, type, phase: "recording" });

				mediaRecorder.onstop = async () => {
					setActiveRecorder(null);
					for (const track of stream.getTracks()) track.stop();
					setVoiceTesting({ id: providerId, type, phase: "transcribing" });

					const audioBlob = new Blob(audioChunks, {
						type: mediaRecorder.mimeType || mimeType,
					});

					try {
						const resp = await transcribeAudio(activeSessionKey, providerId, audioBlob);
						if (resp.ok) {
							const sttRes = (await resp.json()) as {
								ok?: boolean;
								transcription?: { text?: string };
								transcriptionError?: string;
								error?: string;
							};

							if (sttRes.ok && typeof sttRes.transcription?.text === "string") {
								const transcriptText = sttRes.transcription.text.trim();
								setVoiceTestResults((prev) => ({
									...prev,
									[providerId]: {
										text: transcriptText || null,
										error: transcriptText ? null : "No speech detected",
									},
								}));
							} else {
								setVoiceTestResults((prev) => ({
									...prev,
									[providerId]: {
										text: null,
										error: sttRes.transcriptionError || sttRes.error || "STT test failed",
									},
								}));
							}
						} else {
							const errBody = await resp.text();
							console.error("[STT] upload failed: status=%d body=%s", resp.status, errBody);
							let errMsg = "STT test failed";
							try {
								errMsg = (JSON.parse(errBody) as { error?: string })?.error || errMsg;
							} catch (_e) {
								// not JSON
							}
							setVoiceTestResults((prev) => ({
								...prev,
								[providerId]: {
									text: null,
									error: `${errMsg} (HTTP ${resp.status})`,
								},
							}));
						}
					} catch (fetchErr) {
						setVoiceTestResults((prev) => ({
							...prev,
							[providerId]: {
								text: null,
								error: (fetchErr as Error).message || "STT test failed",
							},
						}));
					}
					setVoiceTesting(null);
				};
			} catch (err) {
				const domErr = err as DOMException;
				if (domErr.name === "NotAllowedError") {
					setError("Microphone permission denied");
				} else if (domErr.name === "NotFoundError") {
					setError("No microphone found");
				} else {
					setError(domErr.message || "STT test failed");
				}
				setVoiceTesting(null);
			}
		}
	}

	// ── Render ────────────────────────────────────────────────

	if (loading) {
		return <div className="text-sm text-[var(--muted)]">Checking voice providers&hellip;</div>;
	}

	return (
		<div className="flex flex-col gap-4">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Voice (optional)</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed">
				Enable voice input (speech-to-text) and output (text-to-speech) for your agent. You can configure this later in
				Settings.
			</p>

			{hasAutoDetected ? (
				<div className="rounded-md border border-[var(--border)] bg-[var(--surface2)] p-3 flex flex-col gap-2">
					<div className="text-xs text-[var(--muted)]">Auto-detected from your LLM provider</div>
					<div className="flex flex-wrap gap-2">
						{autoDetected.map((p) => (
							<span key={p.id} className="provider-item-badge configured">
								{p.name}
							</span>
						))}
					</div>
					<button
						type="button"
						className="provider-btn self-start"
						disabled={enableSaving}
						onClick={enableAutoDetected}
					>
						{enableSaving ? "Enabling\u2026" : "Enable voice"}
					</button>
				</div>
			) : null}

			{cloudStt.length > 0 ? (
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)] mb-2">Speech-to-Text</h3>
					<div className="flex flex-col gap-2">
						{cloudStt.map((prov) => (
							<OnboardingVoiceRow
								key={prov.id}
								provider={prov}
								type="stt"
								configuring={configuring}
								apiKey={apiKey}
								setApiKey={setApiKey}
								baseUrl={baseUrl}
								setBaseUrl={setBaseUrl}
								saving={saving}
								error={configuring === prov.id ? error : null}
								onSaveKey={onSaveKey}
								onStartConfigure={onStartConfigure}
								onCancelConfigure={onCancelConfigure}
								onTest={() => testVoiceProvider(prov.id, "stt")}
								voiceTesting={voiceTesting}
								voiceTestResult={voiceTestResults[prov.id] || null}
							/>
						))}
					</div>
				</div>
			) : null}

			{cloudTts.length > 0 ? (
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)] mb-2">Text-to-Speech</h3>
					<div className="flex flex-col gap-2">
						{cloudTts.map((prov) => (
							<OnboardingVoiceRow
								key={prov.id}
								provider={prov}
								type="tts"
								configuring={configuring}
								apiKey={apiKey}
								setApiKey={setApiKey}
								baseUrl={baseUrl}
								setBaseUrl={setBaseUrl}
								saving={saving}
								error={configuring === prov.id ? error : null}
								onSaveKey={onSaveKey}
								onStartConfigure={onStartConfigure}
								onCancelConfigure={onCancelConfigure}
								onTest={() => testVoiceProvider(prov.id, "tts")}
								voiceTesting={voiceTesting}
								voiceTestResult={voiceTestResults[prov.id] || null}
							/>
						))}
					</div>
				</div>
			) : null}

			{error && !configuring ? <ErrorPanel message={error} /> : null}
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
					{t("common:actions.skip")}
				</button>
			</div>
		</div>
	);
}

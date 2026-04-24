// ── Voice section ────────────────────────────────────────────

import { signal } from "@preact/signals";
import type { VNode } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import * as gon from "../../gon";
import { sendRpc } from "../../helpers";
import { connected } from "../../signals";
import * as S from "../../state";
import { fetchPhrase } from "../../tts-phrases";
import { targetChecked, targetValue } from "../../typed-events";
import { Modal } from "../../ui";
import { getPttKey, getVadSensitivity, setPttKey, setVadSensitivity } from "../../voice-input";
import {
	decodeBase64Safe,
	fetchVoiceProviders,
	saveVoiceKey,
	saveVoiceSettings,
	testTts,
	toggleVoiceProvider,
	transcribeAudio,
} from "../../voice-utils";
import type { RpcResponse } from "./_shared";
import { rerender } from "./_shared";
import { cloneHidden } from "./RemoteAccessSection";

// Voice section signals
const voiceShowAddModal = signal(false);
const voiceSelectedProvider = signal<string | null>(null);
const voiceSelectedProviderData = signal<VoiceProviderData | null>(null);

interface VoiceProviderData {
	id: string;
	name: string;
	description?: string;
	type?: string;
	category?: string;
	available?: boolean;
	enabled?: boolean;
	keySource?: string;
	settingsSummary?: string;
	binaryPath?: string;
	statusMessage?: string;
	keyPlaceholder?: string;
	keyUrl?: string;
	keyUrlLabel?: string;
	hint?: string;
	capabilities?: { baseUrl?: boolean };
	settings?: { baseUrl?: string; voiceId?: string; voice?: string; model?: string; languageCode?: string };
}

interface VoiceProviders {
	tts: VoiceProviderData[];
	stt: VoiceProviderData[];
}

interface VoiceTesting {
	id: string;
	type: string;
	phase: string;
}

interface VoiceTestResult {
	text?: string | null;
	success?: boolean;
	error?: string | null;
}

interface VoxtralRequirements {
	os?: string;
	arch?: string;
	compatible?: boolean;
	reasons?: string[];
	python?: { available?: boolean; version?: string };
	cuda?: { available?: boolean; gpu_name?: string; memory_mb?: number };
}

interface PttKeyPickerProps {
	pttListening: boolean;
	setPttListening: (v: boolean) => void;
	pttKeyValue: string;
	setPttKeyValue: (v: string) => void;
}

function PttKeyPicker({ pttListening, setPttListening, pttKeyValue, setPttKeyValue }: PttKeyPickerProps): VNode {
	const handlerRef = useRef<((ev: KeyboardEvent) => void) | null>(null);

	useEffect(() => {
		return () => {
			if (handlerRef.current) {
				document.removeEventListener("keydown", handlerRef.current, true);
				handlerRef.current = null;
			}
		};
	}, []);

	return (
		<button
			type="button"
			className="provider-key-input"
			style={{ minWidth: "120px", textAlign: "center", cursor: "pointer" }}
			onClick={() => {
				if (pttListening) return;
				setPttListening(true);
				const handler = (ev: KeyboardEvent): void => {
					ev.preventDefault();
					ev.stopPropagation();
					setPttKeyValue(ev.key);
					setPttKey(ev.key);
					setPttListening(false);
					document.removeEventListener("keydown", handler, true);
					handlerRef.current = null;
					rerender();
				};
				handlerRef.current = handler;
				document.addEventListener("keydown", handler, true);
				rerender();
			}}
		>
			{pttListening ? "Press any key..." : pttKeyValue}
		</button>
	);
}

export function VoiceSection(): VNode {
	const [allProviders, setAllProviders] = useState<VoiceProviders>({ tts: [], stt: [] });
	const [voiceLoading, setVoiceLoading] = useState(true);
	const [voxtralReqs, setVoxtralReqs] = useState<VoxtralRequirements | null>(null);
	const [savingProvider, setSavingProvider] = useState<string | null>(null);
	const [voiceMsg, setVoiceMsg] = useState<string | null>(null);
	const [voiceErr, setVoiceErr] = useState<string | null>(null);
	const [voiceTesting, setVoiceTesting] = useState<VoiceTesting | null>(null);
	const [activeRecorder, setActiveRecorder] = useState<MediaRecorder | null>(null);
	const [voiceTestResults, setVoiceTestResults] = useState<Record<string, VoiceTestResult>>({});

	// PTT key configuration
	const [pttKeyValue, setPttKeyValue] = useState(getPttKey());
	const [pttListening, setPttListening] = useState(false);

	// VAD sensitivity
	const [vadSens, setVadSens] = useState(getVadSensitivity());

	function fetchVoiceStatus(options?: { silent?: boolean }): void {
		if (!options?.silent) {
			setVoiceLoading(true);
			rerender();
		}
		Promise.all([fetchVoiceProviders(), sendRpc("voice.config.voxtral_requirements", {})])
			.then(([providers, voxtral]) => {
				const provRes = providers as RpcResponse;
				const voxtralRes = voxtral as RpcResponse;
				if (provRes?.ok) setAllProviders((provRes.payload as VoiceProviders) || { tts: [], stt: [] });
				if (voxtralRes?.ok) setVoxtralReqs(voxtralRes.payload as VoxtralRequirements);
				if (!options?.silent) setVoiceLoading(false);
				rerender();
			})
			.catch(() => {
				if (!options?.silent) setVoiceLoading(false);
				rerender();
			});
	}

	useEffect(() => {
		if (connected.value) fetchVoiceStatus();
	}, [connected.value]);

	function onToggleProvider(provider: VoiceProviderData, enabled: boolean, providerType: string): void {
		setVoiceErr(null);
		setVoiceMsg(null);
		setSavingProvider(provider.id);
		rerender();

		toggleVoiceProvider(provider.id, enabled, providerType)
			.then((r: unknown) => {
				const res = r as RpcResponse;
				setSavingProvider(null);
				if (res?.ok) {
					setVoiceMsg(`${provider.name} ${enabled ? "enabled" : "disabled"}.`);
					setTimeout(() => {
						setVoiceMsg(null);
						rerender();
					}, 2000);
					fetchVoiceStatus({ silent: true });
				} else {
					setVoiceErr((res?.error as { message?: string })?.message || "Failed to toggle provider");
				}
				rerender();
			})
			.catch((err: Error) => {
				setSavingProvider(null);
				setVoiceErr(err.message);
				rerender();
			});
	}

	function onConfigureProvider(providerId: string, providerData: VoiceProviderData): void {
		voiceSelectedProvider.value = providerId;
		voiceSelectedProviderData.value = providerData || null;
		voiceShowAddModal.value = true;
	}

	function getUnconfiguredProviders(): VoiceProviderData[] {
		return [...allProviders.stt, ...allProviders.tts].filter((p) => !p.available);
	}

	function stopSttRecording(): void {
		if (activeRecorder) {
			activeRecorder.stop();
		}
	}

	function humanizeMicError(err: { name?: string; message?: string }): string {
		if (err.name === "OverconstrainedError" || (err.message && /constraint/i.test(err.message))) {
			return "No compatible microphone found. Check your audio input device.";
		}
		if (err.name === "NotFoundError" || err.name === "NotAllowedError") {
			return "Microphone access denied or no microphone found. Check browser permissions.";
		}
		if (err.name === "NotReadableError") {
			return "Microphone is in use by another application.";
		}
		return err.message || "STT test failed";
	}

	async function testVoiceProvider(providerId: string, type: string): Promise<void> {
		if (voiceTesting?.id === providerId && voiceTesting?.type === "stt" && voiceTesting?.phase === "recording") {
			stopSttRecording();
			return;
		}

		setVoiceErr(null);
		setVoiceMsg(null);
		setVoiceTesting({ id: providerId, type, phase: "testing" });
		rerender();

		if (type === "tts") {
			try {
				const id = gon.get("identity") as { user_name?: string; name?: string } | undefined;
				const user = id?.user_name || "friend";
				const bot = id?.name || "Moltis";
				const ttsText = await fetchPhrase("settings", user, bot);
				const res = (await testTts(ttsText, providerId)) as RpcResponse;
				if (res?.ok && (res.payload as { audio?: string })?.audio) {
					const payload = res.payload as { audio: string; mimeType?: string; content_type?: string; format?: string };
					const bytes = decodeBase64Safe(payload.audio);
					const audioMime = payload.mimeType || payload.content_type || "audio/mpeg";
					const blob = new Blob([bytes as BlobPart], { type: audioMime });
					const url = URL.createObjectURL(blob);
					const audio = new Audio(url);
					audio.onerror = (e) => {
						console.error("[TTS] audio element error:", audio.error?.message || e);
						URL.revokeObjectURL(url);
					};
					audio.onended = () => URL.revokeObjectURL(url);
					audio.play().catch((e: Error) => console.error("[TTS] play() failed:", e));
					setVoiceTestResults((prev) => ({
						...prev,
						[providerId]: { success: true, error: null },
					}));
				} else {
					setVoiceTestResults((prev) => ({
						...prev,
						[providerId]: { success: false, error: (res?.error as { message?: string })?.message || "TTS test failed" },
					}));
				}
			} catch (err) {
				setVoiceTestResults((prev) => ({
					...prev,
					[providerId]: { success: false, error: (err as Error).message || "TTS test failed" },
				}));
			}
			setVoiceTesting(null);
		} else {
			try {
				const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
				const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
					? "audio/webm;codecs=opus"
					: "audio/webm";
				const mediaRecorder = new MediaRecorder(stream, { mimeType });
				const audioChunks: Blob[] = [];

				mediaRecorder.ondataavailable = (e: BlobEvent) => {
					if (e.data.size > 0) audioChunks.push(e.data);
				};

				mediaRecorder.start();
				setActiveRecorder(mediaRecorder);
				setVoiceTesting({ id: providerId, type, phase: "recording" });
				rerender();

				mediaRecorder.onstop = async () => {
					setActiveRecorder(null);
					for (const track of stream.getTracks()) track.stop();
					setVoiceTesting({ id: providerId, type, phase: "transcribing" });
					rerender();

					const audioBlob = new Blob(audioChunks, { type: mediaRecorder.mimeType || mimeType });

					try {
						const resp = await transcribeAudio(S.activeSessionKey, providerId, audioBlob);
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
								[providerId]: { text: null, error: `${errMsg} (HTTP ${resp.status})` },
							}));
						}
					} catch (fetchErr) {
						setVoiceTestResults((prev) => ({
							...prev,
							[providerId]: { text: null, error: (fetchErr as Error).message || "STT test failed" },
						}));
					}
					setVoiceTesting(null);
					rerender();
				};
			} catch (err) {
				setVoiceErr(humanizeMicError(err as { name?: string; message?: string }));
				setVoiceTesting(null);
			}
		}
		rerender();
	}

	if (voiceLoading || !connected.value) {
		return (
			<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
				<h2 className="text-lg font-medium text-[var(--text-strong)]">Voice</h2>
				<div className="text-xs text-[var(--muted)]">{connected.value ? "Loading\u2026" : "Connecting\u2026"}</div>
			</div>
		);
	}

	return (
		<div className="flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto">
			<h2 className="text-lg font-medium text-[var(--text-strong)]">Voice</h2>
			<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ maxWidth: "600px", margin: 0 }}>
				Configure text-to-speech (TTS) and speech-to-text (STT) providers. STT lets you use the microphone button in
				chat to record voice input. TTS lets you hear responses as audio.
			</p>

			{voiceMsg ? <div className="text-xs text-[var(--accent)]">{voiceMsg}</div> : null}
			{voiceErr ? <div className="text-xs text-[var(--error)]">{voiceErr}</div> : null}

			<div style={{ maxWidth: "700px", display: "flex", flexDirection: "column", gap: "24px" }}>
				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)] mb-3">Speech-to-Text (Voice Input)</h3>
					<div className="flex flex-col gap-2">
						{allProviders.stt.map((prov) => {
							const meta = prov;
							const testState = voiceTesting?.id === prov.id && voiceTesting?.type === "stt" ? voiceTesting : null;
							const testResult = voiceTestResults[prov.id] || null;
							return (
								<VoiceProviderRow
									key={prov.id}
									provider={prov}
									meta={meta}
									type="stt"
									saving={savingProvider === prov.id}
									testState={testState}
									testResult={testResult}
									onToggle={(enabled: boolean) => onToggleProvider(prov, enabled, "stt")}
									onConfigure={() => onConfigureProvider(prov.id, prov)}
									onTest={() => testVoiceProvider(prov.id, "stt")}
								/>
							);
						})}
					</div>
				</div>

				<div>
					<h3 className="text-sm font-medium text-[var(--text-strong)] mb-3">Text-to-Speech (Audio Responses)</h3>
					<div className="flex flex-col gap-2">
						{allProviders.tts.map((prov) => {
							const meta = prov;
							const testState = voiceTesting?.id === prov.id && voiceTesting?.type === "tts" ? voiceTesting : null;
							const testResult = voiceTestResults[prov.id] || null;
							return (
								<VoiceProviderRow
									key={prov.id}
									provider={prov}
									meta={meta}
									type="tts"
									saving={savingProvider === prov.id}
									testState={testState}
									testResult={testResult}
									onToggle={(enabled: boolean) => onToggleProvider(prov, enabled, "tts")}
									onConfigure={() => onConfigureProvider(prov.id, prov)}
									onTest={() => testVoiceProvider(prov.id, "tts")}
								/>
							);
						})}
					</div>
				</div>
			</div>

			{/* Push-to-Talk Configuration */}
			<div style={{ maxWidth: "700px", display: "flex", flexDirection: "column", gap: "12px" }}>
				<h3 className="text-sm font-medium text-[var(--text-strong)]">Push-to-Talk</h3>
				<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
					Hold a keyboard key to record voice input. Release to send. Function keys (F1–F24) work even when focused in
					an input field.
				</p>
				<div className="flex items-center gap-3">
					<span className="text-xs text-[var(--muted)]">PTT Key:</span>
					<PttKeyPicker
						pttListening={pttListening}
						setPttListening={setPttListening}
						pttKeyValue={pttKeyValue}
						setPttKeyValue={setPttKeyValue}
					/>
				</div>
			</div>

			{/* VAD Sensitivity */}
			<div style={{ maxWidth: "700px", display: "flex", flexDirection: "column", gap: "12px" }}>
				<h3 className="text-sm font-medium text-[var(--text-strong)]">Conversation Mode (VAD)</h3>
				<p className="text-xs text-[var(--muted)] leading-relaxed" style={{ margin: 0 }}>
					Adjust how sensitive the voice activity detection is. Higher values pick up softer speech but may trigger on
					background noise.
				</p>
				<div className="flex items-center gap-3">
					<span className="text-xs text-[var(--muted)]" style={{ minWidth: "80px" }}>
						Sensitivity:
					</span>
					<input
						type="range"
						min="0"
						max="100"
						step="5"
						value={vadSens}
						style={{ flex: 1, maxWidth: "200px", accentColor: "var(--accent)" }}
						onInput={(e) => {
							const val = parseInt(targetValue(e), 10);
							setVadSens(val);
							setVadSensitivity(val);
							rerender();
						}}
					/>
					<span className="text-xs text-[var(--muted)]" style={{ minWidth: "35px", textAlign: "right" }}>
						{vadSens}%
					</span>
				</div>
			</div>

			<AddVoiceProviderModal
				unconfiguredProviders={getUnconfiguredProviders()}
				voxtralReqs={voxtralReqs}
				onSaved={() => {
					fetchVoiceStatus();
					voiceShowAddModal.value = false;
					voiceSelectedProvider.value = null;
					voiceSelectedProviderData.value = null;
				}}
			/>
		</div>
	);
}

// Individual provider row with enable toggle

interface VoiceProviderRowProps {
	provider: VoiceProviderData;
	meta: VoiceProviderData;
	type: string;
	saving: boolean;
	testState: VoiceTesting | null;
	testResult: VoiceTestResult | null;
	onToggle: (enabled: boolean) => void;
	onConfigure: () => void;
	onTest: () => void;
}

function VoiceProviderRow({
	provider,
	meta,
	type,
	saving,
	testState,
	testResult,
	onToggle,
	onConfigure,
	onTest,
}: VoiceProviderRowProps): VNode {
	const canEnable = provider.available;
	const keySourceLabel =
		provider.keySource === "env" ? "(from env)" : provider.keySource === "llm_provider" ? "(from LLM provider)" : "";
	const showTestBtn = canEnable && provider.enabled;

	let buttonText = "Test";
	let buttonDisabled = false;
	if (testState) {
		if (testState.phase === "recording") {
			buttonText = "Stop";
		} else if (testState.phase === "transcribing") {
			buttonText = "Testing\u2026";
			buttonDisabled = true;
		} else {
			buttonText = "Testing\u2026";
			buttonDisabled = true;
		}
	}

	return (
		<div
			className="provider-card"
			style={{ padding: "10px 14px", borderRadius: "8px", display: "flex", alignItems: "center", gap: "12px" }}
		>
			<div style={{ flex: 1, display: "flex", flexDirection: "column", gap: "2px" }}>
				<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
					<span className="text-sm text-[var(--text-strong)]">{meta.name}</span>
					{provider.category === "local" ? <span className="provider-item-badge">local</span> : null}
					{keySourceLabel ? <span className="text-xs text-[var(--muted)]">{keySourceLabel}</span> : null}
				</div>
				<span className="text-xs text-[var(--muted)]">{meta.description}</span>
				{provider.settingsSummary ? (
					<span className="text-xs text-[var(--muted)]">Voice: {provider.settingsSummary}</span>
				) : null}
				{provider.binaryPath ? (
					<span className="text-xs text-[var(--muted)]">Found at: {provider.binaryPath}</span>
				) : null}
				{!canEnable && provider.statusMessage ? (
					<span className="text-xs text-[var(--muted)]">{provider.statusMessage}</span>
				) : null}
				{testState?.phase === "recording" ? (
					<div className="voice-recording-hint">
						<span className="voice-recording-dot" />
						<span>Speak now, then click Stop when finished</span>
					</div>
				) : null}
				{testState?.phase === "transcribing" ? (
					<span className="text-xs text-[var(--muted)]">Transcribing...</span>
				) : null}
				{testState?.phase === "testing" && type === "tts" ? (
					<span className="text-xs text-[var(--muted)]">Playing audio...</span>
				) : null}
				{testResult?.text ? (
					<div className="voice-transcription-result">
						<span className="voice-transcription-label">Transcribed:</span>
						<span className="voice-transcription-text">"{testResult.text}"</span>
					</div>
				) : null}
				{testResult?.success === true ? (
					<div className="voice-success-result">
						<span className="icon icon-md icon-check-circle" />
						<span>Audio played successfully</span>
					</div>
				) : null}
				{testResult?.error ? (
					<div className="voice-error-result">
						<span className="icon icon-md icon-x-circle" />
						<span>{testResult.error}</span>
					</div>
				) : null}
			</div>
			<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
				<button className="provider-btn provider-btn-secondary provider-btn-sm" onClick={onConfigure}>
					Configure
				</button>
				{showTestBtn ? (
					<button
						className="provider-btn provider-btn-secondary provider-btn-sm"
						onClick={onTest}
						disabled={buttonDisabled}
						title={type === "tts" ? "Test voice output" : "Test voice input"}
					>
						{buttonText}
					</button>
				) : null}
				{canEnable ? (
					<label className="toggle-switch">
						<input
							type="checkbox"
							checked={provider.enabled}
							disabled={saving}
							onChange={(e: Event) => onToggle(targetChecked(e))}
						/>
						<span className="toggle-slider" />
					</label>
				) : provider.category === "local" ? (
					<span className="text-xs text-[var(--muted)]">Install required</span>
				) : null}
			</div>
		</div>
	);
}

// Local provider instructions component (uses hidden HTML elements)

interface LocalProviderInstructionsProps {
	providerId: string;
	voxtralReqs: VoxtralRequirements | null;
}

function LocalProviderInstructions({ providerId, voxtralReqs }: LocalProviderInstructionsProps): VNode {
	const ref = useRef<HTMLDivElement>(null);

	useEffect(() => {
		const container = ref.current;
		if (!container) return;
		while (container.firstChild) container.removeChild(container.firstChild);

		const templateId: Record<string, string> = {
			"whisper-cli": "voice-whisper-cli-instructions",
			"sherpa-onnx": "voice-sherpa-onnx-instructions",
			piper: "voice-piper-instructions",
			coqui: "voice-coqui-instructions",
			"voxtral-local": "voice-voxtral-instructions",
		};

		const tplId = templateId[providerId];
		if (!tplId) return;

		const el = cloneHidden(tplId);
		if (!el) return;

		if (providerId === "voxtral-local" && el.querySelector("[data-voxtral-requirements]")) {
			const reqsContainer = el.querySelector("[data-voxtral-requirements]") as HTMLElement;
			if (voxtralReqs) {
				let detected = `${voxtralReqs.os}/${voxtralReqs.arch}`;
				if (voxtralReqs.python?.available) detected += `, Python ${voxtralReqs.python.version}`;
				else detected += ", no Python";
				if (voxtralReqs.cuda?.available) {
					detected += `, ${voxtralReqs.cuda.gpu_name || "NVIDIA GPU"} (${Math.round((voxtralReqs.cuda.memory_mb || 0) / 1024)}GB)`;
				} else detected += ", no CUDA GPU";

				const reqEl = cloneHidden(
					voxtralReqs.compatible ? "voice-voxtral-requirements-ok" : "voice-voxtral-requirements-fail",
				);
				if (reqEl) {
					const detectedEl = reqEl.querySelector("[data-voxtral-detected]") as HTMLElement;
					if (detectedEl) detectedEl.textContent = detected;
					if (!voxtralReqs.compatible && voxtralReqs.reasons?.length) {
						const ul = reqEl.querySelector("[data-voxtral-reasons]") as HTMLElement;
						for (const r of voxtralReqs.reasons) {
							const li = document.createElement("li");
							li.style.margin = "2px 0";
							li.textContent = r;
							ul.appendChild(li);
						}
					}
					reqsContainer.appendChild(reqEl);
				}
			} else {
				const loadingEl = document.createElement("div");
				loadingEl.className = "text-xs text-[var(--muted)] mb-3";
				loadingEl.textContent = "Checking system requirements\u2026";
				reqsContainer.appendChild(loadingEl);
			}
		}

		container.appendChild(el);
	}, [providerId, voxtralReqs]);

	return <div ref={ref} />;
}

// Add Voice Provider Modal

interface AddVoiceProviderModalProps {
	unconfiguredProviders: VoiceProviderData[];
	voxtralReqs: VoxtralRequirements | null;
	onSaved: () => void;
}

interface ElevenlabsCatalog {
	voices: { id: string; name: string }[];
	models: { id: string; name: string }[];
	warning: string | null;
}

function AddVoiceProviderModal({ unconfiguredProviders, voxtralReqs, onSaved }: AddVoiceProviderModalProps): VNode {
	const [apiKey, setApiKey] = useState("");
	const [baseUrlValue, setBaseUrlValue] = useState("");
	const [voiceValue, setVoiceValue] = useState("");
	const [modelValue, setModelValue] = useState("");
	const [languageCodeValue, setLanguageCodeValue] = useState("");
	const [elevenlabsCatalog, setElevenlabsCatalog] = useState<ElevenlabsCatalog>({
		voices: [],
		models: [],
		warning: null,
	});
	const [elevenlabsCatalogLoading, setElevenlabsCatalogLoading] = useState(false);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState("");

	const selectedProvider = voiceSelectedProvider.value;
	const providerMeta = selectedProvider
		? unconfiguredProviders.find((p) => p.id === selectedProvider) || voiceSelectedProviderData.value
		: null;
	const isElevenLabsProvider = selectedProvider === "elevenlabs" || selectedProvider === "elevenlabs-stt";
	const supportsTtsVoiceSettings = providerMeta?.type === "tts";
	const supportsBaseUrl = providerMeta?.capabilities?.baseUrl === true;

	function onClose(): void {
		voiceShowAddModal.value = false;
		voiceSelectedProvider.value = null;
		voiceSelectedProviderData.value = null;
		setApiKey("");
		setBaseUrlValue("");
		setVoiceValue("");
		setModelValue("");
		setLanguageCodeValue("");
		setError("");
	}

	function onSaveKey(): void {
		const hasApiKey = apiKey.trim().length > 0;
		const trimmedBaseUrl = baseUrlValue.trim();
		const hadBaseUrl =
			typeof providerMeta?.settings?.baseUrl === "string" && providerMeta.settings.baseUrl.trim().length > 0;
		const hasBaseUrl = supportsBaseUrl && (trimmedBaseUrl.length > 0 || hadBaseUrl);
		const hasSettings =
			(supportsTtsVoiceSettings && (voiceValue.trim() || modelValue.trim() || languageCodeValue.trim())) || hasBaseUrl;
		if (!(hasApiKey || hasSettings)) {
			setError("Provide an API key, base URL, or at least one provider setting.");
			return;
		}
		setError("");
		setSaving(true);

		const voiceOpts = {
			baseUrl: hasBaseUrl ? trimmedBaseUrl : undefined,
			voice: supportsTtsVoiceSettings ? voiceValue.trim() || undefined : undefined,
			model: supportsTtsVoiceSettings ? modelValue.trim() || undefined : undefined,
			languageCode: supportsTtsVoiceSettings ? languageCodeValue.trim() || undefined : undefined,
		};
		const req = hasApiKey
			? saveVoiceKey(selectedProvider as string, apiKey.trim(), voiceOpts)
			: saveVoiceSettings(selectedProvider as string, voiceOpts);
		req
			.then((r: unknown) => {
				const res = r as RpcResponse;
				setSaving(false);
				if (res?.ok) {
					setApiKey("");
					onSaved();
				} else {
					setError((res?.error as { message?: string })?.message || "Failed to save key");
				}
			})
			.catch((err: Error) => {
				setSaving(false);
				setError(err.message);
			});
	}

	function onSelectProvider(providerId: string): void {
		voiceSelectedProvider.value = providerId;
		voiceSelectedProviderData.value = null;
		setApiKey("");
		setBaseUrlValue("");
		setVoiceValue("");
		setModelValue("");
		setLanguageCodeValue("");
		setError("");
	}

	useEffect(() => {
		const settings = voiceSelectedProviderData.value?.settings;
		if (!settings) return;
		setBaseUrlValue(settings.baseUrl || "");
		setVoiceValue(settings.voiceId || settings.voice || "");
		setModelValue(settings.model || "");
		setLanguageCodeValue(settings.languageCode || "");
	}, [selectedProvider, voiceSelectedProviderData.value]);

	useEffect(() => {
		if (!isElevenLabsProvider) {
			setElevenlabsCatalog({ voices: [], models: [], warning: null });
			return;
		}
		setElevenlabsCatalogLoading(true);
		sendRpc("voice.elevenlabs.catalog", {})
			.then((res: RpcResponse) => {
				if (res?.ok) {
					const payload = res.payload as {
						voices?: { id: string; name: string }[];
						models?: { id: string; name: string }[];
						warning?: string;
					};
					setElevenlabsCatalog({
						voices: payload?.voices || [],
						models: payload?.models || [],
						warning: payload?.warning || null,
					});
				}
			})
			.catch(() => {
				setElevenlabsCatalog({ voices: [], models: [], warning: "Failed to fetch ElevenLabs voice catalog." });
			})
			.finally(() => {
				setElevenlabsCatalogLoading(false);
				rerender();
			});
	}, [selectedProvider, isElevenLabsProvider]);

	const sttCloud = unconfiguredProviders.filter((p) => p.type === "stt" && p.category === "cloud");
	const sttLocal = unconfiguredProviders.filter((p) => p.type === "stt" && p.category === "local");
	const ttsProviders = unconfiguredProviders.filter((p) => p.type === "tts");

	if (selectedProvider && providerMeta) {
		if (providerMeta.category === "cloud") {
			return (
				<Modal show={voiceShowAddModal.value} onClose={onClose} title={`Add ${providerMeta.name}`}>
					<div className="channel-form">
						<div className="text-sm text-[var(--text-strong)]">{providerMeta.name}</div>
						<div className="mb-3 text-xs text-[var(--muted)]">{providerMeta.description}</div>

						<label className="text-xs text-[var(--muted)]">API Key</label>
						<input
							type="password"
							className="provider-key-input w-full"
							value={apiKey}
							onInput={(e: Event) => setApiKey(targetValue(e))}
							placeholder={providerMeta.keyPlaceholder || "Leave blank to keep existing key"}
						/>
						{providerMeta.keyUrl ? (
							<div className="text-xs text-[var(--muted)]">
								Get your API key at{" "}
								<a
									href={providerMeta.keyUrl}
									target="_blank"
									rel="noopener"
									className="hover:underline text-[var(--accent)]"
								>
									{providerMeta.keyUrlLabel}
								</a>
							</div>
						) : null}

						{supportsBaseUrl ? (
							<div className="mt-2 flex flex-col gap-2">
								<label className="text-xs text-[var(--muted)]">Base URL</label>
								<input
									type="text"
									className="provider-key-input w-full"
									data-field="baseUrl"
									value={baseUrlValue}
									onInput={(e: Event) => setBaseUrlValue(targetValue(e))}
									placeholder="http://localhost:8000/v1"
								/>
								<div className="text-xs text-[var(--muted)]">
									Use this for a local or OpenAI-compatible server. Leave the API key blank if your endpoint does not
									require one.
								</div>
							</div>
						) : null}

						{supportsTtsVoiceSettings ? (
							<div className="flex flex-col gap-2">
								<label className="text-xs text-[var(--muted)]">Voice</label>
								{isElevenLabsProvider && elevenlabsCatalogLoading ? (
									<div className="text-xs text-[var(--muted)]">Loading ElevenLabs voices...</div>
								) : null}
								{isElevenLabsProvider && elevenlabsCatalog.warning ? (
									<div className="text-xs text-[var(--muted)]">{elevenlabsCatalog.warning}</div>
								) : null}
								{isElevenLabsProvider && elevenlabsCatalog.voices.length > 0 ? (
									<select className="provider-key-input w-full" onChange={(e: Event) => setVoiceValue(targetValue(e))}>
										<option value="">Pick a voice from your account...</option>
										{elevenlabsCatalog.voices.map((v) => (
											<option key={v.id} value={v.id}>
												{v.name} ({v.id})
											</option>
										))}
									</select>
								) : null}
								<input
									type="text"
									className="provider-key-input w-full"
									value={voiceValue}
									onInput={(e: Event) => setVoiceValue(targetValue(e))}
									list={isElevenLabsProvider ? "elevenlabs-voice-options" : undefined}
									placeholder="voice id / name (optional)"
								/>
								{isElevenLabsProvider ? (
									<datalist id="elevenlabs-voice-options">
										{elevenlabsCatalog.voices.map((v) => (
											<option key={v.id} value={v.id}>
												{v.name}
											</option>
										))}
									</datalist>
								) : null}

								<label className="text-xs text-[var(--muted)]">Model</label>
								{isElevenLabsProvider && elevenlabsCatalog.models.length > 0 ? (
									<select className="provider-key-input w-full" onChange={(e: Event) => setModelValue(targetValue(e))}>
										<option value="">Pick a model...</option>
										{elevenlabsCatalog.models.map((m) => (
											<option key={m.id} value={m.id}>
												{m.name} ({m.id})
											</option>
										))}
									</select>
								) : null}
								<input
									type="text"
									className="provider-key-input w-full"
									value={modelValue}
									onInput={(e: Event) => setModelValue(targetValue(e))}
									list={isElevenLabsProvider ? "elevenlabs-model-options" : undefined}
									placeholder="model (optional)"
								/>
								{isElevenLabsProvider ? (
									<datalist id="elevenlabs-model-options">
										{elevenlabsCatalog.models.map((m) => (
											<option key={m.id} value={m.id}>
												{m.name}
											</option>
										))}
									</datalist>
								) : null}

								{selectedProvider === "google" || selectedProvider === "google-tts" ? (
									<div className="flex flex-col gap-2">
										<label className="text-xs text-[var(--muted)]">Language Code</label>
										<input
											type="text"
											className="provider-key-input w-full"
											value={languageCodeValue}
											onInput={(e: Event) => setLanguageCodeValue(targetValue(e))}
											placeholder="en-US (optional)"
										/>
									</div>
								) : null}
							</div>
						) : null}

						{providerMeta.hint ? (
							<div
								className="text-xs text-[var(--muted)]"
								style={{
									marginTop: "8px",
									padding: "8px",
									background: "var(--surface-alt)",
									borderRadius: "4px",
									fontStyle: "italic",
								}}
							>
								{providerMeta.hint}
							</div>
						) : null}

						{error ? (
							<div className="text-xs" style={{ color: "var(--error)" }}>
								{error}
							</div>
						) : null}

						<div style={{ display: "flex", gap: "8px", marginTop: "8px" }}>
							<button
								className="provider-btn provider-btn-secondary"
								onClick={() => {
									voiceSelectedProvider.value = null;
									setApiKey("");
									setError("");
								}}
							>
								Back
							</button>
							<button className="provider-btn" disabled={saving} onClick={onSaveKey}>
								{saving ? "Saving\u2026" : "Save"}
							</button>
						</div>
					</div>
				</Modal>
			);
		}

		if (providerMeta.category === "local") {
			return (
				<Modal show={voiceShowAddModal.value} onClose={onClose} title={`Add ${providerMeta.name}`}>
					<div className="channel-form">
						<div className="text-sm text-[var(--text-strong)]">{providerMeta.name}</div>
						<div className="text-xs text-[var(--muted)]" style={{ marginBottom: "12px" }}>
							{providerMeta.description}
						</div>
						<LocalProviderInstructions providerId={selectedProvider} voxtralReqs={voxtralReqs} />
						<div style={{ display: "flex", gap: "8px", marginTop: "12px" }}>
							<button
								className="provider-btn provider-btn-secondary"
								onClick={() => {
									voiceSelectedProvider.value = null;
								}}
							>
								Back
							</button>
						</div>
					</div>
				</Modal>
			);
		}
	}

	const providerButton = (p: VoiceProviderData) => (
		<button
			key={p.id}
			className="provider-card"
			style={{
				padding: "10px 12px",
				borderRadius: "6px",
				cursor: "pointer",
				textAlign: "left",
				border: "1px solid var(--border)",
				background: "var(--surface)",
			}}
			onClick={() => onSelectProvider(p.id)}
		>
			<div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
				<div style={{ flex: 1 }}>
					<div className="text-sm text-[var(--text-strong)]">{p.name}</div>
					<div className="text-xs text-[var(--muted)]">{p.description}</div>
				</div>
				<span className="icon icon-chevron-right" style={{ color: "var(--muted)" }} />
			</div>
		</button>
	);

	return (
		<Modal show={voiceShowAddModal.value} onClose={onClose} title="Add Voice Provider">
			<div className="channel-form" style={{ gap: "16px" }}>
				{sttCloud.length > 0 ? (
					<div>
						<h4
							className="text-xs font-medium text-[var(--muted)]"
							style={{ margin: "0 0 8px", textTransform: "uppercase", letterSpacing: "0.5px" }}
						>
							Speech-to-Text (Cloud)
						</h4>
						<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>{sttCloud.map(providerButton)}</div>
					</div>
				) : null}

				{sttLocal.length > 0 ? (
					<div>
						<h4
							className="text-xs font-medium text-[var(--muted)]"
							style={{ margin: "0 0 8px", textTransform: "uppercase", letterSpacing: "0.5px" }}
						>
							Speech-to-Text (Local)
						</h4>
						<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>{sttLocal.map(providerButton)}</div>
					</div>
				) : null}

				{ttsProviders.length > 0 ? (
					<div>
						<h4
							className="text-xs font-medium text-[var(--muted)]"
							style={{ margin: "0 0 8px", textTransform: "uppercase", letterSpacing: "0.5px" }}
						>
							Text-to-Speech
						</h4>
						<div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
							{ttsProviders.map(providerButton)}
						</div>
					</div>
				) : null}

				{unconfiguredProviders.length === 0 ? (
					<div className="text-sm text-[var(--muted)]" style={{ textAlign: "center", padding: "20px 0" }}>
						All available providers are already configured.
					</div>
				) : null}
			</div>
		</Modal>
	);
}

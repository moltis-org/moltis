// ── Provider modal — thin re-export barrel ──────────────────
//
// All implementation lives in ./providers/ sub-modules. This file
// re-exports the public API so existing import paths continue to work.

export { openModelSelectorForProvider, showApiKeyForm, showOAuthFlow } from "./providers/auth-flow";
export { showCustomProviderForm } from "./providers/custom-provider";
export {
	getModelState,
	initModelLifecycleTracking,
	showLocalModelFlow,
	showModelDownloadProgress,
} from "./providers/local-models";
export { closeProviderModal, getProviderModal, openProviderModal } from "./providers/shared";
export type {
	AddCustomPayload,
	BackendInfo,
	HfSearchResult,
	LocalLlmDownloadPayload,
	LocalLlmLifecyclePayload,
	LocalModelInfo,
	ModelEntry,
	ModelSelectorWrapper,
	ModelStateEntry,
	ModelsData,
	ProbeResult,
	ProviderInfo,
	ProviderModalElements,
	SystemInfo,
	ValidationEventPayload,
	ValidationProgressState,
	ValidationProgressUpdate,
} from "./providers/types";

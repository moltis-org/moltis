import { init as initI18n } from "./i18n";
import * as S from "./state";
import { initTheme, injectMarkdownStyles } from "./theme";
import "./time-format";

// Expose state module for E2E test WS mocking via shims.
window.__moltis_state = S;

initTheme();
injectMarkdownStyles();
const i18nReady = initI18n().catch((err: unknown) => {
	console.warn("[i18n] onboarding init failed", err);
});

const root = document.getElementById("onboardingRoot");
if (!root) {
	throw new Error("onboarding root element not found");
}

function showFallbackError(container: HTMLElement): void {
	// Static error content — no user input involved, safe to set directly.
	const card = document.createElement("div");
	card.className = "onboarding-card";
	const alert = document.createElement("div");
	alert.setAttribute("role", "alert");
	alert.className = "alert-error-text whitespace-pre-line";
	const errorSpan = document.createElement("span");
	errorSpan.className = "text-[var(--error)] font-medium";
	errorSpan.textContent = "Error:";
	alert.appendChild(errorSpan);
	alert.appendChild(
		document.createTextNode(
			" Failed to load onboarding UI. Please refresh. If this persists, update your browser and disable conflicting extensions.",
		),
	);
	card.appendChild(alert);
	container.textContent = "";
	container.appendChild(card);
}

i18nReady.finally(() => {
	import("./onboarding-view")
		.then((mod) => {
			if (typeof mod.mountOnboarding !== "function") {
				throw new Error("onboarding module did not export mountOnboarding");
			}
			mod.mountOnboarding(root);
		})
		.catch((err: unknown) => {
			console.error("[onboarding] failed to load onboarding module", err);
			showFallbackError(root);
		});
});

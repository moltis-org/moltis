import { init as initI18n } from "./i18n.js";
import { initTheme, injectMarkdownStyles } from "./theme.js";
import "./time-format.js";

initTheme();
injectMarkdownStyles();
var i18nReady = initI18n().catch((err) => {
	console.warn("[i18n] onboarding init failed", err);
});

var root = document.getElementById("onboardingRoot");
if (!root) {
	throw new Error("onboarding root element not found");
}

i18nReady.finally(() => {
	import("./onboarding-view.js")
		.then((mod) => {
			if (typeof mod.mountOnboarding !== "function") {
				throw new Error("onboarding module did not export mountOnboarding");
			}
			mod.mountOnboarding(root);
		})
		.catch((err) => {
			console.error("[onboarding] failed to load onboarding module", err);
			root.innerHTML =
				'<div class="onboarding-card"><div role="alert" class="alert-error-text whitespace-pre-line"><span class="text-[var(--error)] font-medium">Error:</span> Failed to load onboarding UI. Please refresh. If this persists, update your browser and disable conflicting extensions.</div></div>';
		});
});

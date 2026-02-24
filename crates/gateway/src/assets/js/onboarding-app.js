import { init as initI18n } from "./i18n.js";
import { mountOnboarding } from "./onboarding-view.js";
import { initTheme, injectMarkdownStyles } from "./theme.js";

initTheme();
injectMarkdownStyles();
var i18nReady = initI18n().catch((err) => {
	console.warn("[i18n] onboarding init failed", err);
});

var root = document.getElementById("onboardingRoot");
if (root) {
	i18nReady.finally(() => {
		mountOnboarding(root);
	});
}

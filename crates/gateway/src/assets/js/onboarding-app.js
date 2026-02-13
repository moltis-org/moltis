import { mountOnboarding } from "./onboarding-view.js";
import { initTheme, injectMarkdownStyles } from "./theme.js";

initTheme();
injectMarkdownStyles();

var root = document.getElementById("onboardingRoot");
if (root) {
	mountOnboarding(root);
}

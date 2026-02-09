import { mountOnboarding } from "./onboarding-view.js";
import { initTheme, injectMarkdownStyles } from "./theme.js";
import { connectWs } from "./ws-connect.js";

initTheme();
injectMarkdownStyles();
connectWs({ backoff: { factor: 2, max: 10000 } });

var root = document.getElementById("onboardingRoot");
if (root) {
	mountOnboarding(root);
}

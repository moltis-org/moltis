const __vite__mapDeps=(i,m=__vite__mapDeps,d=(m.f||(m.f=["chunks/onboarding-view.js","chunks/jsxRuntime.module.js","chunks/theme.js","chunks/webauthn-helpers.js"])))=>i.map(i=>d[i]);
import { bN as initTheme, bO as injectMarkdownStyles, bP as init, _ as __vitePreload } from "./chunks/theme.js";
import "./chunks/time-format.js";
initTheme();
injectMarkdownStyles();
const i18nReady = init().catch((err) => {
  console.warn("[i18n] onboarding init failed", err);
});
const root = document.getElementById("onboardingRoot");
if (!root) {
  throw new Error("onboarding root element not found");
}
function showFallbackError(container) {
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
      " Failed to load onboarding UI. Please refresh. If this persists, update your browser and disable conflicting extensions."
    )
  );
  card.appendChild(alert);
  container.textContent = "";
  container.appendChild(card);
}
i18nReady.finally(() => {
  __vitePreload(() => import("./chunks/onboarding-view.js"), true ? __vite__mapDeps([0,1,2,3]) : void 0).then((mod) => {
    if (typeof mod.mountOnboarding !== "function") {
      throw new Error("onboarding module did not export mountOnboarding");
    }
    mod.mountOnboarding(root);
  }).catch((err) => {
    console.error("[onboarding] failed to load onboarding module", err);
    showFallbackError(root);
  });
});

// ── Onboarding page (stub) ──────────────────────────────────

// @ts-expect-error -- onboarding-view not yet migrated to TS
import { mountOnboarding, unmountOnboarding } from "../onboarding-view";
import { registerPage } from "../router";
import { routes } from "../routes";

registerPage(routes.onboarding!, mountOnboarding, unmountOnboarding);

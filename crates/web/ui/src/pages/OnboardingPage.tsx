// ── Onboarding page (stub) ──────────────────────────────────

import { mountOnboarding, unmountOnboarding } from "../onboarding-view";
import { registerPage } from "../router";
import { routes } from "../routes";

registerPage(routes.onboarding!, mountOnboarding, unmountOnboarding);

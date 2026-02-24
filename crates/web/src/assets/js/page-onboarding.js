import { mountOnboarding, unmountOnboarding } from "./onboarding-view.js";
import { registerPage } from "./router.js";
import { routes } from "./routes.js";

registerPage(routes.onboarding, mountOnboarding, unmountOnboarding);

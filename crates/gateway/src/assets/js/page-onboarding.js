import { mountOnboarding, unmountOnboarding } from "./onboarding-view.js";
import { registerPage } from "./router.js";

registerPage("/onboarding", mountOnboarding, unmountOnboarding);

// E2E shim — proxies to bundled i18n module
const M = (window.__moltis_modules || {})["i18n"] || {};
export default M;
export const locale = M.locale;
export const t = (...args) => M.t?.(...args);
export const hasTranslation = (...args) => M.hasTranslation?.(...args);
export const init = (...args) => M.init?.(...args);
export const setLocale = (...args) => M.setLocale?.(...args);
export const translateStaticElements = (...args) => M.translateStaticElements?.(...args);

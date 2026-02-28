// ── i18n core module ────────────────────────────────────────
//
// Single entry point for all translations. Uses i18next under the hood.
// English is loaded eagerly; other locales are lazy-loaded on demand.
//
// Exports:
//   locale       – reactive Preact signal for current locale
//   t(key, opts) – global translation function for imperative DOM code
//   useTranslation(ns) – Preact hook that subscribes to locale signal
//   setLocale(lng)     – switch language, persist to localStorage
//   init()             – initialise i18next, load English bundles
//   translateStaticElements(root) – translate static data-i18n elements/attrs

import { signal, useComputed } from "@preact/signals";
import i18next from "i18next";

var STORAGE_KEY = "moltis-locale";
var initPromise = null;
var SUPPORTED_LOCALES = new Set(["en", "fr", "zh"]);
export var supportedLocales = Object.freeze(["en", "fr", "zh"]);

function normalizeLocaleTag(value) {
	if (!value) return "en";
	var tag = String(value).trim().replace("_", "-");
	if (!tag) return "en";
	var idx = tag.indexOf("-");
	if (idx !== -1) {
		tag = tag.slice(0, idx);
	}
	return tag.toLowerCase();
}

function resolveSupportedLocale(value) {
	var normalized = normalizeLocaleTag(value);
	if (SUPPORTED_LOCALES.has(normalized)) return normalized;
	return "en";
}

export function getPreferredLocale() {
	var stored = localStorage.getItem(STORAGE_KEY);
	if (stored) {
		return resolveSupportedLocale(stored);
	}
	return resolveSupportedLocale(navigator.language || "en");
}

// ── Locale signal ───────────────────────────────────────────
// Reactive — Preact components that read locale.value will re-render
// when the language changes.
export var locale = signal(getPreferredLocale());

// ── Namespace registry ──────────────────────────────────────
// Maps namespace name → lazy loader. English bundles are loaded eagerly
// at init(); other locales load on demand via setLocale().
var namespaces = {
	common: (lng) => import(`./locales/${lng}/common.js`),
	errors: (lng) => import(`./locales/${lng}/errors.js`),
	settings: (lng) => import(`./locales/${lng}/settings.js`),
	providers: (lng) => import(`./locales/${lng}/providers.js`),
	chat: (lng) => import(`./locales/${lng}/chat.js`),
	onboarding: (lng) => import(`./locales/${lng}/onboarding.js`),
	login: (lng) => import(`./locales/${lng}/login.js`),
	crons: (lng) => import(`./locales/${lng}/crons.js`),
	mcp: (lng) => import(`./locales/${lng}/mcp.js`),
	skills: (lng) => import(`./locales/${lng}/skills.js`),
	channels: (lng) => import(`./locales/${lng}/channels.js`),
	hooks: (lng) => import(`./locales/${lng}/hooks.js`),
	projects: (lng) => import(`./locales/${lng}/projects.js`),
	images: (lng) => import(`./locales/${lng}/images.js`),
	metrics: (lng) => import(`./locales/${lng}/metrics.js`),
	pwa: (lng) => import(`./locales/${lng}/pwa.js`),
	sessions: (lng) => import(`./locales/${lng}/sessions.js`),
	logs: (lng) => import(`./locales/${lng}/logs.js`),
};

// ── Load all namespace bundles for a language ───────────────
function loadLanguage(lng) {
	var keys = Object.keys(namespaces);
	var promises = keys.map((ns) =>
		namespaces[ns](lng)
			.then((mod) => {
				i18next.addResourceBundle(lng, ns, mod.default || mod, true, true);
			})
			.catch((err) => {
				console.warn(`[i18n] failed to load ${lng}/${ns}`, err);
			}),
	);
	return Promise.all(promises);
}

function applyDocumentLocale(lng) {
	if (typeof document === "undefined" || !document.documentElement) return;
	document.documentElement.lang = lng || "en";
}

// ── Public API ──────────────────────────────────────────────

/**
 * Initialise i18next with English bundles.
 * Call once at app startup before any t() calls.
 */
export function init() {
	if (initPromise) return initPromise;
	initPromise = i18next
		.init({
			lng: locale.value,
			fallbackLng: "en",
			defaultNS: "common",
			ns: Object.keys(namespaces),
			interpolation: {
				escapeValue: false, // Preact / DOM handles escaping
			},
			resources: {},
		})
		.then(() => loadLanguage("en"))
		.then(() => {
			// If the detected locale isn't English, load it too.
			if (locale.value !== "en") {
				return loadLanguage(locale.value);
			}
		})
		.then(() => {
			// Ensure i18next is set to the detected locale after bundles load.
			if (i18next.language !== locale.value) {
				return i18next.changeLanguage(locale.value);
			}
		})
		.then(() => {
			applyDocumentLocale(locale.value);
		});
	return initPromise;
}

/**
 * Global translation function for imperative DOM code.
 *   t("common:actions.save")
 *   t("errors:usageLimitReached.title", { planType: "free" })
 *
 * Namespace can be specified with colon prefix or via the `ns` option.
 */
export function t(key, opts) {
	return i18next.t(key, opts);
}

export function hasTranslation(key, opts) {
	return i18next.exists(key, opts);
}

/**
 * Preact hook — returns { t, locale } that triggers re-render on locale change.
 *
 * Usage:
 *   var { t } = useTranslation("settings");
 *   return html`<h2>${t("identity.title")}</h2>`;
 */
export function useTranslation(ns) {
	// Reading locale.value inside useComputed creates a reactive dependency.
	// When locale changes, the computed re-evaluates and Preact re-renders.
	var bound = useComputed(() => {
		var _lng = locale.value; // subscribe to signal
		void _lng;
		return {
			t: (key, opts) => {
				var options = opts ? Object.assign({ ns: ns }, opts) : { ns: ns };
				return i18next.t(key, options);
			},
			locale: locale.value,
		};
	});
	return bound.value;
}

/**
 * Switch the active locale. Lazy-loads the bundle if needed, persists
 * to localStorage, and triggers a re-render of all subscribed components.
 */
export function setLocale(lng) {
	var normalized = resolveSupportedLocale(lng);
	localStorage.setItem(STORAGE_KEY, normalized);
	return loadLanguage(normalized).then(() =>
		i18next.changeLanguage(normalized).then(() => {
			locale.value = normalized;
			applyDocumentLocale(normalized);
			// Re-translate any static data-i18n elements.
			translateStaticElements(document.documentElement);
			window.dispatchEvent(new CustomEvent("moltis:locale-changed", { detail: { locale: normalized } }));
		}),
	);
}

function applyStaticTranslation(el, key, attrName) {
	if (!key) return;
	var translated = i18next.t(key);
	// Only update if i18next returned a real translation (not the key itself).
	if (!(translated && translated !== key)) return;
	if (attrName) {
		el.setAttribute(attrName, translated);
		return;
	}
	el.textContent = translated;
}

/**
 * Translate static data-i18n markers under `root`.
 *
 * Supported markers:
 * - `data-i18n="ns:key"`: set element textContent
 * - `data-i18n-title="ns:key"`: set `title` attribute
 * - `data-i18n-placeholder="ns:key"`: set `placeholder` attribute
 * - `data-i18n-aria-label="ns:key"`: set `aria-label` attribute
 */
export function translateStaticElements(root) {
	if (!root) return;
	var elements = root.querySelectorAll("[data-i18n],[data-i18n-title],[data-i18n-placeholder],[data-i18n-aria-label]");
	for (var el of elements) {
		applyStaticTranslation(el, el.getAttribute("data-i18n"));
		applyStaticTranslation(el, el.getAttribute("data-i18n-title"), "title");
		applyStaticTranslation(el, el.getAttribute("data-i18n-placeholder"), "placeholder");
		applyStaticTranslation(el, el.getAttribute("data-i18n-aria-label"), "aria-label");
	}
}

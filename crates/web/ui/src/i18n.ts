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

import type { ReadonlySignal } from "@preact/signals";
import { signal, useComputed } from "@preact/signals";
import i18next from "i18next";

const STORAGE_KEY = "moltis-locale";
let initPromise: Promise<void> | null = null;
const SUPPORTED_LOCALES = new Set(["en", "fr", "zh"]);
export const supportedLocales: readonly string[] = Object.freeze(["en", "fr", "zh"]);

function normalizeLocaleTag(value: string | null | undefined): string {
	if (!value) return "en";
	let tag = String(value).trim().replace("_", "-");
	if (!tag) return "en";
	const idx = tag.indexOf("-");
	if (idx !== -1) {
		tag = tag.slice(0, idx);
	}
	return tag.toLowerCase();
}

function resolveSupportedLocale(value: string | null | undefined): string {
	const normalized = normalizeLocaleTag(value);
	if (SUPPORTED_LOCALES.has(normalized)) return normalized;
	return "en";
}

export function getPreferredLocale(): string {
	const stored = localStorage.getItem(STORAGE_KEY);
	if (stored) {
		return resolveSupportedLocale(stored);
	}
	return resolveSupportedLocale(navigator.language || "en");
}

// ── Locale signal ───────────────────────────────────────────
// Reactive — Preact components that read locale.value will re-render
// when the language changes.
export const locale = signal<string>(getPreferredLocale());

// ── Namespace registry ──────────────────────────────────────
// Maps namespace name → lazy loader. English bundles are loaded eagerly
// at init(); other locales load on demand via setLocale().
const namespaces: Record<string, (lng: string) => Promise<{ default?: Record<string, unknown> }>> = {
	common: (lng: string) => import(`./locales/${lng}/common.ts`),
	errors: (lng: string) => import(`./locales/${lng}/errors.ts`),
	settings: (lng: string) => import(`./locales/${lng}/settings.ts`),
	providers: (lng: string) => import(`./locales/${lng}/providers.ts`),
	chat: (lng: string) => import(`./locales/${lng}/chat.ts`),
	onboarding: (lng: string) => import(`./locales/${lng}/onboarding.ts`),
	login: (lng: string) => import(`./locales/${lng}/login.ts`),
	crons: (lng: string) => import(`./locales/${lng}/crons.ts`),
	mcp: (lng: string) => import(`./locales/${lng}/mcp.ts`),
	skills: (lng: string) => import(`./locales/${lng}/skills.ts`),
	channels: (lng: string) => import(`./locales/${lng}/channels.ts`),
	hooks: (lng: string) => import(`./locales/${lng}/hooks.ts`),
	projects: (lng: string) => import(`./locales/${lng}/projects.ts`),
	images: (lng: string) => import(`./locales/${lng}/images.ts`),
	metrics: (lng: string) => import(`./locales/${lng}/metrics.ts`),
	pwa: (lng: string) => import(`./locales/${lng}/pwa.ts`),
	sessions: (lng: string) => import(`./locales/${lng}/sessions.ts`),
	logs: (lng: string) => import(`./locales/${lng}/logs.ts`),
};

// ── Load all namespace bundles for a language ───────────────
function loadLanguage(lng: string): Promise<void[]> {
	const keys = Object.keys(namespaces);
	const promises = keys.map((ns) =>
		namespaces[ns](lng)
			.then((mod) => {
				i18next.addResourceBundle(lng, ns, mod.default || mod, true, true);
			})
			.catch((err: unknown) => {
				console.warn(`[i18n] failed to load ${lng}/${ns}`, err);
			}),
	);
	return Promise.all(promises);
}

function applyDocumentLocale(lng: string): void {
	if (typeof document === "undefined" || !document.documentElement) return;
	document.documentElement.lang = lng || "en";
}

// ── Public API ──────────────────────────────────────────────

/**
 * Initialise i18next with English bundles.
 * Call once at app startup before any t() calls.
 */
export function init(): Promise<void> {
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
				// Discard the TFunction return; we only need the side-effect.
				return void i18next.changeLanguage(locale.value);
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
export function t(key: string, opts?: string | Record<string, unknown>): string {
	return i18next.t(key, opts as Record<string, unknown>);
}

export function hasTranslation(key: string, opts?: Record<string, unknown>): boolean {
	return i18next.exists(key, opts);
}

/**
 * Preact hook — returns { t, locale } that triggers re-render on locale change.
 *
 * Usage:
 *   const { t } = useTranslation("settings");
 *   return html`<h2>${t("identity.title")}</h2>`;
 */
export function useTranslation(ns: string): {
	t: (key: string, opts?: Record<string, unknown>) => string;
	locale: string;
} {
	// Reading locale.value inside useComputed creates a reactive dependency.
	// When locale changes, the computed re-evaluates and Preact re-renders.
	const bound: ReadonlySignal<{ t: (key: string, opts?: Record<string, unknown>) => string; locale: string }> =
		useComputed(() => {
			const _lng = locale.value; // subscribe to signal
			void _lng;
			return {
				t: (key: string, opts?: Record<string, unknown>) => {
					const options = opts ? Object.assign({ ns: ns }, opts) : { ns: ns };
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
export function setLocale(lng: string): Promise<void> {
	const normalized = resolveSupportedLocale(lng);
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

function applyStaticTranslation(el: Element, key: string | null, attrName?: string): void {
	if (!key) return;
	const translated = i18next.t(key);
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
export function translateStaticElements(root: Element | null): void {
	if (!root) return;
	const elements = root.querySelectorAll(
		"[data-i18n],[data-i18n-title],[data-i18n-placeholder],[data-i18n-aria-label]",
	);
	for (const el of elements) {
		applyStaticTranslation(el, el.getAttribute("data-i18n"));
		applyStaticTranslation(el, el.getAttribute("data-i18n-title"), "title");
		applyStaticTranslation(el, el.getAttribute("data-i18n-placeholder"), "placeholder");
		applyStaticTranslation(el, el.getAttribute("data-i18n-aria-label"), "aria-label");
	}
}

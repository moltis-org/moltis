#!/usr/bin/env node
/**
 * Validates structural parity across all language versions of the site.
 * Checks: same number of sections, same IDs, same classes on key elements,
 * same interactive elements (buttons, links, inputs).
 */

import { readFileSync, existsSync } from "fs";
import { JSDOM } from "jsdom";

const LANGUAGES = [
  { code: "en", locale: "en_US", file: "index.en.html" },
  { code: "fr", locale: "fr_FR", file: "index.fr.html" },
  { code: "zh", locale: "zh_CN", file: "index.zh.html" },
  { code: "es", locale: "es_ES", file: "index.es.html" },
  { code: "de", locale: "de_DE", file: "index.de.html" },
  { code: "it", locale: "it_IT", file: "index.it.html" },
  { code: "pt", locale: "pt_BR", file: "index.pt.html" },
  { code: "ja", locale: "ja_JP", file: "index.ja.html" },
  { code: "ko", locale: "ko_KR", file: "index.ko.html" },
  { code: "ru", locale: "ru_RU", file: "index.ru.html" },
];

let exitCode = 0;

function fail(msg) {
  console.error(`  FAIL: ${msg}`);
  exitCode = 1;
}

function pass(msg) {
  console.log(`  OK: ${msg}`);
}

function loadDOM(file) {
  const html = readFileSync(file, "utf-8");
  return new JSDOM(html).window.document;
}

// Load reference (English) and all other languages
const enDoc = loadDOM(LANGUAGES[0].file);
const docs = new Map();
for (const lang of LANGUAGES) {
  if (!existsSync(lang.file)) {
    fail(`${lang.code.toUpperCase()}: file ${lang.file} does not exist`);
    continue;
  }
  docs.set(lang.code, loadDOM(lang.file));
}

console.log(`Validating i18n structural parity (${docs.size} languages)...\n`);

// 1. Check <html lang> and og:locale for each language
for (const lang of LANGUAGES) {
  const doc = docs.get(lang.code);
  if (!doc) continue;

  const htmlLang = doc.documentElement.getAttribute("lang");
  if (htmlLang === lang.code) pass(`${lang.code.toUpperCase()} lang="${htmlLang}"`);
  else fail(`${lang.code.toUpperCase()} lang should be "${lang.code}", got "${htmlLang}"`);

  const locale = doc.querySelector('meta[property="og:locale"]')?.getAttribute("content");
  if (locale === lang.locale) pass(`${lang.code.toUpperCase()} og:locale="${locale}"`);
  else fail(`${lang.code.toUpperCase()} og:locale should be "${lang.locale}", got "${locale}"`);
}

// 2. Compare element counts by selector (each language vs English)
const selectors = [
  ".page-content",
  ".nav-tab",
  "table",
  "table tr",
  "table th",
  "table td",
  "img",
  "a[href]",
  "button",
  "svg",
  "[id]",
  "section, div.mb-8, div.mb-10",
];

for (const lang of LANGUAGES) {
  if (lang.code === "en") continue;
  const doc = docs.get(lang.code);
  if (!doc) continue;

  console.log(`\n--- ${lang.code.toUpperCase()} vs EN ---`);

  for (const sel of selectors) {
    const enCount = enDoc.querySelectorAll(sel).length;
    const langCount = doc.querySelectorAll(sel).length;
    if (enCount === langCount) {
      pass(`${sel}: ${enCount} elements`);
    } else {
      fail(`${sel}: EN has ${enCount}, ${lang.code.toUpperCase()} has ${langCount}`);
    }
  }

  // 3. Compare all IDs
  const enIds = [...enDoc.querySelectorAll("[id]")].map((el) => el.id).sort();
  const langIds = [...doc.querySelectorAll("[id]")].map((el) => el.id).sort();

  const missingIds = enIds.filter((id) => !langIds.includes(id));
  const extraIds = langIds.filter((id) => !enIds.includes(id));

  if (missingIds.length === 0) pass(`All EN IDs present in ${lang.code.toUpperCase()}`);
  else fail(`IDs missing in ${lang.code.toUpperCase()}: ${missingIds.join(", ")}`);

  if (extraIds.length === 0) pass(`No extra IDs in ${lang.code.toUpperCase()}`);
  else fail(`Extra IDs in ${lang.code.toUpperCase()}: ${extraIds.join(", ")}`);

  // 4. Check data-page attributes match
  const enPages = [...enDoc.querySelectorAll("[data-page]")]
    .map((el) => el.dataset.page)
    .sort();
  const langPages = [...doc.querySelectorAll("[data-page]")]
    .map((el) => el.dataset.page)
    .sort();

  if (JSON.stringify(enPages) === JSON.stringify(langPages)) {
    pass(`data-page attributes match (${enPages.length} elements)`);
  } else {
    fail(`data-page mismatch: EN=[${enPages}] ${lang.code.toUpperCase()}=[${langPages}]`);
  }

  // 5. Check data-tab attributes match
  const enTabs = [...enDoc.querySelectorAll("[data-tab]")]
    .map((el) => el.dataset.tab)
    .sort();
  const langTabs = [...doc.querySelectorAll("[data-tab]")]
    .map((el) => el.dataset.tab)
    .sort();

  if (JSON.stringify(enTabs) === JSON.stringify(langTabs)) {
    pass(`data-tab attributes match (${enTabs.length} elements)`);
  } else {
    fail(`data-tab mismatch: EN=[${enTabs}] ${lang.code.toUpperCase()}=[${langTabs}]`);
  }

  // 6. Check external links match (hrefs)
  const enHrefs = [...enDoc.querySelectorAll('a[href^="http"]')]
    .map((a) => a.href)
    .sort();
  const langHrefs = [...doc.querySelectorAll('a[href^="http"]')]
    .map((a) => a.href)
    .sort();

  if (JSON.stringify(enHrefs) === JSON.stringify(langHrefs)) {
    pass(`External links match (${enHrefs.length} links)`);
  } else {
    const missingLinks = enHrefs.filter((h) => !langHrefs.includes(h));
    const extraLinks = langHrefs.filter((h) => !enHrefs.includes(h));
    if (missingLinks.length) fail(`Links missing in ${lang.code.toUpperCase()}: ${missingLinks.join(", ")}`);
    if (extraLinks.length) fail(`Extra links in ${lang.code.toUpperCase()}: ${extraLinks.join(", ")}`);
  }
}

// Summary
console.log();
if (exitCode === 0) {
  console.log("All checks passed.");
} else {
  console.error("Some checks failed. Fix the issues above.");
}

process.exit(exitCode);

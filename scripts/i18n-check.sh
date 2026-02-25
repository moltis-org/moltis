#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
locales_dir="$repo_root/crates/web/src/assets/js/locales"

if [[ ! -d "$locales_dir/en" ]]; then
	echo "Missing English locale directory: $locales_dir/en" >&2
	exit 1
fi

node --input-type=module - "$locales_dir" <<'NODE'
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const localesDir = process.argv[2];

function flattenKeys(value, prefix = "", out = new Set()) {
	if (value == null || typeof value !== "object" || Array.isArray(value)) {
		if (prefix) out.add(prefix);
		return out;
	}
	for (const [key, child] of Object.entries(value)) {
		const next = prefix ? `${prefix}.${key}` : key;
		if (child != null && typeof child === "object" && !Array.isArray(child)) {
			flattenKeys(child, next, out);
		} else {
			out.add(next);
		}
	}
	return out;
}

async function loadLocaleModule(filePath) {
	const fileUrl = `${pathToFileURL(filePath).href}?v=${Date.now()}`;
	const mod = await import(fileUrl);
	return mod.default ?? {};
}

function sortedLocaleDirs(baseDir) {
	return fs
		.readdirSync(baseDir, { withFileTypes: true })
		.filter((entry) => entry.isDirectory())
		.map((entry) => entry.name)
		.sort();
}

function sortedNamespaceFiles(enDir) {
	return fs
		.readdirSync(enDir, { withFileTypes: true })
		.filter((entry) => entry.isFile() && entry.name.endsWith(".js"))
		.map((entry) => entry.name)
		.sort();
}

const localeDirs = sortedLocaleDirs(localesDir);
if (!localeDirs.includes("en")) {
	console.error("Missing required locale directory: en");
	process.exit(1);
}

const namespaceFiles = sortedNamespaceFiles(path.join(localesDir, "en"));
if (namespaceFiles.length === 0) {
	console.error("No locale namespace files found under en/");
	process.exit(1);
}

let hasFailures = false;

for (const namespaceFile of namespaceFiles) {
	const enPath = path.join(localesDir, "en", namespaceFile);
	const enObj = await loadLocaleModule(enPath);
	const enKeys = flattenKeys(enObj);

	for (const locale of localeDirs) {
		if (locale === "en") continue;

		const localePath = path.join(localesDir, locale, namespaceFile);
		if (!fs.existsSync(localePath)) {
			hasFailures = true;
			console.error(`[${locale}] missing namespace file: ${namespaceFile}`);
			continue;
		}

		const localeObj = await loadLocaleModule(localePath);
		const localeKeys = flattenKeys(localeObj);

		const missing = [...enKeys].filter((k) => !localeKeys.has(k)).sort();
		const extra = [...localeKeys].filter((k) => !enKeys.has(k)).sort();

		if (missing.length === 0 && extra.length === 0) {
			continue;
		}

		hasFailures = true;
		console.error(`[${locale}/${namespaceFile}] missing=${missing.length}, extra=${extra.length}`);
		for (const key of missing.slice(0, 20)) {
			console.error(`  - missing: ${key}`);
		}
		if (missing.length > 20) {
			console.error(`  - ... ${missing.length - 20} more missing keys`);
		}
		for (const key of extra.slice(0, 20)) {
			console.error(`  + extra: ${key}`);
		}
		if (extra.length > 20) {
			console.error(`  + ... ${extra.length - 20} more extra keys`);
		}
	}
}

if (hasFailures) {
	process.exit(1);
}

console.log(`i18n parity OK: ${localeDirs.length} locales, ${namespaceFiles.length} namespaces.`);
NODE

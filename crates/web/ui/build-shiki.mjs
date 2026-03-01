// Build script: bundles shiki/bundle/web into a single vendor .mjs file.
// The web bundle includes ~25 common languages and inlines the oniguruma WASM.
//
// Usage: node build-shiki.mjs

import { build } from "esbuild";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const outfile = join(__dirname, "..", "src", "assets", "js", "vendor", "shiki.mjs");

await build({
	entryPoints: ["shiki/bundle/web"],
	bundle: true,
	format: "esm",
	outfile,
	minify: true,
	target: "es2022",
	platform: "browser",
	// Inline the oniguruma WASM as base64 data URL
	loader: { ".wasm": "binary" },
	// Tree-shake unused exports
	treeShaking: true,
});

console.log(`Built ${outfile}`);

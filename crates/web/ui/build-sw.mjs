import { createHash } from "node:crypto";
import { readdir, readFile } from "node:fs/promises";
import { build } from "esbuild";

const hash = createHash("sha256");

async function hashTree(url, root = url) {
	let entries;
	try {
		entries = await readdir(url, { withFileTypes: true });
	} catch (error) {
		if (error.code === "ENOENT") return;
		throw error;
	}
	for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
		if (entry.name === "sw.js") continue;
		const child = new URL(entry.name, url.href.endsWith("/") ? url : new URL(`${url.href}/`));
		if (entry.isDirectory()) {
			await hashTree(new URL(`${child.href}/`), root);
		} else if (entry.isFile()) {
			hash.update(child.pathname.slice(root.pathname.length));
			hash.update(await readFile(child));
		}
	}
}

for (const path of ["./src/sw.ts", "./build-sw.mjs", "./package.json", "./package-lock.json"]) {
	hash.update(path);
	hash.update(await readFile(new URL(path, import.meta.url)));
}
await hashTree(new URL("../src/assets/", import.meta.url));
const version = hash.digest("hex").slice(0, 16);

await build({
	entryPoints: ["src/sw.ts"],
	bundle: true,
	format: "esm",
	outfile: "../src/assets/sw.js",
	define: { __MOLTIS_SW_VERSION__: JSON.stringify(version) },
});

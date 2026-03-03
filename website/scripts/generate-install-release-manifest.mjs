#!/usr/bin/env node

import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

const DEFAULT_SITE = "https://moltis.org";
const DEFAULT_REPO = "moltis-org/moltis";
const DEFAULT_CHANNEL = "stable";

const RELEASE_TARGETS = [
  { target: "aarch64-apple-darwin", ext: "tar.gz" },
  { target: "x86_64-apple-darwin", ext: "tar.gz" },
  { target: "aarch64-unknown-linux-gnu", ext: "tar.gz" },
  { target: "x86_64-unknown-linux-gnu", ext: "tar.gz" },
  { target: "x86_64-pc-windows-msvc", ext: "exe" }
];

function usage() {
  console.error(`Usage:
  node scripts/generate-install-release-manifest.mjs --version <x.y.z> [options]

Options:
  --version <x.y.z>           Release version (required)
  --published-at <YYYY-MM-DD> Release date (default: UTC today)
  --channel <name>            Channel to update (default: stable)
  --repo <owner/repo>         GitHub repo (default: moltis-org/moltis)
  --site <https://host>       Site origin (default: https://moltis.org)
  --checksums-file <path>     File containing lines: <sha256> <filename>
  --allow-missing-sha         Allow missing SHA256 entries
`);
}

function parseArgs(argv) {
  const out = {
    channel: DEFAULT_CHANNEL,
    repo: DEFAULT_REPO,
    site: DEFAULT_SITE,
    checksumsFile: null,
    allowMissingSha: false
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--allow-missing-sha") {
      out.allowMissingSha = true;
      continue;
    }

    const value = argv[i + 1];
    if (value == null) {
      throw new Error(`Missing value for ${arg}`);
    }

    if (arg === "--version") out.version = value;
    else if (arg === "--published-at") out.publishedAt = value;
    else if (arg === "--channel") out.channel = value;
    else if (arg === "--repo") out.repo = value;
    else if (arg === "--site") out.site = value;
    else if (arg === "--checksums-file") out.checksumsFile = value;
    else throw new Error(`Unknown argument: ${arg}`);
    i += 1;
  }

  if (!out.version) {
    throw new Error("--version is required");
  }

  if (!/^\d+\.\d+\.\d+$/.test(out.version)) {
    throw new Error(`Invalid version '${out.version}' (expected x.y.z)`);
  }

  if (!out.publishedAt) {
    out.publishedAt = new Date().toISOString().slice(0, 10);
  }

  if (!/^\d{4}-\d{2}-\d{2}$/.test(out.publishedAt)) {
    throw new Error(`Invalid date '${out.publishedAt}' (expected YYYY-MM-DD)`);
  }

  return out;
}

function parseChecksums(content) {
  const map = new Map();
  const lines = content.split(/\r?\n/);

  for (const raw of lines) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;

    const match = line.match(/^([a-fA-F0-9]{64})\s+\*?(.+)$/);
    if (!match) continue;

    const [, sha, filename] = match;
    map.set(filename.trim(), sha.toLowerCase());
  }

  return map;
}

function releaseAsset(version, target, ext, repo) {
  const filename = `moltis-${version}-${target}.${ext}`;
  const base = `https://github.com/${repo}/releases/download/v${version}`;
  return {
    target,
    filename,
    url: `${base}/${filename}`,
    sha256_url: `${base}/${filename}.sha256`
  };
}

async function loadChecksumsMap(checksumsFile) {
  if (!checksumsFile) {
    return new Map();
  }

  const content = await readFile(checksumsFile, "utf8");
  return parseChecksums(content);
}

async function main() {
  let opts;
  try {
    opts = parseArgs(process.argv.slice(2));
  } catch (error) {
    usage();
    console.error(`\nError: ${error.message}`);
    process.exit(2);
  }

  const root = process.cwd();
  const channelDir = path.join(root, ".well-known", "moltis-install", "channels");
  const releaseDir = path.join(root, ".well-known", "moltis-install", "releases");

  await mkdir(channelDir, { recursive: true });
  await mkdir(releaseDir, { recursive: true });

  const checksumsMap = await loadChecksumsMap(opts.checksumsFile);

  const artifacts = RELEASE_TARGETS.map(({ target, ext }) => {
    const base = releaseAsset(opts.version, target, ext, opts.repo);
    const sha256 = checksumsMap.get(base.filename) || null;
    return {
      ...base,
      sha256
    };
  });

  const missingSha = artifacts.filter((item) => !item.sha256).map((item) => item.filename);
  if (missingSha.length > 0 && !opts.allowMissingSha) {
    console.error("Missing SHA256 for release artifacts:");
    for (const name of missingSha) {
      console.error(`  - ${name}`);
    }
    console.error("\nProvide --checksums-file or pass --allow-missing-sha.");
    process.exit(3);
  }

  const releaseManifest = {
    release_manifest_version: "1",
    version: opts.version,
    tag: `v${opts.version}`,
    published_at: opts.publishedAt,
    repo: opts.repo,
    artifacts,
    notes:
      missingSha.length > 0
        ? "Some sha256 values are null. Regenerate with --checksums-file before production use."
        : "All artifact hashes resolved from checksums file."
  };

  const channelManifest = {
    channel_manifest_version: "1",
    channel: opts.channel,
    updated_at: new Date().toISOString(),
    latest_version: opts.version,
    release_manifest: `${opts.site}/.well-known/moltis-install/releases/${opts.version}.json`,
    release_notes_url: `https://github.com/${opts.repo}/releases/tag/v${opts.version}`
  };

  const releasePath = path.join(releaseDir, `${opts.version}.json`);
  const channelPath = path.join(channelDir, `${opts.channel}.json`);

  await writeFile(releasePath, `${JSON.stringify(releaseManifest, null, 2)}\n`, "utf8");
  await writeFile(channelPath, `${JSON.stringify(channelManifest, null, 2)}\n`, "utf8");

  console.log(`Wrote ${path.relative(root, releasePath)}`);
  console.log(`Wrote ${path.relative(root, channelPath)}`);

  if (missingSha.length > 0) {
    console.log("\nWarning: Missing SHA256 values for:");
    for (const name of missingSha) {
      console.log(`  - ${name}`);
    }
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

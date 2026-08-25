import { readFile, writeFile } from "node:fs/promises";

const [operation] = process.argv.slice(2);
if (!["--check", "patch", "minor", "major"].includes(operation ?? "")) {
  throw new Error("Usage: version-bump.mjs <--check|patch|minor|major>");
}

const packageJson = JSON.parse(await readFile("package.json", "utf8"));
const packageLock = JSON.parse(await readFile("package-lock.json", "utf8"));
const tauriJson = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"));
let cargoToml = await readFile("src-tauri/Cargo.toml", "utf8");
let cargoLock = await readFile("src-tauri/Cargo.lock", "utf8");

const cargoTomlVersion = cargoToml.match(/\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
const cargoLockVersion = cargoLock.match(
  /\[\[package\]\]\r?\nname = "orcestr-real-translate"\r?\nversion = "([^"]+)"/,
)?.[1];
const versions = new Map([
  ["package.json", packageJson.version],
  ["package-lock.json", packageLock.version],
  ["package-lock.json root package", packageLock.packages?.[""]?.version],
  ["src-tauri/tauri.conf.json", tauriJson.version],
  ["src-tauri/Cargo.toml", cargoTomlVersion],
  ["src-tauri/Cargo.lock", cargoLockVersion],
]);
const uniqueVersions = new Set(versions.values());
if (uniqueVersions.size !== 1 || uniqueVersions.has(undefined)) {
  throw new Error(
    `Version files are out of sync:\n${[...versions].map(([file, version]) => `- ${file}: ${version ?? "<missing>"}`).join("\n")}`,
  );
}

const current = packageJson.version;
if (!/^\d+\.\d+\.\d+$/.test(current)) {
  throw new Error(`Only stable semantic versions are supported, received ${current}`);
}
if (operation === "--check") {
  process.stdout.write(`All version files are synchronized at ${current}.\n`);
  process.exit(0);
}

const parts = current.split(".").map(Number);
if (operation === "major") {
  parts[0] += 1;
  parts[1] = 0;
  parts[2] = 0;
} else if (operation === "minor") {
  parts[1] += 1;
  parts[2] = 0;
} else {
  parts[2] += 1;
}
const next = parts.join(".");

packageJson.version = next;
packageLock.version = next;
packageLock.packages[""].version = next;
tauriJson.version = next;
cargoToml = cargoToml.replace(
  /(\[package\][\s\S]*?^version\s*=\s*")[^"]+("\s*$)/m,
  `$1${next}$2`,
);
cargoLock = cargoLock.replace(
  /(\[\[package\]\]\r?\nname = "orcestr-real-translate"\r?\nversion = ")[^"]+("\r?$)/m,
  `$1${next}$2`,
);

const json = (value) => `${JSON.stringify(value, null, 2)}\n`;
await Promise.all([
  writeFile("package.json", json(packageJson), "utf8"),
  writeFile("package-lock.json", json(packageLock), "utf8"),
  writeFile("src-tauri/tauri.conf.json", json(tauriJson), "utf8"),
  writeFile("src-tauri/Cargo.toml", cargoToml, "utf8"),
  writeFile("src-tauri/Cargo.lock", cargoLock, "utf8"),
]);
process.stdout.write(`Version updated from ${current} to ${next}. No commit, tag, or push was created.\n`);

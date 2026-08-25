import { readFile } from "node:fs/promises";

const [tag] = process.argv.slice(2);

if (!/^v\d+\.\d+\.\d+$/.test(tag ?? "")) {
  throw new Error("Release tag must be an exact stable semver such as v1.2.3");
}

const expected = tag.slice(1);
const packageJson = JSON.parse(await readFile("package.json", "utf8"));
const tauriJson = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"));
const cargoToml = await readFile("src-tauri/Cargo.toml", "utf8");
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

for (const [source, version] of [
  ["package.json", packageJson.version],
  ["src-tauri/tauri.conf.json", tauriJson.version],
  ["src-tauri/Cargo.toml", cargoVersion],
]) {
  if (version !== expected) {
    throw new Error(`${source} version ${version ?? "<missing>"} does not match ${tag}`);
  }
}

process.stdout.write(`Release version ${expected} is consistent.\n`);

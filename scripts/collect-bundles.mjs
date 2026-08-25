import { copyFile, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";

const [bundleRoot, outputDirectory, platform] = process.argv.slice(2);
if (!bundleRoot || !outputDirectory || !/^(windows|darwin|linux)$/.test(platform ?? "")) {
  throw new Error("Usage: collect-bundles.mjs <bundle-root> <output-dir> <windows|darwin|linux>");
}

const runnerArchitecture =
  { X64: "x86_64", ARM64: "aarch64", X86: "i686" }[process.env.RUNNER_ARCH] ??
  { x64: "x86_64", arm64: "aarch64", ia32: "i686" }[process.arch];
if (!runnerArchitecture) {
  throw new Error(`Unsupported release architecture: ${process.env.RUNNER_ARCH ?? process.arch}`);
}
const universalMac = platform === "darwin" && process.env.RELEASE_ARCHITECTURE === "universal";
const updaterArchitectures = universalMac
  ? ["aarch64", "x86_64"]
  : [runnerArchitecture];

const deliverablePatterns = {
  windows: [/\.exe$/i, /\.msi$/i, /\.(?:exe|msi)\.sig$/i],
  darwin: [/\.dmg$/i, /\.app\.tar\.gz$/i, /\.app\.tar\.gz\.sig$/i],
  linux: [/\.AppImage$/i, /\.AppImage\.sig$/i, /\.deb$/i, /\.deb\.sig$/i],
};
const updaterTargets = {
  windows: [
    { installer: "nsis", pattern: /-setup\.exe$/i },
    { installer: "msi", pattern: /\.msi$/i },
  ],
  darwin: [{ installer: "app", pattern: /\.app\.tar\.gz$/i }],
  linux: [
    { installer: "appimage", pattern: /\.AppImage$/i },
    { installer: "deb", pattern: /\.deb$/i },
  ],
};

async function walk(directory) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await walk(path)));
    } else if (entry.isFile()) {
      files.push(path);
    }
  }
  return files;
}

const allFiles = await walk(bundleRoot);
const deliverables = allFiles.filter((file) =>
  deliverablePatterns[platform].some((pattern) => pattern.test(file)),
);
if (deliverables.length === 0) {
  throw new Error(`No ${platform} distributable bundles were found below ${bundleRoot}`);
}

const updaters = [];
for (const { installer, pattern } of updaterTargets[platform]) {
  const artifact = deliverables.find((file) => pattern.test(file) && !file.endsWith(".sig"));
  if (!artifact) {
    throw new Error(`Missing ${platform}-${installer} updater artifact`);
  }
  const signaturePath = `${artifact}.sig`;
  if (!deliverables.includes(signaturePath)) {
    throw new Error(`Missing signed updater sidecar: ${signaturePath}`);
  }
  for (const architecture of updaterArchitectures) {
    updaters.push({
      target: `${platform}-${architecture}-${installer}`,
      updaterFile: `${platform}-${basename(artifact)}`,
      signature: (await readFile(signaturePath, "utf8")).trim(),
    });
  }
}

await rm(outputDirectory, { recursive: true, force: true });
await mkdir(outputDirectory, { recursive: true });
const copiedNames = new Set();
for (const source of deliverables) {
  const name = `${platform}-${basename(source)}`;
  if (copiedNames.has(name)) {
    throw new Error(`Duplicate release artifact name: ${name}`);
  }
  copiedNames.add(name);
  await copyFile(source, join(outputDirectory, name));
}

const metadata = {
  platform,
  architecture: universalMac ? "universal" : runnerArchitecture,
  files: [...copiedNames].sort(),
  updaters,
};
await writeFile(
  join(outputDirectory, `${platform}-metadata.json`),
  `${JSON.stringify(metadata, null, 2)}\n`,
  "utf8",
);
process.stdout.write(
  `Collected ${metadata.files.length} ${platform} artifact(s) and ${updaters.length} signed updater target(s).\n`,
);

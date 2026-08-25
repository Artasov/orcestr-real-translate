import { readFile, readdir, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";

const [artifactDirectory, tag, publicBaseUrl, outputFile, channelOutputFile] = process.argv.slice(2);
if (!artifactDirectory || !/^v\d+\.\d+\.\d+$/.test(tag ?? "") || !publicBaseUrl || !outputFile) {
  throw new Error(
    "Usage: build-release-manifest.mjs <artifact-dir> <vX.Y.Z> <public-base-url> <version-output> [channel-output]",
  );
}

const sourceDateEpoch = process.env.SOURCE_DATE_EPOCH;
if (!/^(0|[1-9]\d*)$/.test(sourceDateEpoch ?? "")) {
  throw new Error("SOURCE_DATE_EPOCH must be a non-negative integer Unix timestamp in seconds");
}
const epochSeconds = Number(sourceDateEpoch);
const epochMilliseconds = epochSeconds * 1000;
if (!Number.isSafeInteger(epochSeconds) || !Number.isFinite(epochMilliseconds)) {
  throw new Error("SOURCE_DATE_EPOCH is outside the supported numeric range");
}
const pubDate = new Date(epochMilliseconds);
if (Number.isNaN(pubDate.getTime())) {
  throw new Error("SOURCE_DATE_EPOCH is outside the supported date range");
}

const publicBase = new URL(publicBaseUrl.endsWith("/") ? publicBaseUrl : `${publicBaseUrl}/`);
if (publicBase.protocol !== "https:") {
  throw new Error("S3_PUBLIC_BASE_URL must use HTTPS");
}

const version = tag.slice(1);
const metadataNames = (await readdir(artifactDirectory))
  .filter((name) => /^(windows|darwin|linux)-metadata\.json$/.test(name))
  .sort();
const metadata = await Promise.all(
  metadataNames.map(async (name) =>
    JSON.parse(await readFile(join(artifactDirectory, name), "utf8")),
  ),
);
if (metadata.length !== 3) {
  throw new Error("Release publication requires metadata from Windows, macOS, and Linux");
}

const targetPattern = /^(windows|darwin|linux)-(x86_64|aarch64|i686)-(nsis|msi|app|appimage|deb)$/;
const entries = metadata.flatMap((item) => {
  if (!Array.isArray(item.files) || !Array.isArray(item.updaters)) {
    throw new Error(`Invalid release metadata for ${String(item.platform)}`);
  }
  return item.updaters.map((updater) => {
    if (
      typeof updater.target !== "string" ||
      !targetPattern.test(updater.target) ||
      typeof updater.updaterFile !== "string" ||
      basename(updater.updaterFile) !== updater.updaterFile ||
      !item.files.includes(updater.updaterFile) ||
      typeof updater.signature !== "string" ||
      !updater.signature.trim()
    ) {
      throw new Error(`Invalid signed updater metadata for ${String(updater.target)}`);
    }
    return { ...updater, signature: updater.signature.trim() };
  });
});

for (const required of [
  /-nsis$/,
  /-msi$/,
  /^darwin-.*-app$/,
  /-appimage$/,
  /-deb$/,
]) {
  if (!entries.some(({ target }) => required.test(target))) {
    throw new Error(`Missing signed updater target matching ${required}`);
  }
}
if (new Set(entries.map(({ target }) => target)).size !== entries.length) {
  throw new Error("Duplicate updater target in aggregate metadata");
}

const publicUrl = (filename) => {
  const objectPath = `orcestr-real-translate/v${version}/${encodeURIComponent(filename)}`;
  return new URL(objectPath, publicBase).toString();
};
const platforms = Object.fromEntries(
  entries.map(({ target, updaterFile, signature }) => [
    target,
    { signature, url: publicUrl(updaterFile) },
  ]),
);
const serialized = `${JSON.stringify(
  {
    version,
    notes: `Orcestr Real Translate ${tag}`,
    pub_date: pubDate.toISOString(),
    platforms,
  },
  null,
  2,
)}\n`;

await writeFile(outputFile, serialized, "utf8");
if (channelOutputFile) {
  await writeFile(channelOutputFile, serialized, "utf8");
}

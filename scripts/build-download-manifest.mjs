import { createHash } from "node:crypto";
import { readFile, readdir, stat, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";

const [artifactDirectory, tag, publicBaseUrl, outputFile, channelOutputFile] =
  process.argv.slice(2);

if (
  !artifactDirectory ||
  !/^v\d+\.\d+\.\d+$/.test(tag ?? "") ||
  !publicBaseUrl ||
  !outputFile
) {
  throw new Error(
    "Usage: build-download-manifest.mjs <artifact-dir> <vX.Y.Z> <public-base-url> <version-output> [channel-output]",
  );
}

const sourceDateEpoch = process.env.SOURCE_DATE_EPOCH;
if (!/^(0|[1-9]\d*)$/.test(sourceDateEpoch ?? "")) {
  throw new Error("SOURCE_DATE_EPOCH must be a non-negative integer Unix timestamp in seconds");
}
const epochMilliseconds = Number(sourceDateEpoch) * 1000;
if (!Number.isSafeInteger(Number(sourceDateEpoch)) || !Number.isFinite(epochMilliseconds)) {
  throw new Error("SOURCE_DATE_EPOCH is outside the supported numeric range");
}
const publishedAt = new Date(epochMilliseconds);
if (Number.isNaN(publishedAt.getTime())) {
  throw new Error("SOURCE_DATE_EPOCH is outside the supported date range");
}

const publicBase = new URL(publicBaseUrl.endsWith("/") ? publicBaseUrl : `${publicBaseUrl}/`);
if (publicBase.protocol !== "https:") {
  throw new Error("S3_PUBLIC_BASE_URL must use HTTPS");
}

const targetByPlatform = {
  windows: "windows-x64",
  darwin: "macos-universal",
  linux: "linux-x64",
};
const downloadablePatterns = {
  windows: [/\.exe$/i, /\.msi$/i],
  darwin: [/\.dmg$/i],
  linux: [/\.AppImage$/i, /\.deb$/i],
};

const metadataNames = (await readdir(artifactDirectory))
  .filter((name) => /^(windows|darwin|linux)-metadata\.json$/.test(name))
  .sort();
if (metadataNames.length !== 3) {
  throw new Error("Download manifest requires metadata from Windows, macOS, and Linux");
}

const version = tag.slice(1);
const publicUrl = (filename) =>
  new URL(
    `orcestr-real-translate/v${version}/${encodeURIComponent(filename)}`,
    publicBase,
  ).toString();

const targets = {};
for (const metadataName of metadataNames) {
  const metadata = JSON.parse(
    await readFile(join(artifactDirectory, metadataName), "utf8"),
  );
  const platform = metadata.platform;
  if (
    !Object.hasOwn(targetByPlatform, platform) ||
    !Array.isArray(metadata.files) ||
    typeof metadata.architecture !== "string"
  ) {
    throw new Error(`Invalid release metadata in ${metadataName}`);
  }

  const names = metadata.files
    .filter(
      (name) =>
        typeof name === "string" &&
        basename(name) === name &&
        downloadablePatterns[platform].some((pattern) => pattern.test(name)),
    )
    .sort();
  if (names.length === 0) {
    throw new Error(`No human-installable downloads found for ${platform}`);
  }

  const files = [];
  for (const name of names) {
    const path = join(artifactDirectory, name);
    const contents = await readFile(path);
    const fileStat = await stat(path);
    files.push({
      name,
      url: publicUrl(name),
      sha256: createHash("sha256").update(contents).digest("hex"),
      size: fileStat.size,
    });
  }
  targets[targetByPlatform[platform]] = files;
}

const serialized = `${JSON.stringify(
  {
    version,
    published_at: publishedAt.toISOString(),
    targets,
  },
  null,
  2,
)}\n`;

await writeFile(outputFile, serialized, "utf8");
if (channelOutputFile) {
  await writeFile(channelOutputFile, serialized, "utf8");
}

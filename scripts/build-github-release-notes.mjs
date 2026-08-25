import { readFile, readdir, writeFile } from "node:fs/promises";
import { basename, join } from "node:path";

const [artifactDirectory, tag, publicBaseUrl, generatedNotesFile, outputFile] =
  process.argv.slice(2);
if (
  !artifactDirectory ||
  !/^v\d+\.\d+\.\d+$/.test(tag ?? "") ||
  !publicBaseUrl ||
  !generatedNotesFile ||
  !outputFile
) {
  throw new Error(
    "Usage: build-github-release-notes.mjs <artifact-dir> <vX.Y.Z> <public-base-url> <generated-notes> <output>",
  );
}

const publicBase = new URL(
  publicBaseUrl.endsWith("/") ? publicBaseUrl : `${publicBaseUrl}/`,
);
if (publicBase.protocol !== "https:") {
  throw new Error("S3_PUBLIC_BASE_URL must use HTTPS");
}

const metadataNames = (await readdir(artifactDirectory))
  .filter((name) => /^(windows|darwin|linux)-metadata\.json$/.test(name))
  .sort();
if (metadataNames.length !== 3) {
  throw new Error("GitHub release notes require Windows, macOS, and Linux metadata");
}

const version = tag.slice(1);
const labels = { windows: "Windows", darwin: "macOS", linux: "Linux" };
const sections = [];
for (const metadataName of metadataNames) {
  const metadata = JSON.parse(
    await readFile(join(artifactDirectory, metadataName), "utf8"),
  );
  if (!labels[metadata.platform] || !Array.isArray(metadata.files)) {
    throw new Error(`Invalid release metadata in ${metadataName}`);
  }
  const files = metadata.files
    .filter((file) => typeof file === "string" && !file.endsWith(".sig"))
    .sort();
  if (files.length === 0) {
    throw new Error(`No downloadable files in ${metadataName}`);
  }
  const links = files.map((file) => {
    if (basename(file) !== file) throw new Error(`Unsafe release filename: ${file}`);
    const path = `orcestr-real-translate/v${version}/${encodeURIComponent(file)}`;
    return `- [${file}](${new URL(path, publicBase).toString()})`;
  });
  sections.push(`### ${labels[metadata.platform]}\n\n${links.join("\n")}`);
}

const generatedNotes = (await readFile(generatedNotesFile, "utf8")).trim();
const body = [
  `# Orcestr Real Translate ${tag}`,
  "Installers and updater packages are hosted in immutable S3 storage. This GitHub release intentionally contains no uploaded assets.",
  "## Downloads",
  sections.join("\n\n"),
  generatedNotes ? `## Changes\n\n${generatedNotes}` : "",
]
  .filter(Boolean)
  .join("\n\n");

await writeFile(outputFile, `${body}\n`, "utf8");

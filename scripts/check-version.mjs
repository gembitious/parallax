// 앱 버전이 적힌 곳(Cargo 워크스페이스, package.json·package-lock.json, tauri.conf.json)이 모두 같은지 보고,
// 태그를 주면 그 태그가 `v<버전>`인지도 본다. 릴리스 태그를 밀기 전에, 그리고 CI가 태그 빌드에서 돌린다.
//
// 사용법: node scripts/check-version.mjs [v<버전>]

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (path) => readFileSync(join(root, path), "utf8");

function cargoWorkspaceVersion() {
  const text = read("Cargo.toml");
  const section = text.match(/^\[workspace\.package\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m);
  const version = section?.[1].match(/^version\s*=\s*"([^"]+)"/m);
  if (!version) throw new Error("Cargo.toml의 [workspace.package]에 version이 없다");
  return version[1];
}

const lock = JSON.parse(read("package-lock.json"));
const versions = {
  "Cargo.toml [workspace.package]": cargoWorkspaceVersion(),
  "package.json": JSON.parse(read("package.json")).version,
  "package-lock.json": lock.version,
  'package-lock.json packages[""]': lock.packages?.[""]?.version,
  "src-tauri/tauri.conf.json": JSON.parse(read("src-tauri/tauri.conf.json")).version,
};

const version = versions["Cargo.toml [workspace.package]"];
const wrong = Object.entries(versions).filter(([, v]) => v !== version);
if (wrong.length > 0) {
  console.error(`앱 버전이 서로 다르다(Cargo.toml: ${version}):`);
  for (const [where, v] of wrong) console.error(`  ${where}: ${v}`);
  process.exit(1);
}

const tag = process.argv[2];
if (tag !== undefined && tag !== `v${version}`) {
  console.error(`태그와 앱 버전이 다르다(태그 ${tag}, 앱 버전 ${version}). 태그는 버전을 올린 커밋에 v<버전>으로 단다.`);
  process.exit(1);
}
console.log(version);

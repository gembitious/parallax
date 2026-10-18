#!/usr/bin/env node
// 릴리스 빌드에 넣는 제3자 저작권 고지를 모은다(`npm run notices`. `tauri build`가 화면을 빌드하기 전에 돌린다).
//
// 앱 실행 파일에 링크되는 Rust 크레이트(Windows·macOS·Linux, x86_64·aarch64 모두의 합)와 화면 번들에 들어가는
// npm 패키지(package-lock.json에서 dev가 아닌 것)마다 그 패키지에 든 라이선스·NOTICE 파일을 그대로 싣는다.
// 같은 문구는 한 번만 싣고 그 문구를 쓰는 구성 요소를 위에 적는다. 라이선스 파일이 없는 패키지는 같은 저장소·같은
// 라이선스 표기인 다른 패키지의 파일을 쓰고(한 프로젝트의 여러 크레이트), 없으면 선언한 라이선스로 채운다(MIT는 작성자로
// 저작권 줄을 만들고, 저작권 줄이 없는 표준 문구(Apache-2.0, MPL-2.0 등)는 그 문구). 그래도 못 채우면 실패한다.
//
// 출력: public/third-party-notices.txt. Vite가 dist로 옮겨 실행 파일에 묻히고 앱의 "오픈 소스 라이선스"가 보인다.
// 빌드 산출물이라 커밋하지 않는다. 경로·시각을 넣지 않아 어느 머신에서 만들어도 같다.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const output = join(root, "public", "third-party-notices.txt");
/** 실행 파일 패키지(src-tauri) */
const APP_PACKAGE = "parallax";
const TARGETS = [
  "x86_64-pc-windows-msvc",
  "aarch64-pc-windows-msvc",
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
];
const LICENSE_FILE = /^(licen[sc]e|copying|copyright|notice|unlicense)([-_.].*)?$/i;
const NOT_A_LICENSE = /\.(rs|js|mjs|cjs|ts|json|toml|py|sh|c|h|html?)$/i;
const RULE = "-".repeat(80);
/** 프로젝트마다 다른 저작권 줄이 없어 다른 패키지에 든 문구를 그대로 쓸 수 있는 라이선스 */
const STANDARD_TEXT = new Set(["Apache-2.0", "MPL-2.0", "CC0-1.0", "Unlicense", "BSL-1.0"]);

/** 앱 실행 파일에 링크되는 크레이트(빌드 스크립트·테스트용 의존성은 빼고, 워크스페이스 크레이트도 뺀다) */
function crates() {
  const found = new Map();
  for (const target of TARGETS) {
    const json = execFileSync(
      "cargo",
      ["metadata", "--format-version", "1", "--locked", "--filter-platform", target],
      { cwd: root, maxBuffer: 512 << 20 },
    );
    const meta = JSON.parse(json);
    const packages = new Map(meta.packages.map((p) => [p.id, p]));
    const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
    const members = new Set(meta.workspace_members);
    const app = meta.packages.find((p) => p.name === APP_PACKAGE && members.has(p.id));
    if (!app) throw new Error(`워크스페이스에 ${APP_PACKAGE} 패키지가 없다`);
    const stack = [app.id];
    const seen = new Set();
    while (stack.length > 0) {
      const id = stack.pop();
      if (seen.has(id)) continue;
      seen.add(id);
      for (const dep of nodes.get(id).deps) {
        if (dep.dep_kinds.some((kind) => kind.kind === null)) stack.push(dep.pkg);
      }
      if (!members.has(id)) found.set(id, packages.get(id));
    }
  }
  return [...found.values()].map((p) => {
    const dir = dirname(p.manifest_path);
    return {
      name: p.name,
      version: p.version,
      license: p.license ?? null,
      url: p.repository ?? p.homepage ?? `https://crates.io/crates/${p.name}`,
      authors: p.authors ?? [],
      dir,
      extraFiles: p.license_file ? [join(dir, p.license_file)] : [],
    };
  });
}

/** 화면 번들에 들어가는 npm 패키지(dev·devOptional이 아닌 것) */
function npmPackages() {
  const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
  const list = [];
  for (const [key, entry] of Object.entries(lock.packages)) {
    if (!key.startsWith("node_modules/") || entry.dev || entry.devOptional) continue;
    const dir = join(root, key);
    if (!existsSync(join(dir, "package.json"))) {
      if (entry.optional) continue; // 다른 플랫폼 전용
      throw new Error(`${key}가 설치되지 않았다(npm install을 먼저 돌린다)`);
    }
    const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const name = pkg.name ?? key.slice(key.lastIndexOf("node_modules/") + "node_modules/".length);
    const author = typeof pkg.author === "string" ? pkg.author : pkg.author?.name;
    list.push({
      name,
      version: pkg.version ?? entry.version,
      license: entry.license ?? (typeof pkg.license === "string" ? pkg.license : pkg.license?.type) ?? null,
      url: repositoryUrl(pkg.repository) ?? pkg.homepage ?? `https://www.npmjs.com/package/${name}`,
      authors: author ? [author] : [],
      dir,
      extraFiles: [],
    });
  }
  return list;
}

function repositoryUrl(repository) {
  const url = typeof repository === "string" ? repository : repository?.url;
  if (!url) return null;
  if (/^github:/.test(url) || /^[\w.-]+\/[\w.-]+$/.test(url)) {
    return `https://github.com/${url.replace(/^github:/, "")}`;
  }
  return url.replace(/^git\+/, "").replace(/^git:\/\//, "https://").replace(/\.git$/, "");
}

/** 줄 끝을 LF로, 줄 끝 공백과 앞뒤 빈 줄을 지운다(같은 문구를 한 번만 싣기 위해). */
function normalize(text) {
  return text
    .replace(/^﻿/, "")
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n")
    .replace(/^\n+|\n+$/g, "");
}

/** 패키지 디렉터리 맨 위(와 REUSE 식 LICENSES/)의 라이선스·NOTICE 파일 내용 */
function licenseTexts(component) {
  const files = [...component.extraFiles];
  const add = (dir) => {
    for (const name of readdirSync(dir).sort()) {
      const path = join(dir, name);
      if (LICENSE_FILE.test(name) && !NOT_A_LICENSE.test(name) && statSync(path).isFile()) files.push(path);
    }
  };
  add(component.dir);
  const reuse = join(component.dir, "LICENSES");
  if (existsSync(reuse) && statSync(reuse).isDirectory()) {
    for (const name of readdirSync(reuse).sort()) files.push(join(reuse, name));
  }
  const texts = [];
  for (const path of new Set(files)) {
    const text = normalize(readFileSync(path, "utf8"));
    if (text && !texts.includes(text)) texts.push(text);
  }
  return texts;
}

/** 선언한 라이선스 식의 SPDX 이름들 */
function spdxIds(expression) {
  return (expression ?? "")
    .split(/\s+(?:OR|AND|WITH)\s+|[()/]/)
    .map((id) => id.trim())
    .filter(Boolean);
}

const MIT_BODY = `Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`;

/**
 * 라이선스 파일이 없는 패키지의 문구. 같은 저장소·같은 라이선스 표기인 다른 패키지의 파일, 없으면 선언한 라이선스
 * (MIT는 작성자로 저작권 줄을 만들고, `standard`의 라이선스는 그 문구). `A AND B`는 모두, `A OR B`는 하나라도
 * 채워야 한다. 못 채우면 null.
 */
function fallbackTexts(component, components, standard) {
  const sibling = components.find(
    (other) =>
      other !== component && other.url === component.url && other.license === component.license && other.texts.length,
  );
  if (sibling) return sibling.texts;
  const ids = spdxIds(component.license);
  const texts = [];
  let missing = 0;
  for (const id of ids) {
    if (id === "MIT") {
      const holders = component.authors.map((a) => a.replace(/\s*<[^>]*>/, "")).join(", ");
      texts.push(`MIT License\n\nCopyright (c) ${holders || `the ${component.name} authors`}\n\n${MIT_BODY}`);
    } else if (standard.has(id)) {
      texts.push(standard.get(id));
    } else {
      missing += 1;
    }
  }
  const all = / AND /.test(component.license ?? "");
  return texts.length > 0 && (!all || missing === 0) ? texts : null;
}

/** 저작권 줄이 없는 라이선스의 표준 문구: 그 라이선스 하나만 표기하고 파일이 하나인 패키지에서(Apache-2.0은 우리 것) */
function standardTexts(components, apacheText) {
  const standard = new Map([["Apache-2.0", apacheText]]);
  for (const component of components) {
    const ids = spdxIds(component.license);
    if (ids.length === 1 && STANDARD_TEXT.has(ids[0]) && component.texts.length === 1 && !standard.has(ids[0])) {
      standard.set(ids[0], component.texts[0]);
    }
  }
  return standard;
}

function main() {
  const own = ["LICENSE-MIT", "LICENSE-APACHE"].map((name) => normalize(readFileSync(join(root, name), "utf8")));
  const apacheText = own[1];
  const components = [
    ...crates().map((c) => ({ ...c, kind: "crate" })),
    ...npmPackages().map((c) => ({ ...c, kind: "npm" })),
  ];
  components.sort(
    (a, b) => a.kind.localeCompare(b.kind) || a.name.localeCompare(b.name) || a.version.localeCompare(b.version),
  );

  for (const component of components) component.texts = licenseTexts(component);
  const standard = standardTexts(components, apacheText);
  /** 문구 → 그 문구를 쓰는 구성 요소 */
  const groups = new Map();
  const filled = [];
  const failed = [];
  for (const component of components) {
    let texts = component.texts;
    if (texts.length === 0) {
      texts = fallbackTexts(component, components, standard);
      if (!texts) {
        failed.push(component);
        continue;
      }
      filled.push(component);
    }
    const line = `${component.kind === "crate" ? "Rust" : "npm"} ${component.name} ${component.version} (${
      component.license ?? "라이선스 표기 없음"
    }) ${component.url}`;
    for (const text of texts) {
      if (!groups.has(text)) groups.set(text, []);
      groups.get(text).push(line);
    }
  }
  if (failed.length > 0) {
    const names = failed.map((c) => `${c.name} ${c.version} (${c.license ?? "표기 없음"})`).join("\n  ");
    throw new Error(`라이선스 문구를 찾지 못한 구성 요소가 있다:\n  ${names}`);
  }

  const crateCount = components.filter((c) => c.kind === "crate").length;
  const npmCount = components.length - crateCount;
  const out = [
    "parallax 오픈 소스 라이선스",
    "",
    "parallax는 MIT 또는 Apache-2.0 가운데 하나를 골라 쓸 수 있다(아래 두 문구). 그 아래는 이 앱에 들어 있는",
    `다른 프로젝트의 저작권·라이선스 고지다: Rust 크레이트 ${crateCount}개, npm 패키지 ${npmCount}개.`,
    "같은 문구는 한 번만 싣고 그 문구를 쓰는 구성 요소를 위에 적었다.",
    "",
    RULE,
    "parallax",
    RULE,
    "",
    own[0],
    "",
    own[1],
    "",
  ];
  if (filled.length > 0) {
    out.push(
      RULE,
      "패키지에 라이선스 파일이 없어 같은 저장소의 다른 패키지 파일이나 선언한 라이선스로 문구를 채운 구성 요소:",
      ...filled.map((c) => `  ${c.name} ${c.version} (${c.license})`),
      "",
    );
  }
  const ordered = [...groups.entries()].sort(([, a], [, b]) => a[0].localeCompare(b[0]));
  for (const [text, users] of ordered) {
    out.push(RULE, ...users, RULE, "", text, "");
  }
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, out.join("\n"));
  const size = (statSync(output).size / 1024).toFixed(0);
  console.log(
    `third-party-notices: 크레이트 ${crateCount}개, npm ${npmCount}개, 문구 ${groups.size}개, ${size}KB` +
      (filled.length > 0 ? `, 선언한 라이선스로 채움 ${filled.length}개` : ""),
  );
}

main();

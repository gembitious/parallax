import { describe, expect, it } from "vitest";
import { indexStatus, withUntracked, type WorktreeStatus } from "./status";
import type { DirListing } from "./tree";

const status: WorktreeStatus = {
  head: "a".repeat(40),
  files: [
    { path: "README.md", kind: "modified" },
    { path: "src/new.ts", kind: "untracked" },
    { path: "src/old.ts", kind: "deleted" },
    { path: "src/gen/out/a.txt", kind: "untracked" },
    { path: "tools/x.sh", kind: "added" },
  ],
  untrackedOmitted: 0,
};

const listing = (dir: string, dirs: string[], files: string[]): DirListing => ({
  repo: "/r",
  dir,
  dirs,
  files: files.map((name) => ({ name, missing: false })),
});

describe("indexStatus", () => {
  const index = indexStatus(status);
  it("경로와 바뀐 파일이 든 폴더", () => {
    expect(index.files.get("src/old.ts")).toBe("deleted");
    expect(index.count).toBe(5);
    expect([...index.dirs].sort()).toEqual(["src", "src/gen", "src/gen/out", "tools"]);
  });
  it("추적하지 않는 파일과 그것만 든 폴더", () => {
    expect([...(index.untracked.get("src")?.files ?? [])]).toEqual(["new.ts"]);
    expect([...(index.untracked.get("src")?.dirs ?? [])]).toEqual(["gen"]);
    expect([...(index.untracked.get("")?.dirs ?? [])]).toEqual(["src"]);
    expect(index.untracked.has("tools")).toBe(false);
  });
});

describe("withUntracked", () => {
  const index = indexStatus(status);
  it("없는 것만 이름 순으로 더한다", () => {
    const merged = withUntracked(listing("src", ["lib"], ["main.ts", "old.ts"]), index);
    expect(merged.dirs).toEqual(["gen", "lib"]);
    expect(merged.files.map((f) => f.name)).toEqual(["main.ts", "new.ts", "old.ts"]);
  });
  it("이미 있으면 그대로", () => {
    const same = listing("", ["src", "tools"], ["README.md"]);
    expect(withUntracked(same, index)).toBe(same);
    const other = listing("docs", [], ["a.md"]);
    expect(withUntracked(other, index)).toBe(other);
  });
  it("인덱스에 없는 폴더(빈 목록)", () => {
    expect(withUntracked(listing("src/gen/out", [], []), index).files.map((f) => f.name)).toEqual(["a.txt"]);
  });
});

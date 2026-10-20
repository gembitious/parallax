import { describe, expect, it } from "vitest";
import { ancestorsOf, parentOf, sortListing, treeKey, treeRows, type DirListing } from "./tree";

const listing = (dir: string, dirs: string[], files: (string | [string, boolean])[]): DirListing =>
  sortListing({
    repo: "/repo",
    dir,
    dirs,
    files: files.map((file) => (typeof file === "string" ? { name: file, missing: false } : { name: file[0], missing: file[1] })),
  });

const listings = new Map([
  ["", listing("", ["src", "docs", "Zeta", "a10", "a2"], ["README.md", "package.json", "file10.txt", "file2.txt"])],
  ["src", listing("src", ["한글"], ["main.rs", ["gone.rs", true]])],
  ["src/한글", listing("src/한글", [], ["파일 이름.rs"])],
]);

describe("treeRows", () => {
  it("폴더 먼저, 이름은 숫자를 수로 보고 대소문자를 가리지 않는다", () => {
    const rows = treeRows(listings, new Set());
    expect(rows.map((r) => r.name)).toEqual([
      "a2",
      "a10",
      "docs",
      "src",
      "Zeta",
      "file2.txt",
      "file10.txt",
      "package.json",
      "README.md",
    ]);
    expect(rows.every((r) => r.depth === 0 && !r.open)).toBe(true);
  });

  it("펼친 폴더만 아래 행을 늘어놓고, 목록이 아직 없으면 읽는 중이다", () => {
    const rows = treeRows(listings, new Set(["src", "src/한글", "docs"]));
    expect(rows.map((r) => [r.path, r.depth, r.open, r.loading, r.missing])).toEqual([
      ["a2", 0, false, false, false],
      ["a10", 0, false, false, false],
      ["docs", 0, true, true, false],
      ["src", 0, true, false, false],
      ["src/한글", 1, true, false, false],
      ["src/한글/파일 이름.rs", 2, false, false, false],
      ["src/gone.rs", 1, false, false, true],
      ["src/main.rs", 1, false, false, false],
      ["Zeta", 0, false, false, false],
      ["file2.txt", 0, false, false, false],
      ["file10.txt", 0, false, false, false],
      ["package.json", 0, false, false, false],
      ["README.md", 0, false, false, false],
    ]);
    expect(treeRows(new Map(), new Set(["src"]))).toEqual([]);
  });
});

describe("경로", () => {
  it("부모 폴더와 담은 폴더들", () => {
    expect(parentOf("src/한글/파일 이름.rs")).toBe("src/한글");
    expect(parentOf("README.md")).toBe("");
    expect(ancestorsOf("src/한글/파일 이름.rs")).toEqual(["src", "src/한글"]);
    expect(ancestorsOf("README.md")).toEqual([]);
  });
});

describe("treeKey", () => {
  const rows = treeRows(listings, new Set(["src"]));
  const paths = rows.map((r) => r.path);

  it("위아래·페이지·처음·끝으로 옮긴다(끝에서 멈춘다)", () => {
    expect(treeKey(rows, null, "j")).toEqual({ kind: "select", path: "a2" });
    expect(treeKey(rows, null, "k")).toEqual({ kind: "select", path: "a2" });
    expect(treeKey(rows, "a2", "ArrowDown")).toEqual({ kind: "select", path: "a10" });
    expect(treeKey(rows, "a2", "k")).toBeNull();
    expect(treeKey(rows, "README.md", "j")).toBeNull();
    expect(treeKey(rows, "a2", "PageDown", 3)).toEqual({ kind: "select", path: paths[3] });
    expect(treeKey(rows, "a2", "End")).toEqual({ kind: "select", path: "README.md" });
    expect(treeKey(rows, "README.md", "Home")).toEqual({ kind: "select", path: "a2" });
  });

  it("→는 펼치고 펼친 폴더에서는 첫 자식으로, ←는 접고 그다음은 부모로", () => {
    expect(treeKey(rows, "docs", "ArrowRight")).toEqual({ kind: "toggle", path: "docs" });
    expect(treeKey(rows, "src", "l")).toEqual({ kind: "select", path: "src/한글" });
    expect(treeKey(rows, "src", "h")).toEqual({ kind: "toggle", path: "src" });
    expect(treeKey(rows, "src/main.rs", "ArrowLeft")).toEqual({ kind: "select", path: "src" });
    expect(treeKey(rows, "src/한글", "ArrowLeft")).toEqual({ kind: "select", path: "src" });
    expect(treeKey(rows, "README.md", "ArrowLeft")).toBeNull();
    expect(treeKey(rows, "README.md", "ArrowRight")).toBeNull();
    // 빈 폴더는 펼쳐도 갈 자식이 없다
    const empty = treeRows(new Map([["", listing("", ["빈"], [])], ["빈", listing("빈", [], [])]]), new Set(["빈"]));
    expect(treeKey(empty, "빈", "ArrowRight")).toBeNull();
  });

  it("Enter는 파일을 열고 폴더를 펼치거나 접는다, Space는 미리보기", () => {
    expect(treeKey(rows, "src/main.rs", "Enter")).toEqual({ kind: "open", path: "src/main.rs", missing: false, preview: false });
    expect(treeKey(rows, "src/gone.rs", " ")).toEqual({ kind: "open", path: "src/gone.rs", missing: true, preview: true });
    expect(treeKey(rows, "docs", "Enter")).toEqual({ kind: "toggle", path: "docs" });
    expect(treeKey(rows, "docs", " ")).toBeNull();
    expect(treeKey(rows, null, "Enter")).toBeNull();
    expect(treeKey(rows, "a2", "x")).toBeNull();
  });
});

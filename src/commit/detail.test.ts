import { describe, expect, it } from "vitest";
import { STATS_BATCH_MIN, formatBytes, modeNote, remember, sameNote, statBatches, versionsSize, withStats } from "./detail";
import type { CommitDetail, FileChange, FileStat, FileVersions } from "./types";

const stat = (additions: number, deletions: number): FileStat => ({ binary: false, missing: false, additions, deletions });

const file = (path: string, s: FileStat | null): FileChange => ({
  status: "modified",
  path,
  oldPath: null,
  pathLossy: false,
  oldId: "a",
  newId: "b",
  oldMode: "100644",
  newMode: "100644",
  submodule: false,
  stat: s,
});

const detail = (files: FileChange[]): CommitDetail => ({
  info: {} as CommitDetail["info"],
  files,
  additions: 0,
  deletions: 0,
  counted: false,
  renamesIncomplete: false,
  renamesLimited: false,
});

describe("statBatches", () => {
  it("세지 않은 파일만 목록 순서대로 묶는다", () => {
    const files = [file("a", stat(1, 0)), file("b", null), file("c", stat(0, 1)), file("d", null)];
    expect(statBatches(files)).toEqual([[1, 3]]);
    expect(statBatches([file("a", stat(1, 1))])).toEqual([]);
  });

  it("작은 커밋은 STATS_BATCH_MIN씩, 큰 커밋은 갱신이 20번 안팎이 되게 키운다", () => {
    const small = Array.from({ length: 450 }, (_, i) => file(`f${i}`, null));
    expect(statBatches(small).map((b) => b.length)).toEqual([STATS_BATCH_MIN, STATS_BATCH_MIN, 50]);
    const big = Array.from({ length: 50_000 }, (_, i) => file(`f${i}`, null));
    const batches = statBatches(big);
    expect(batches.length).toBe(20);
    expect(batches.flat()).toEqual(big.map((_, i) => i));
  });
});

describe("withStats", () => {
  it("받은 줄 수를 넣고 합계와 다 셌는지를 다시 낸다", () => {
    const a = file("a", stat(1, 2));
    const d = detail([a, file("b", null), file("c", null)]);
    const half = withStats(d, [1], [stat(10, 20)]);
    expect(half.files[0]).toBe(a);
    expect(half.files[1].stat).toEqual(stat(10, 20));
    expect([half.additions, half.deletions, half.counted]).toEqual([11, 22, false]);
    const all = withStats(half, [2], [stat(100, 0)]);
    expect([all.additions, all.deletions, all.counted]).toEqual([111, 22, true]);
    // 원래 상세는 바꾸지 않는다.
    expect(d.files[1].stat).toBeNull();
  });
});

describe("modeNote", () => {
  it("실행 권한과 종류 바뀜을 설명하고, 같거나 한쪽이 없으면 없다", () => {
    expect(modeNote({ oldMode: "100644", newMode: "100755" })).toBe("실행 권한을 켰다 (100644 → 100755)");
    expect(modeNote({ oldMode: "100755", newMode: "100644" })).toBe("실행 권한을 껐다 (100755 → 100644)");
    expect(modeNote({ oldMode: "100644", newMode: "120000" })).toBe("종류: 파일 → 심볼릭 링크 (100644 → 120000)");
    expect(modeNote({ oldMode: "100644", newMode: "100644" })).toBeNull();
    expect(modeNote({ oldMode: null, newMode: "100755" })).toBeNull();
  });
});

describe("sameNote", () => {
  const same = (old: string, now: string) => ({ old, new: now });

  it("빈 파일을 더하거나 지웠다", () => {
    expect(sameNote("added", same("", ""), same("", ""), false)).toBe("빈 파일");
    expect(sameNote("deleted", same("", ""), same("", ""), false)).toBe("빈 파일");
    // 빈 줄 하나는 빈 파일이 아니다(화면에는 빈 줄로 보인다).
    expect(sameNote("added", same("", "\n"), same("", ""), false)).toBeNull();
  });

  it("이름·권한만 바뀌었거나 보이지 않는 바이트만 바뀌었다", () => {
    expect(sameNote("renamed", same("a\n", "a\n"), same("a", "a"), false)).toBe("내용은 그대로다");
    expect(sameNote("modified", same("a\n", "a\r\n"), same("a", "a"), false)).toBe("보이는 글자는 같다 (바이트가 다르다)");
    // 줄 끝 알림이 이미 설명한다.
    expect(sameNote("modified", same("a\n", "a\r\n"), same("a", "a"), true)).toBeNull();
    expect(sameNote("modified", same("a", "b"), same("a", "b"), false)).toBeNull();
  });
});

describe("formatBytes", () => {
  it("단위를 붙인다", () => {
    expect(formatBytes(512)).toBe("512B");
    expect(formatBytes(1536)).toBe("1.5KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0MB");
  });
});

describe("remember", () => {
  const size = (v: number) => v;

  it("한도를 넘으면 오래된 것부터 잊고, 다시 넣은 것은 최근으로 옮긴다", () => {
    let m = new Map<string, number>();
    for (const k of ["a", "b", "c"]) m = remember(m, k, 1, size, { entries: 3, chars: 100 }, null);
    m = remember(m, "a", 1, size, { entries: 3, chars: 100 }, null);
    m = remember(m, "d", 1, size, { entries: 3, chars: 100 }, null);
    expect([...m.keys()]).toEqual(["c", "a", "d"]);
  });

  it("글자 수 한도도 본다. 넣은 것과 지금 고른 것은 잊지 않는다", () => {
    let m = new Map<string, number>([
      ["old", 40],
      ["shown", 40],
    ]);
    m = remember(m, "late", 40, size, { entries: 10, chars: 50 }, "shown");
    expect([...m.keys()]).toEqual(["shown", "late"]);
    // 혼자서 한도를 넘는 것도 남는다(보고 있는 파일이다).
    m = remember(new Map(), "huge", 500, size, { entries: 10, chars: 50 }, null);
    expect([...m.keys()]).toEqual(["huge"]);
  });

  it("받는 중·실패는 크기가 없다", () => {
    const versions: FileVersions = {
      old: { text: "ab", encoding: "UTF-8", lineCount: 1 },
      new: { text: "abc", encoding: "UTF-8", lineCount: 1 },
      binary: false,
      missing: false,
      tooLarge: null,
      notes: [],
    };
    expect(versionsSize(versions)).toBe(5);
    expect(versionsSize("loading")).toBe(0);
    expect(versionsSize({ error: "x" })).toBe(0);
  });
});

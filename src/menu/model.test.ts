import { describe, expect, it } from "vitest";
import type { Command } from "../palette/Palette";
import { buildMenus, nativeLabel, shortcut, tidy, toNative, type BarItem, type BarMenu, type MenuSources } from "./model";

const noop = () => {};

const sources = (more: Partial<MenuSources> = {}): MenuSources => ({
  mac: false,
  workspace: [
    { id: "tree", label: "파일 트리 숨기기", keys: "Ctrl+B", accelerator: "CmdOrCtrl+B", app: true, run: noop },
    { id: "panel", label: "아래 패널 열기", keys: "Ctrl+J", accelerator: "CmdOrCtrl+J", app: true, run: noop },
    { id: "closeTab", label: "탭 닫기", keys: "Ctrl+W", accelerator: "CmdOrCtrl+W", app: true, run: noop },
  ],
  handles: { palette: noop, help: noop, files: noop, graph: noop, closeTab: noop },
  view: null,
  recent: [],
  tabs: true,
  gutterShown: true,
  hud: false,
  hideWindow: noop,
  actions: {
    openRepo: noop,
    openRecent: noop,
    installCli: noop,
    notices: noop,
    quit: noop,
    toggleGutter: noop,
    hud: noop,
  },
  ...more,
});

const labels = (items: BarItem[]): string[] =>
  items.map((item) => (item.kind === "separator" ? "-" : item.kind === "submenu" ? `${item.label} ▸` : item.label));

const menu = (menus: BarMenu[], title: string) => menus.find((m) => m.title === title)!;

const blameView = (run: (command: Command) => void = noop) => ({
  title: "blame",
  run,
  commands: [
    { id: "commit", label: "이 줄 커밋의 상세", keys: "Enter", run: noop },
    { label: "커밋 sha 복사", keys: "y", run: noop },
    { id: "gutter", label: "blame 거터 숨기기", keys: "b", run: noop },
    { label: "파일 찾기 (Quick Open)", keys: "f · Ctrl+P", app: true, run: noop },
    { id: "worktrees", label: "워크트리 고르기", keys: "W", run: noop },
    { id: "help", label: "단축키 도움말", keys: "?", run: noop },
    { id: "back", label: "뒤로 (맨 앞 화면이면 창 숨기기)", keys: "q", run: noop },
  ] satisfies Command[],
});

describe("shortcut", () => {
  it("macOS는 기호, 나머지는 Ctrl+", () => {
    expect(shortcut("CmdOrCtrl+Shift+P", true)).toBe("⌘⇧P");
    expect(shortcut("CmdOrCtrl+Shift+P", false)).toBe("Ctrl+Shift+P");
    expect(shortcut("CmdOrCtrl+O", false)).toBe("Ctrl+O");
  });
});

describe("buildMenus", () => {
  it("Windows·Linux: 파일 · 보기 · 화면 · 도움말", () => {
    const menus = buildMenus(sources({ view: blameView() }));
    expect(menus.map((m) => m.title)).toEqual(["파일", "보기", "blame", "도움말"]);
    expect(labels(menu(menus, "파일").items)).toEqual([
      "저장소 열기…",
      "최근 저장소 ▸",
      "-",
      "파일 찾기 (Quick Open)",
      "-",
      "탭 닫기",
      "-",
      "터미널 명령 plx 설치…",
      "-",
      "종료",
    ]);
    expect(labels(menu(menus, "보기").items)).toEqual([
      "파일 트리 숨기기",
      "아래 패널 열기",
      "-",
      "blame 거터 숨기기",
      "커밋 그래프 (아래 패널)",
      "-",
      "명령 팔레트",
    ]);
  });

  it("보기: 에디터 나누기·탭 옮기기·그룹으로는 에디터 레이아웃 하위 메뉴에", () => {
    const layout = [
      { id: "split", label: "에디터 나누기", keys: "Ctrl+\\", accelerator: "CmdOrCtrl+\\", app: true, run: noop },
      { id: "tabToPanel", label: "탭을 아래 패널로", app: true, run: noop },
      { id: "group1", label: "에디터 그룹 1로", keys: "Ctrl+1", accelerator: "CmdOrCtrl+1", app: true, run: noop },
      { id: "group2", label: "에디터 그룹 2로", keys: "Ctrl+2", accelerator: "CmdOrCtrl+2", app: true, run: noop },
    ] satisfies Command[];
    const menus = buildMenus(sources({ workspace: [...sources().workspace, ...layout] }));
    const view = menu(menus, "보기").items;
    expect(labels(view).slice(0, 3)).toEqual(["파일 트리 숨기기", "아래 패널 열기", "에디터 레이아웃 ▸"]);
    const submenu = view[2] as Extract<BarItem, { kind: "submenu" }>;
    expect(labels(submenu.items)).toEqual(["에디터 나누기", "탭을 아래 패널로", "-", "에디터 그룹 1로", "에디터 그룹 2로"]);
    expect(submenu.items[0]).toMatchObject({ keys: "Ctrl+\\", accelerator: "CmdOrCtrl+\\" });
    expect(shortcut("CmdOrCtrl+\\", true)).toBe("⌘\\");
    // 나눌 탭이 없으면(첫 화면·빈 에디터) 하위 메뉴가 없다
    expect(labels(menu(buildMenus(sources()), "보기").items)).not.toContain("에디터 레이아웃 ▸");
  });

  it("macOS: 앱 메뉴·편집·윈도우가 붙고 종료·설치는 앱 메뉴에", () => {
    const menus = buildMenus(sources({ mac: true, view: blameView() }));
    expect(menus.map((m) => m.title)).toEqual(["parallax", "파일", "편집", "보기", "blame", "윈도우", "도움말"]);
    expect(menu(menus, "윈도우").role).toBe("window");
    expect(menu(menus, "도움말").role).toBe("help");
    expect(labels(menu(menus, "파일").items)).not.toContain("종료");
    expect(menu(menus, "파일").items[0]).toMatchObject({ keys: "⌘O", accelerator: "CmdOrCtrl+O" });
  });

  it("화면 메뉴: 앱 전역 명령과 제자리(보기·도움말)가 있는 것은 빼고, 어느 화면에나 있는 것은 끝에 묶는다", () => {
    const ran: string[] = [];
    const menus = buildMenus(sources({ view: blameView((command) => ran.push(command.label)) }));
    const view = menu(menus, "blame");
    expect(labels(view.items)).toEqual(["이 줄 커밋의 상세", "커밋 sha 복사", "-", "워크트리 고르기", "뒤로 (맨 앞 화면이면 창 숨기기)"]);
    // 화면의 명령은 그 칸에 포커스를 주는 run으로 실행한다
    const first = view.items[0];
    if (first.kind === "item") first.run();
    expect(ran).toEqual(["이 줄 커밋의 상세"]);
  });

  it("최근 저장소와 탭이 없을 때", () => {
    let hidden = 0;
    const menus = buildMenus(
      sources({
        recent: [{ name: "parallax", display: "~/work/parallax", path: "/home/me/work/parallax" }],
        tabs: false,
        hideWindow: () => (hidden += 1),
      }),
    );
    const recent = menu(menus, "파일").items[1];
    expect(recent).toMatchObject({ kind: "submenu", label: "최근 저장소" });
    if (recent.kind === "submenu") expect(recent.items[0]).toMatchObject({ label: "parallax", detail: "~/work/parallax" });
    expect(labels(menu(menus, "파일").items)).toContain("창 닫기");
    expect(buildMenus(sources()).map((m) => m.title)).toEqual(["파일", "보기", "도움말"]);
    const none = buildMenus(sources({ recent: [] }));
    const empty = menu(none, "파일").items[1];
    if (empty.kind === "submenu") expect(empty.items).toMatchObject([{ label: "없음", disabled: true }]);
    // 첫 화면(작업 공간이 없다): 창 닫기는 창을 숨긴다
    const start = buildMenus(sources({ workspace: [], handles: null, tabs: false, hideWindow: () => (hidden += 1) }));
    expect(start.map((m) => m.title)).toEqual(["파일", "도움말"]);
    const close = menu(start, "파일").items.find((item) => item.kind === "item" && item.label === "창 닫기");
    if (close?.kind === "item") close.run();
    expect(hidden).toBe(1);
  });
});

describe("tidy", () => {
  it("앞뒤·겹친 구분선을 뺀다", () => {
    const sep: BarItem = { kind: "separator" };
    const a: BarItem = { kind: "item", label: "a", run: noop };
    expect(labels(tidy([sep, a, sep, sep, a, sep]))).toEqual(["a", "-", "a"]);
  });
});

describe("toNative", () => {
  it("시스템 메뉴: 시스템 단축키가 아닌 키는 이름 뒤에, id는 메뉴·항목 이름", () => {
    expect(nativeLabel("파일 찾기 (Quick Open)", "f · ⌘P", "CmdOrCtrl+P")).toBe("파일 찾기 (Quick Open) (f)");
    expect(nativeLabel("명령 팔레트", ": · ⌘⇧P", "CmdOrCtrl+Shift+P")).toBe("명령 팔레트 (:)");
    expect(nativeLabel("탭 닫기", "⌘W", "CmdOrCtrl+W")).toBe("탭 닫기");
    expect(nativeLabel("이 줄 커밋의 상세", "Enter", undefined)).toBe("이 줄 커밋의 상세 (Enter)");

    let opened = "";
    const menus = buildMenus(
      sources({
        mac: true,
        recent: [{ name: "parallax", display: "~/work/parallax", path: "/p" }],
        actions: { ...sources().actions, openRecent: (path) => (opened = path) },
      }),
    );
    const native = toNative(menus);
    const file = native.menus.find((m) => m.title === "파일")!;
    expect(file.items[0]).toEqual({
      kind: "item",
      id: "파일/저장소 열기…",
      label: "저장소 열기…",
      accelerator: "CmdOrCtrl+O",
      enabled: true,
    });
    const recent = file.items[1];
    expect(recent.kind).toBe("submenu");
    if (recent.kind !== "submenu") return;
    // 같은 이름의 저장소가 여럿이어도 경로로 나뉜다. 시스템 메뉴에는 경로를 보인다.
    expect(recent.items[0]).toMatchObject({ id: "파일/최근 저장소/parallax\t~/work/parallax", label: "~/work/parallax" });
    native.actions.get("파일/최근 저장소/parallax\t~/work/parallax")?.();
    expect(opened).toBe("/p");
    expect(native.menus.find((m) => m.title === "편집")!.items[0]).toEqual({ kind: "predefined", name: "undo", label: "실행 취소" });
    // 꺼진 항목은 실행하지 않는다
    const empty = toNative(buildMenus(sources({ mac: true })));
    const none = empty.menus.find((m) => m.title === "파일")!.items[1];
    if (none.kind === "submenu") expect(none.items[0]).toMatchObject({ enabled: false });
    expect(empty.actions.has("파일/최근 저장소/없음")).toBe(false);
  });
});

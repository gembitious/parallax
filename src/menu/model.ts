// 메뉴 막대 모형: 파일 · 보기 · <키를 받는 화면> · 도움말(macOS는 앱 메뉴·편집·윈도우도). 모든 동작과 단축키를 둔다.
// macOS는 이 모형으로 시스템 메뉴 막대를 만들고(toNative → src-tauri/src/menu.rs), Windows·Linux는 도구 막대 왼쪽에
// HTML로 그린다(src/workspace/Toolbar.tsx). 순수 함수라 단위 테스트한다(model.test.ts).
import type { Command, CommandId } from "../palette/Palette";

/** 시스템이 하는 항목(macOS). 이름은 macOS 한국어 메뉴를 따른다. */
export type Predefined =
  | "about"
  | "services"
  | "hide"
  | "hideOthers"
  | "showAll"
  | "quit"
  | "undo"
  | "redo"
  | "cut"
  | "copy"
  | "paste"
  | "selectAll"
  | "minimize"
  | "zoom"
  | "fullscreen"
  | "bringAllToFront";

export type BarItem =
  | {
      kind: "item";
      label: string;
      keys?: string;
      /** macOS 시스템 단축키(⌘가 붙은 것만) */
      accelerator?: string;
      /** 이름 옆에 흐리게(HTML 메뉴만) */
      detail?: string;
      disabled?: boolean;
      run(): void;
    }
  | { kind: "separator" }
  | { kind: "submenu"; label: string; items: BarItem[] }
  | { kind: "predefined"; name: Predefined; label: string };

export type BarMenu = {
  title: string;
  /** macOS: 창 목록이 붙는 윈도우 메뉴, 검색 칸이 붙는 도움말 메뉴 */
  role?: "window" | "help";
  items: BarItem[];
};

export type MenuActions = {
  openRepo(): void;
  openRecent(path: string): void;
  installCli(): void;
  notices(): void;
  quit(): void;
  toggleGutter(): void;
  hud(): void;
};

/** 키를 받는 화면(없으면 작업 공간)의 창. 첫 화면에는 없다. */
export type MenuHandles = {
  palette(): void;
  help(): void;
  files(): void;
  /** 저장소 그래프(아래 패널) */
  graph(): void;
  /** 탭 닫기, 탭이 없으면 창 숨기기 */
  closeTab(): void;
};

export type MenuSources = {
  mac: boolean;
  /** 작업 공간 명령(트리·패널·탭, id가 붙은 것). 첫 화면이면 비었다. */
  workspace: Command[];
  handles: MenuHandles | null;
  /** 키를 받는 화면: 메뉴 이름과 명령(앱 전역 명령은 빼고 그린다). 명령은 `run`으로 실행한다(그 칸에 포커스를 준다). */
  view: { title: string; commands: Command[]; run(command: Command): void } | null;
  /** 최근 저장소(없어진 폴더는 뺀 것) */
  recent: { name: string; display: string; path: string }[];
  /** 탭이 있다(⌘W·Ctrl+W가 탭을 닫는다. 없으면 창을 숨긴다) */
  tabs: boolean;
  gutterShown: boolean;
  /** 성능 HUD를 쓸 수 있다(개발 빌드, PLX_PERF=1) */
  hud: boolean;
  /** 창을 숨긴다(첫 화면의 ⌘W) */
  hideWindow(): void;
  actions: MenuActions;
};

const SEPARATOR: BarItem = { kind: "separator" };

/** 시스템 단축키 → 보이는 단축키(macOS ⌘⇧P, 나머지 Ctrl+Shift+P) */
export function shortcut(accelerator: string, mac: boolean): string {
  const parts = accelerator.split("+");
  const key = parts.pop() ?? "";
  if (!mac) return [...parts.map((p) => (p === "CmdOrCtrl" ? "Ctrl" : p)), key].join("+");
  const glyph: Record<string, string> = { CmdOrCtrl: "⌘", Cmd: "⌘", Shift: "⇧", Alt: "⌥", Ctrl: "⌃" };
  return parts.map((p) => glyph[p] ?? p).join("") + key;
}

const item = (label: string, run: () => void, more: Partial<Extract<BarItem, { kind: "item" }>> = {}): BarItem => ({
  kind: "item",
  label,
  run,
  ...more,
});

const predefined = (name: Predefined, label: string): BarItem => ({ kind: "predefined", name, label });

const fromCommand = (command: Command | undefined, run?: () => void): BarItem[] =>
  command ? [item(command.label, run ?? command.run, { keys: command.keys, accelerator: command.accelerator })] : [];

/** 앞뒤·겹친 구분선을 뺀다. */
export function tidy(items: BarItem[]): BarItem[] {
  const out: BarItem[] = [];
  for (const entry of items) {
    if (entry.kind === "separator" && (out.length === 0 || out[out.length - 1].kind === "separator")) continue;
    out.push(entry);
  }
  while (out.length > 0 && out[out.length - 1].kind === "separator") out.pop();
  return out;
}

/** 화면 메뉴 끝에 묶는 것(어느 화면에나 있는 것) */
const TAIL: ReadonlySet<CommandId> = new Set(["worktrees", "back"]);
/** 화면 메뉴에 넣지 않는 것: 제자리(보기·도움말 메뉴)에 있다 */
const ELSEWHERE: ReadonlySet<CommandId> = new Set(["gutter", "help"]);

export function buildMenus(src: MenuSources): BarMenu[] {
  const { mac, actions, handles } = src;
  const ws = (id: CommandId) => src.workspace.find((command) => command.id === id);
  const keysOf = (accelerator: string, plain?: string) => [plain, shortcut(accelerator, mac)].filter(Boolean).join(" · ");
  const menus: BarMenu[] = [];

  if (mac) {
    menus.push({
      title: "parallax",
      items: [
        predefined("about", "parallax에 관하여"),
        SEPARATOR,
        item("터미널 명령 plx 설치…", actions.installCli),
        SEPARATOR,
        predefined("services", "서비스"),
        SEPARATOR,
        predefined("hide", "parallax 가리기"),
        predefined("hideOthers", "기타 가리기"),
        predefined("showAll", "모두 보기"),
        SEPARATOR,
        predefined("quit", "parallax 종료"),
      ],
    });
  }

  const recent: BarItem[] =
    src.recent.length > 0
      ? src.recent.map((repo) => item(repo.name, () => actions.openRecent(repo.path), { detail: repo.display }))
      : [item("없음", () => {}, { disabled: true })];
  menus.push({
    title: "파일",
    items: tidy([
      item("저장소 열기…", actions.openRepo, { keys: keysOf("CmdOrCtrl+O"), accelerator: "CmdOrCtrl+O" }),
      { kind: "submenu", label: "최근 저장소", items: recent },
      SEPARATOR,
      ...(handles ? [item("파일 찾기 (Quick Open)", handles.files, { keys: keysOf("CmdOrCtrl+P", "f"), accelerator: "CmdOrCtrl+P" })] : []),
      SEPARATOR,
      item(src.tabs ? "탭 닫기" : "창 닫기", handles ? handles.closeTab : src.hideWindow, {
        keys: keysOf("CmdOrCtrl+W"),
        accelerator: "CmdOrCtrl+W",
      }),
      ...(mac ? [] : [SEPARATOR, item("터미널 명령 plx 설치…", actions.installCli), SEPARATOR, item("종료", actions.quit)]),
    ]),
  });

  if (mac) {
    menus.push({
      title: "편집",
      items: [
        predefined("undo", "실행 취소"),
        predefined("redo", "실행 복귀"),
        SEPARATOR,
        predefined("cut", "오려두기"),
        predefined("copy", "복사하기"),
        predefined("paste", "붙이기"),
        predefined("selectAll", "모두 선택"),
      ],
    });
  }

  // 에디터 나누기·탭 옮기기(탭 ↔ 패널)·그룹으로
  const layout = tidy([
    ...fromCommand(ws("split")),
    ...fromCommand(ws("tabLeft")),
    ...fromCommand(ws("tabRight")),
    ...fromCommand(ws("tabToPanel")),
    ...fromCommand(ws("panelToEditor")),
    SEPARATOR,
    ...fromCommand(ws("group1")),
    ...fromCommand(ws("group2")),
    ...fromCommand(ws("group3")),
    ...fromCommand(ws("group4")),
  ]);
  const view = tidy([
    ...fromCommand(ws("tree")),
    ...fromCommand(ws("treeFocus")),
    ...fromCommand(ws("panel")),
    ...fromCommand(ws("panelMax")),
    ...(layout.length > 0 ? [{ kind: "submenu" as const, label: "에디터 레이아웃", items: layout }] : []),
    SEPARATOR,
    ...(handles
      ? [
          item(src.gutterShown ? "blame 거터 숨기기" : "blame 거터 보이기", actions.toggleGutter, { keys: "b" }),
          item("커밋 그래프 (아래 패널)", handles.graph, { keys: "G" }),
        ]
      : []),
    ...fromCommand(ws("changes")),
    SEPARATOR,
    ...fromCommand(ws("nextTab")),
    ...fromCommand(ws("previousTab")),
    SEPARATOR,
    ...(handles
      ? [item("명령 팔레트", handles.palette, { keys: keysOf("CmdOrCtrl+Shift+P", ":"), accelerator: "CmdOrCtrl+Shift+P" })]
      : []),
    ...(src.hud ? [item("성능 HUD 보이기·숨기기", actions.hud, { keys: "`" })] : []),
    ...(mac ? [SEPARATOR, predefined("fullscreen", "전체 화면 시작")] : []),
  ]);
  if (view.length > 0) menus.push({ title: "보기", items: view });

  if (src.view) {
    const { run } = src.view;
    const items: BarItem[] = [];
    let tail = false;
    for (const command of src.view.commands) {
      if (command.app || (command.id !== undefined && ELSEWHERE.has(command.id))) continue;
      if (command.id !== undefined && TAIL.has(command.id) && !tail) {
        items.push(SEPARATOR);
        tail = true;
      }
      items.push(item(command.label, () => run(command), { keys: command.keys, accelerator: command.accelerator }));
    }
    if (items.length > 0) menus.push({ title: src.view.title, items: tidy(items) });
  }

  if (mac) {
    menus.push({
      title: "윈도우",
      role: "window",
      items: [predefined("minimize", "최소화"), predefined("zoom", "확대/축소"), SEPARATOR, predefined("bringAllToFront", "모두 앞으로 가져오기")],
    });
  }

  menus.push({
    title: "도움말",
    role: "help",
    items: [
      ...(handles ? [item("단축키 도움말", handles.help, { keys: "?" })] : []),
      item("오픈 소스 라이선스", actions.notices),
    ],
  });
  return menus;
}

export type NativeItem =
  | { kind: "item"; id: string; label: string; accelerator?: string; enabled: boolean }
  | { kind: "separator" }
  | { kind: "submenu"; label: string; items: NativeItem[] }
  | { kind: "predefined"; name: Predefined; label: string };

export type NativeMenu = { title: string; role?: "window" | "help"; items: NativeItem[] };

/**
 * macOS 시스템 메뉴의 이름. 시스템 단축키로 넣지 않는 키(글자 하나 등)는 이름 뒤에 쓴다: 시스템에 넣으면 입력란에 친
 * 글자까지 메뉴가 가로챈다.
 */
export function nativeLabel(label: string, keys: string | undefined, accelerator: string | undefined): string {
  if (!keys) return label;
  const shown = accelerator ? shortcut(accelerator, true) : null;
  const rest = keys.split(" · ").filter((key) => key !== shown);
  return rest.length > 0 ? `${label} (${rest.join(", ")})` : label;
}

/** 시스템 메뉴 모형과 항목 id → 실행. id는 메뉴 이름과 항목 이름이라 모형을 다시 만들어도 같은 항목은 같다. */
export function toNative(menus: BarMenu[]): { menus: NativeMenu[]; actions: Map<string, () => void> } {
  const actions = new Map<string, () => void>();
  const convert = (items: BarItem[], path: string): NativeItem[] =>
    items.map((entry): NativeItem => {
      switch (entry.kind) {
        case "item": {
          const id = `${path}/${entry.label}${entry.detail ? `\t${entry.detail}` : ""}`;
          if (!entry.disabled) actions.set(id, entry.run);
          return {
            kind: "item",
            id,
            label: entry.detail ?? nativeLabel(entry.label, entry.keys, entry.accelerator),
            accelerator: entry.accelerator,
            enabled: !entry.disabled,
          };
        }
        case "submenu":
          return { kind: "submenu", label: entry.label, items: convert(entry.items, `${path}/${entry.label}`) };
        default:
          return entry;
      }
    });
  return {
    menus: menus.map((menu) => ({ title: menu.title, role: menu.role, items: convert(menu.items, menu.title) })),
    actions,
  };
}

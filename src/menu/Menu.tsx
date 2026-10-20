// 우클릭 메뉴와 메뉴 막대(Windows·Linux)의 드롭다운. 한 번에 하나만 뜨고 App이 `<MenuHost/>`로 그린다.
// 메뉴가 키를 모두 받는다(↑↓ Home End 고르기, Enter·Space 실행, Esc 닫기, 메뉴 막대는 ←→로 옆 메뉴): 화면들과 작업 공간은
// window에서 키를 받으므로 그보다 먼저(창의 캡처 단계) 받아 전파를 막는다. 닫으면 연 때 포커스가 있던 곳(코드 뷰·목록)으로
// 돌려주고, 고른 것은 그다음 실행한다.
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { EditorView } from "@codemirror/view";
import { copyText } from "../clipboard";
import { isComposing } from "../keys";
import { MOD } from "../workspace/keys";
import { MENU_MARGIN, edgeIndex, placeMenu, stepIndex, type Box, type Point } from "./nav";

export type MenuItem = {
  label: string;
  /** 단축키(보이기만 한다) */
  keys?: string;
  /** 이름 옆에 흐리게(경로 등) */
  detail?: string;
  /** 지금은 할 수 없다(흐리게, 고를 수 없다) */
  disabled?: boolean;
  run(): void;
};

/** 메뉴 한 줄: 항목, 구분선, 머리말(묶음 이름) */
export type MenuEntry = MenuItem | "separator" | { header: string };

type OpenMenu = {
  entries: MenuEntry[];
  at: Point | Box;
  /** 메뉴 막대의 드롭다운(`index`번째 제목): ←→로 옆 메뉴를 연다 */
  bar?: { index: number; move(delta: 1 | -1): void };
  /** 첫 항목을 골라 둔다(키로 연 메뉴 막대) */
  select: boolean;
  /** 화면 읽기 프로그램에 알릴 이름 */
  label: string;
  /** 열 때 포커스가 있던 곳. 닫으면 돌려준다. */
  restore: HTMLElement | null;
  serial: number;
};

let current: OpenMenu | null = null;
let serial = 0;
const listeners = new Set<() => void>();
/** 떠 있는 메뉴의 키 처리(MenuPopup이 넣는다) */
let keyHandler: ((e: KeyboardEvent) => void) | null = null;

// 메뉴가 떠 있는 동안 키는 포커스가 어디 있든 메뉴의 것이다. 이 모듈을 불러올 때 등록해 화면·작업 공간의 리스너보다 먼저,
// 창의 캡처 단계에서 맨 먼저 받는다. 메뉴의 포커스에 기대면 포커스가 메뉴에 가지 않았을 때 Esc가 메뉴를 닫지 않고 j가
// 아래 코드 뷰의 커서를 옮긴다(메뉴가 숨은 채 포커스를 청하던 때 그랬다).
window.addEventListener("keydown", (e) => keyHandler?.(e), true);

function set(next: OpenMenu | null) {
  current = next;
  for (const listener of listeners) listener();
}

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

const isItem = (entry: MenuEntry): entry is MenuItem => typeof entry === "object" && "run" in entry;

/** 메뉴를 연다(떠 있던 것은 닫는다). 메뉴 막대가 바꿔 열 때는 처음 연 때의 포커스를 이어받는다. */
export function openMenu(
  at: Point | Box,
  entries: MenuEntry[],
  options: { label?: string; bar?: OpenMenu["bar"]; select?: boolean } = {},
) {
  if (!entries.some(isItem)) return;
  const active = document.activeElement instanceof HTMLElement && !document.activeElement.closest(".menu") ? document.activeElement : null;
  serial += 1;
  set({
    entries,
    at,
    bar: options.bar,
    select: options.select ?? false,
    label: options.label ?? "메뉴",
    restore: current?.restore ?? active,
    serial,
  });
}

/** 우클릭 메뉴: 기본 메뉴(새로 고침·검사)를 막고 마우스 자리에 연다. */
export function openContextMenu(e: Pick<MouseEvent, "clientX" | "clientY" | "preventDefault">, entries: MenuEntry[], label = "메뉴") {
  e.preventDefault();
  openMenu({ x: e.clientX, y: e.clientY }, entries, { label });
}

/** 닫는다. `restore`면 연 때의 포커스를 돌려준다. */
export function closeMenu(restore = true) {
  const open = current;
  if (!open) return;
  set(null);
  if (restore && open.restore?.isConnected) open.restore.focus({ preventScroll: true });
}

/** 메뉴 막대: 드롭다운이 떠 있다(제목에 마우스를 올리면 그 메뉴로 바꾼다) */
export const isBarMenuOpen = () => current?.bar !== undefined;

/** 떠 있는 메뉴(메뉴 막대가 열린 제목을 칠한다) */
export function useOpenMenu(): Readonly<Pick<OpenMenu, "bar">> | null {
  return useSyncExternalStore(subscribe, () => current);
}

/** 우클릭 메뉴의 복사: 코드 뷰는 그 선택(화면 밖 줄 포함), 아니면 문서에서 고른 글 */
export function selectedText(target: EventTarget | null): string {
  const el = target instanceof Element ? target.closest(".cm-editor") : null;
  const view = el instanceof HTMLElement ? EditorView.findFromDOM(el) : null;
  if (view) {
    const { from, to } = view.state.selection.main;
    return view.state.sliceDoc(from, to);
  }
  return window.getSelection()?.toString() ?? "";
}

/** 고른 글 복사 항목(⌘C·Ctrl+C) */
export const copyItem = (text: string): MenuItem => ({
  label: "복사",
  keys: `${MOD}C`,
  disabled: text === "",
  run: () => void copyText(text).catch((e: unknown) => console.error("복사하지 못했다", e)),
});

export function MenuHost() {
  const open = useSyncExternalStore(subscribe, () => current);

  // 화면이 처리하지 않은 우클릭: 입력란은 기본 메뉴(붙여넣기 등), 나머지는 기본 메뉴(새로 고침·검사)를 막고 고른 글이
  // 있으면 복사만 보인다.
  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      if (e.defaultPrevented) return;
      const target = e.target as Element | null;
      if (target?.closest?.("input, textarea")) return;
      e.preventDefault();
      const text = selectedText(target);
      if (text) openMenu({ x: e.clientX, y: e.clientY }, [copyItem(text)]);
    };
    window.addEventListener("contextmenu", onContextMenu);
    return () => window.removeEventListener("contextmenu", onContextMenu);
  }, []);

  return open ? <MenuPopup key={open.serial} open={open} /> : null;
}

function MenuPopup({ open }: { open: OpenMenu }) {
  const { entries } = open;
  const ref = useRef<HTMLDivElement>(null);
  const selectable = entries.map((entry) => isItem(entry) && !entry.disabled);
  // 키로 연 메뉴 막대는 첫 항목을 골라 두고, 마우스로 연 메뉴는 아무것도 고르지 않는다(↓가 첫 항목을 고른다).
  const [selected, setSelected] = useState(() => (open.select ? edgeIndex(selectable, false) : -1));
  const [place, setPlace] = useState<{ left: number; top: number } | null>(null);
  const latest = useRef({ selected, selectable });
  latest.current = { selected, selectable };

  // 크기를 재서 창 안에 놓는다.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const size = { width: el.offsetWidth, height: el.offsetHeight };
    setPlace(placeMenu(open.at, size, { width: window.innerWidth, height: window.innerHeight }));
  }, [open]);

  // 보이면 포커스를 가져온다(화면 읽기 프로그램, 코드 뷰에 글자가 가지 않게). 크기를 재는 동안 숨긴 요소는 포커스를
  // 받지 않는다(Chromium·WebKit 모두 focus()가 아무 일도 하지 않는다).
  const shown = place !== null;
  useLayoutEffect(() => {
    if (shown) ref.current?.focus({ preventScroll: true });
  }, [shown]);

  // 메뉴 밖을 누르거나, 창이 포커스를 잃거나 크기가 바뀌거나, 메뉴 밖을 굴리면 닫는다. 메뉴 막대의 제목은 메뉴 막대가
  // 받는다(같은 제목은 닫고 다른 제목은 바꿔 연다).
  useEffect(() => {
    const outside = (e: Event) => {
      const target = e.target as Element | null;
      return !target?.closest?.(".menu") && !(open.bar && target?.closest?.("[data-menubar]"));
    };
    const onDown = (e: MouseEvent) => {
      if (outside(e)) closeMenu();
    };
    const onWheel = (e: WheelEvent) => {
      if (outside(e)) closeMenu();
    };
    const onBlur = () => closeMenu(false);
    const onResize = () => closeMenu();
    document.addEventListener("mousedown", onDown, true);
    document.addEventListener("wheel", onWheel, true);
    window.addEventListener("blur", onBlur);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("mousedown", onDown, true);
      document.removeEventListener("wheel", onWheel, true);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("resize", onResize);
    };
  }, [open]);

  const run = (index: number) => {
    const entry = entries[index];
    if (!entry || !isItem(entry) || entry.disabled) return;
    closeMenu();
    entry.run();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    // 메뉴가 떠 있는 동안 키는 모두 메뉴의 것이다(아래 화면·작업 공간으로 가지 않는다).
    e.stopImmediatePropagation();
    if (isComposing(e)) return;
    const { selected: at, selectable: can } = latest.current;
    let handled = true;
    if (e.key === "ArrowDown") setSelected(stepIndex(can, at, 1));
    else if (e.key === "ArrowUp") setSelected(stepIndex(can, at, -1));
    else if (e.key === "Home" || e.key === "PageUp") setSelected(edgeIndex(can, false));
    else if (e.key === "End" || e.key === "PageDown") setSelected(edgeIndex(can, true));
    else if (e.key === "Enter" || e.key === " ") run(at);
    else if (e.key === "Escape") closeMenu();
    else if (e.key === "Tab") closeMenu();
    else if (open.bar && (e.key === "ArrowLeft" || e.key === "ArrowRight")) open.bar.move(e.key === "ArrowLeft" ? -1 : 1);
    else handled = false;
    // 단축키(⌘C 등)도 메뉴가 떠 있는 동안은 먹지 않는다. 고른 글 복사는 메뉴의 "복사"로 한다.
    if (handled || e.ctrlKey || e.metaKey) e.preventDefault();
  };

  const keys = useRef(onKeyDown);
  keys.current = onKeyDown;
  useLayoutEffect(() => {
    const handler = (e: KeyboardEvent) => keys.current(e);
    keyHandler = handler;
    return () => {
      if (keyHandler === handler) keyHandler = null;
    };
  }, []);

  const maxHeight = window.innerHeight - 2 * MENU_MARGIN;
  return (
    <div
      ref={ref}
      className="menu"
      role="menu"
      aria-label={open.label}
      tabIndex={-1}
      style={place ? { left: place.left, top: place.top, maxHeight } : { left: 0, top: 0, maxHeight, visibility: "hidden" }}
      onContextMenu={(e: ReactMouseEvent) => e.preventDefault()}
      onMouseLeave={() => setSelected(-1)}
    >
      {entries.map((entry, i) => {
        if (entry === "separator") return <div key={i} className="menu-separator" role="separator" />;
        if (!isItem(entry)) {
          return (
            <div key={i} className="menu-header" role="presentation">
              {entry.header}
            </div>
          );
        }
        const classes = ["menu-item"];
        if (i === selected) classes.push("selected");
        if (entry.disabled) classes.push("disabled");
        return (
          <div
            key={i}
            role="menuitem"
            aria-disabled={entry.disabled || undefined}
            className={classes.join(" ")}
            onMouseMove={() => !entry.disabled && i !== latest.current.selected && setSelected(i)}
            onClick={() => run(i)}
          >
            <span className="menu-label">{entry.label}</span>
            {entry.detail && <span className="menu-detail">{entry.detail}</span>}
            {entry.keys && <kbd className="menu-keys">{entry.keys}</kbd>}
          </div>
        );
      })}
    </div>
  );
}

// 도구 막대(작업 공간 맨 위): Windows·Linux는 메뉴 막대(파일 · 보기 · <화면> · 도움말, macOS는 시스템 메뉴 막대), 뒤로,
// 키를 받는 화면의 주요 이동(리비전, 커밋, 부모 blame, 히스토리, 그래프), 파일 찾기·명령 팔레트·도움말. 단추는 그 화면의
// 명령(src/menu/registry.ts)을 실행하고 풍선 도움말에 단축키를 보인다. 그 화면에 없는 일은 흐리게 둔다(자리가 바뀌지
// 않게. 풍선 도움말이 어느 화면의 일인지 알린다). 창이 좁으면 가운데 단추들이 가로로 스크롤된다.
import type { MouseEvent as ReactMouseEvent, ReactNode } from "react";
import { closeMenu, isBarMenuOpen, openMenu, useOpenMenu, type MenuEntry } from "../menu/Menu";
import type { BarItem, BarMenu } from "../menu/model";
import { runIn, targetHandles, targetView, useRegistryVersion, workspaceCommands } from "../menu/registry";
import { currentMenus } from "../menu/sources";
import type { Command, CommandId } from "../palette/Palette";
import { IS_MAC, MOD } from "./keys";

const SHIFT_MOD = IS_MAC ? "⌘⇧" : "Ctrl+Shift+";

/** 단추: 키를 받는 화면에 그 일이 있으면 그 명령을 실행한다. `where`는 흐릴 때 알리는 그 일이 있는 화면 */
const TOOLS: { id: CommandId; label: string; name: string; where: string }[] = [
  { id: "older", label: "‹ 이전", name: "이전 리비전", where: "blame" },
  { id: "newer", label: "다음 ›", name: "다음 리비전", where: "blame" },
  { id: "commit", label: "커밋", name: "커밋 상세", where: "blame·히스토리·그래프" },
  { id: "parent", label: "부모 blame", name: "부모 리비전 blame (바꾸기 전)", where: "blame·히스토리" },
  { id: "blame", label: "blame", name: "이 리비전의 파일 blame", where: "히스토리·커밋 상세" },
  { id: "history", label: "히스토리", name: "파일 히스토리", where: "blame" },
  { id: "lineHistory", label: "라인 히스토리", name: "라인 히스토리 (선택한 줄)", where: "blame" },
];

const titleOf = (command: Pick<Command, "label" | "keys">) => (command.keys ? `${command.label} (${command.keys})` : command.label);

/** 누르는 동안 포커스를 가져가지 않는다: 코드 뷰·목록이 그대로 키를 받는다. */
const keepFocus = (e: ReactMouseEvent) => e.preventDefault();

/** 꺼진 단추도 풍선 도움말이 뜨게 `disabled` 대신 aria-disabled로 끈다. */
function Tool({
  label,
  title,
  disabled = false,
  onClick,
  className,
}: {
  label: ReactNode;
  title: string;
  disabled?: boolean;
  onClick(): void;
  className?: string;
}) {
  return (
    <button
      type="button"
      className={["tool", className, disabled && "off"].filter(Boolean).join(" ")}
      title={title}
      aria-label={typeof label === "string" ? undefined : title}
      aria-disabled={disabled || undefined}
      onMouseDown={keepFocus}
      onClick={() => !disabled && onClick()}
    >
      {label}
    </button>
  );
}

export function Toolbar({ canBack, onBack }: { canBack: boolean; onBack(): void }) {
  useRegistryVersion();
  const target = targetView();
  const find = (id: CommandId) => target?.entry.commands.find((command) => command.id === id);
  const handles = targetHandles();
  const run = (command: Command) => (target ? runIn(target.slot, command) : command.run());
  const open = (pick: "files" | "palette" | "help") => () => {
    const h = targetHandles();
    if (!h) return;
    const go = { run: h[pick] };
    if (target) runIn(target.slot, go);
    else go.run();
  };
  // 그래프: 화면의 그래프(그 커밋을 골라서)가 있으면 그것, 화면이 없으면 저장소 그래프. 그래프 화면에는 없다.
  const graph = find("graph");
  const repoGraph = target ? undefined : handles?.graph;
  // 작업트리 변경: 어느 화면에서든(작업 공간 명령)
  const changes = workspaceCommands().find((command) => command.id === "changes");

  return (
    <div className="toolbar" role="toolbar" aria-label="도구 막대">
      {!IS_MAC && <MenuBar />}
      <div className="toolbar-tools">
        <Tool label="←" title={canBack ? "뒤로 (q)" : "뒤로 (q): 돌아갈 화면이 없다"} disabled={!canBack} onClick={onBack} />
        <span className="tool-sep" />
        {TOOLS.map((tool) => {
          const command = find(tool.id);
          return (
            <Tool
              key={tool.id}
              label={tool.label}
              title={command ? titleOf(command) : `${tool.name}: ${tool.where}에서`}
              disabled={!command}
              onClick={() => command && run(command)}
            />
          );
        })}
        <Tool
          label="그래프"
          title={graph ? titleOf(graph) : repoGraph ? "커밋 그래프 (G)" : "커밋 그래프: 그래프가 아닌 화면에서"}
          disabled={!graph && !repoGraph}
          onClick={() => (graph ? run(graph) : repoGraph?.())}
        />
        <Tool
          label="변경"
          title={changes ? `${titleOf(changes)}: 커밋하지 않은 것` : "작업트리 변경: 저장소를 연 뒤에"}
          disabled={!changes}
          onClick={() => changes?.run()}
        />
      </div>
      <span className="toolbar-spacer" />
      <button type="button" className="tool tool-search" title={`파일 찾기 (f · ${MOD}P)`} onMouseDown={keepFocus} onClick={open("files")}>
        <span>파일 찾기</span>
        <kbd>{MOD}P</kbd>
      </button>
      <Tool label="명령" title={`명령 팔레트 (: · ${SHIFT_MOD}P)`} onClick={open("palette")} />
      <Tool label="?" title="단축키 도움말 (?)" className="tool-help" onClick={open("help")} />
    </div>
  );
}

/** 메뉴 막대 모형 → 드롭다운 줄(하위 메뉴는 머리말 아래에 펼친다) */
function entriesOf(items: BarItem[]): MenuEntry[] {
  return items.flatMap((entry): MenuEntry[] => {
    switch (entry.kind) {
      case "item":
        return [{ label: entry.label, keys: entry.keys, detail: entry.detail, disabled: entry.disabled, run: entry.run }];
      case "separator":
        return ["separator"];
      case "submenu":
        return ["separator", { header: entry.label }, ...entriesOf(entry.items), "separator"];
      case "predefined":
        return [];
    }
  });
}

/** Windows·Linux의 메뉴 막대: 제목을 누르면 드롭다운, 떠 있는 동안 다른 제목에 마우스를 올리면 그 메뉴로 바꾼다. */
function MenuBar() {
  const shown = useOpenMenu();
  const menus = currentMenus(false);
  // `keyboard`: 키로 열었다(첫 항목을 골라 둔다).
  const openAt = (index: number, anchor: HTMLElement | undefined, keyboard: boolean) => {
    // 연 때의 상태로 다시 만든다(이름: 보이기·숨기기, 최근 저장소).
    const fresh: BarMenu[] = currentMenus(false);
    const i = (index + fresh.length) % fresh.length;
    const title = anchor ?? document.querySelector<HTMLElement>(`[data-menubar="${i}"]`);
    if (!title) return;
    const items = entriesOf(fresh[i].items);
    openMenu(title.getBoundingClientRect(), tidyEntries(items), {
      label: fresh[i].title,
      bar: { index: i, move: (delta) => openAt(i + delta, undefined, true) },
      select: keyboard,
    });
  };
  return (
    <div className="menubar" role="menubar" aria-label="메뉴 막대">
      {menus.map((menu, i) => (
        <button
          key={menu.title}
          type="button"
          role="menuitem"
          data-menubar={i}
          className={shown?.bar?.index === i ? "menubar-title open" : "menubar-title"}
          aria-haspopup="menu"
          aria-expanded={shown?.bar?.index === i}
          onMouseDown={(e) => {
            e.preventDefault();
            if (shown?.bar?.index === i) closeMenu();
            else openAt(i, e.currentTarget, false);
          }}
          onMouseEnter={(e) => {
            if (isBarMenuOpen() && shown?.bar?.index !== i) openAt(i, e.currentTarget, false);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown") {
              e.preventDefault();
              openAt(i, e.currentTarget, true);
            }
          }}
        >
          {menu.title}
        </button>
      ))}
      <span className="tool-sep" />
    </div>
  );
}

/** 앞뒤·겹친 구분선을 뺀다(하위 메뉴를 펼치며 넣은 것) */
function tidyEntries(entries: MenuEntry[]): MenuEntry[] {
  const out: MenuEntry[] = [];
  for (const entry of entries) {
    if (entry === "separator" && (out.length === 0 || out[out.length - 1] === "separator")) continue;
    out.push(entry);
  }
  while (out.length > 0 && out[out.length - 1] === "separator") out.pop();
  return out;
}

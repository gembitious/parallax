// 명령 팔레트(`:`, Ctrl·⌘+Shift+P): 지금 화면에서 할 수 있는 일을 이름으로 찾아 실행한다. 키를 몰라도 키보드만으로
// 모든 기능을 쓸 수 있게 하고(마우스로만 되던 것 포함), 옆에 단축키를 보여 익히게 한다.
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { highlightRuns } from "../quickopen/highlight";
import { Runs } from "../quickopen/QuickOpen";
import { filterByLabel } from "./match";
import { openNotices } from "../notices/Notices";
import { OPEN_REPO_KEYS } from "../start/keys";
import { installCli, openPath, openRepoDialog, recentRepos } from "../start/open";

export type Command = {
  /** 팔레트에 보이는 이름(검색 대상) */
  label: string;
  /** 단축키(보이기만 한다) */
  keys?: string;
  /** 덧붙이는 설명(흐리게) */
  detail?: string;
  run(): void;
};

/**
 * 화면에 팔레트를 붙인다. `open()`으로 열고 `overlay`를 화면 끝에 그린다. 고르면 닫은 뒤 그 명령을 실행하고,
 * 그냥 닫으면 `onClosed`(화면에 포커스를 돌려준다)를 부른다. `close()`는 화면이 숨을 때 닫는다(포커스는 그대로).
 * `commands`는 렌더마다 새로 만들어도 된다.
 */
export function usePalette(
  commands: Command[],
  onClosed?: () => void,
): { open(): void; close(): void; isOpen: boolean; overlay: ReactNode } {
  const [isOpen, setOpen] = useState(false);
  const overlay = isOpen ? (
    <Palette
      commands={commands}
      onClose={() => {
        setOpen(false);
        onClosed?.();
      }}
      onRun={(command) => {
        setOpen(false);
        onClosed?.();
        command.run();
      }}
    />
  ) : null;
  return { open: () => setOpen(true), close: () => setOpen(false), isOpen, overlay };
}

function Palette({
  commands,
  onRun,
  onClose,
}: {
  commands: Command[];
  onRun(command: Command): void;
  onClose(): void;
}) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);
  // 명령 목록은 화면이 렌더마다 새로 만든다(실행할 함수가 새 상태를 본다). 맞추기는 이름이 바뀔 때만 다시 하고,
  // 결과는 지금 목록의 명령으로 잇는다.
  const labels = commands.map((command, index) => ({ label: command.label, index }));
  // 이름이 같은 명령(같은 폴더 이름의 최근 저장소)은 설명으로 가린다.
  const labelKey = commands.map(({ label, detail }) => `${label}\t${detail ?? ""}`).join("\n");
  const matched = useMemo(() => filterByLabel(query, labels), [query, labelKey]);
  const results = matched.map(({ item, match }) => ({ item: commands[item.index], match }));

  useEffect(() => {
    inputRef.current?.focus();
  }, []);
  useEffect(() => setSelected(0), [query]);
  useLayoutEffect(() => {
    listRef.current?.children[selected]?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  const move = (delta: number) => setSelected((i) => Math.max(0, Math.min(results.length - 1, i + delta)));
  const run = (index: number) => {
    const result = results[index];
    if (result) onRun(result.item);
  };

  return (
    <div className="overlay top" role="dialog" aria-label="명령" onClick={onClose}>
      <div className="help picker quick-open palette" onClick={(e) => e.stopPropagation()}>
        <div className="quick-open-bar">
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="할 일 이름·초성(ㅍㅎ → 파일 히스토리)"
            aria-label="명령"
            aria-controls="palette-list"
            aria-activedescendant={results.length > 0 ? `palette-${selected}` : undefined}
            spellCheck={false}
            autoComplete="off"
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.nativeEvent.isComposing || e.keyCode === 229) return;
              let handled = true;
              if (e.key === "ArrowDown" || (e.ctrlKey && (e.key === "n" || e.key === "j"))) move(1);
              else if (e.key === "ArrowUp" || (e.ctrlKey && (e.key === "p" || e.key === "k"))) move(-1);
              else if (e.key === "PageDown") move(10);
              else if (e.key === "PageUp") move(-10);
              else if (e.key === "Enter") run(selected);
              else if (e.key === "Escape") onClose();
              else if (e.key !== "Tab") handled = false;
              if (handled) e.preventDefault();
            }}
          />
          <span className="search-status">
            {results.length} / {commands.length}
          </span>
        </div>
        <ul id="palette-list" className="picker-list quick-open-list" role="listbox" ref={listRef}>
          {results.map(({ item, match }, i) => (
            <li
              key={`${item.label}\t${item.detail ?? ""}`}
              id={`palette-${i}`}
              role="option"
              aria-selected={i === selected}
              className={`quick-open-item palette-item${i === selected ? " selected" : ""}`}
              onMouseMove={() => i !== selected && setSelected(i)}
              onClick={() => run(i)}
            >
              <span className="quick-open-name">
                <Runs runs={highlightRuns(item.label, match.positions)} />
              </span>
              {item.detail && <span className="quick-open-dir">{item.detail}</span>}
              {item.keys && <kbd className="palette-keys">{item.keys}</kbd>}
            </li>
          ))}
        </ul>
        {results.length === 0 && <p className="muted">맞는 명령이 없다.</p>}
        <p className="muted">↑↓ 고르기 · Enter 실행 · Esc 닫기</p>
      </div>
    </div>
  );
}

/** 모든 화면에 있는 명령 */
export function commonCommands(actions: {
  files(): void;
  worktrees(): void;
  help(): void;
  back(): void;
  hud?: () => void;
}): Command[] {
  const list: Command[] = [
    { label: "파일 찾기 (Quick Open)", keys: "f · Ctrl+P", run: actions.files },
    { label: "워크트리 고르기", keys: "W", run: actions.worktrees },
    { label: "저장소 열기…", keys: OPEN_REPO_KEYS, detail: "폴더를 골라 그 저장소의 그래프", run: openRepoDialog },
    ...recentRepos()
      .filter((repo) => !repo.missing)
      .map((repo) => ({ label: `최근 저장소: ${repo.name}`, detail: repo.display, run: () => openPath(repo.path) })),
    { label: "단축키 도움말", keys: "?", run: actions.help },
    { label: "뒤로 (맨 앞 화면이면 창 숨기기)", keys: "q", run: actions.back },
    { label: "터미널 명령 plx 설치", detail: "터미널에서 plx로 이 창에 연다", run: () => void installCli() },
    { label: "오픈 소스 라이선스", detail: "앱에 든 다른 프로젝트의 저작권 고지", run: openNotices },
  ];
  if (actions.hud) list.push({ label: "성능 HUD 보이기·숨기기", keys: "`", run: actions.hud });
  return list;
}

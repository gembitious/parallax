// 에디터 그룹의 탭 막대. 누르면 그 탭이 보이고, 두 번 누르면 고정(미리보기 → 보통 탭), 가운데 단추·×로 닫고, 우클릭은
// 탭 메뉴다. 탭을 끌면 작업 공간이 옮긴다(같은 막대의 다른 자리, 다른 그룹, 그룹의 옆, 아래 패널: drag.ts). 놓을 자리를
// 찾게 막대에 그룹(`data-tabbar`), 탭에 탭(`data-tab`)을 적어 둔다.
import { useEffect, useRef, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { MOD } from "./keys";
import { topOf, type Group, type Tab } from "./state";
import { tabLabel } from "./tabs";

type Props = {
  group: Group;
  onActivate(tab: number): void;
  onPin(tab: number): void;
  onClose(tab: number): void;
  /** 우클릭: 탭 메뉴 */
  onMenu(tab: Tab, e: ReactMouseEvent): void;
  /** 탭을 눌렀다(왼쪽 단추). 움직이면 끌기다. */
  onDragStart(tab: Tab, e: ReactPointerEvent<HTMLElement>): void;
};

export function TabBar({ group, onActivate, onPin, onClose, onMenu, onDragStart }: Props) {
  const activeRef = useRef<HTMLDivElement>(null);
  // 아무것도 돌려주지 않는다: 새 Chromium(Windows CI의 WebView2)은 scrollIntoView가 Promise를 돌려줘, 그것을 정리
  // 함수로 받은 React가 탭이 바뀔 때 부르다 실패하고 화면을 모두 내렸다.
  useEffect(() => {
    activeRef.current?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [group.active]);
  return (
    <div className="tab-bar" role="tablist" aria-label="열린 탭" data-tabbar={group.key}>
      {group.tabs.map((tab) => {
        const { name, detail, title } = tabLabel(topOf(tab));
        const active = tab.key === group.active;
        const classes = ["tab"];
        if (active) classes.push("active");
        if (tab.preview) classes.push("preview");
        return (
          <div
            key={tab.key}
            ref={active ? activeRef : undefined}
            data-tab={tab.key}
            role="tab"
            aria-selected={active}
            className={classes.join(" ")}
            title={`${title}${tab.preview ? "\n미리보기: 두 번 누르면 고정한다" : ""}\n끌어서 옮긴다(옆에 놓으면 나눠 본다)`}
            onPointerDown={(e) => {
              if (e.button === 0 && !(e.target as Element).closest(".tab-close")) onDragStart(tab, e);
            }}
            onMouseDown={(e) => {
              // 가운데 단추로 닫는다
              if (e.button === 1) {
                e.preventDefault();
                onClose(tab.key);
              }
            }}
            onClick={() => onActivate(tab.key)}
            onDoubleClick={() => onPin(tab.key)}
            onContextMenu={(e) => onMenu(tab, e)}
          >
            <span className="tab-name">{name}</span>
            {detail && <span className="tab-rev">{detail}</span>}
            <button
              type="button"
              className="tab-close"
              aria-label={`${name} 닫기`}
              title={`닫기 (${MOD}W)`}
              onClick={(e) => {
                e.stopPropagation();
                onClose(tab.key);
              }}
            >
              ×
            </button>
          </div>
        );
      })}
    </div>
  );
}

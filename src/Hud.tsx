import { useEffect, useState, useSyncExternalStore } from "react";
import { clearEntries, getEntries, getPerfInfo, subscribe, type Entry } from "./perf";
import { runProbe } from "./probe";

const VISIBLE_KEY = "plx.hud.visible";

// localStorage는 막혀 있을 수 있다(사생활 보호 설정 등). 그때는 기본값으로 동작한다.
function readVisible(): boolean {
  try {
    return localStorage.getItem(VISIBLE_KEY) !== "0";
  } catch {
    return true;
  }
}

function writeVisible(visible: boolean) {
  try {
    localStorage.setItem(VISIBLE_KEY, visible ? "1" : "0");
  } catch {
    // 저장하지 못해도 이번 실행에는 반영된다.
  }
}

const fmt = (ms: number) => (Number.isFinite(ms) ? ms.toFixed(1) : "—");

function Row({ entry }: { entry: Entry }) {
  const over = entry.budgetMs !== undefined && entry.totalMs > entry.budgetMs;
  return (
    <li className={entry.ok ? undefined : "hud-failed"}>
      <div className="hud-row">
        <span className="hud-label" title={entry.label}>
          {entry.label}
        </span>
        <span className={over ? "hud-total hud-over" : "hud-total"}>
          {fmt(entry.totalMs)}
          {entry.budgetMs !== undefined && <span className="hud-budget"> / {entry.budgetMs}</span>}
        </span>
      </div>
      {(entry.phases.length > 1 || entry.detail) && (
        <div className="hud-phases">
          {entry.phases.length > 1 && entry.phases.map((p) => `${p.label} ${fmt(p.ms)}`).join(" · ")}
          {entry.detail && ` (${entry.detail})`}
        </div>
      )}
    </li>
  );
}

/** 개발 빌드(또는 PLX_PERF=1)에서 요청별 소요 시간(ms)을 보여 준다. ` 키로 켜고 끈다. */
const TOGGLE_EVENT = "plx:hud-toggle";

/** HUD를 보이거나 숨긴다(명령 팔레트). `` ` `` 키와 같다. */
export function toggleHud(): void {
  window.dispatchEvent(new Event(TOGGLE_EVENT));
}

export function Hud() {
  const info = useSyncExternalStore(subscribe, getPerfInfo);
  const entries = useSyncExternalStore(subscribe, getEntries);
  const [visible, setVisible] = useState(readVisible);
  const [probing, setProbing] = useState(false);

  useEffect(() => {
    const toggle = () =>
      setVisible((v) => {
        writeVisible(!v);
        return !v;
      });
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "`" || e.ctrlKey || e.metaKey || e.altKey) return;
      toggle();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener(TOGGLE_EVENT, toggle);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener(TOGGLE_EVENT, toggle);
    };
  }, []);

  if (!info.enabled || !visible) return null;

  const probe = () => {
    setProbing(true);
    runProbe()
      .catch((e) => console.error("IPC 측정 실패", e))
      .finally(() => setProbing(false));
  };

  return (
    <aside className="hud mono" aria-label="성능 HUD">
      <header>
        <span>성능 HUD (ms)</span>
        <button type="button" onClick={probe} disabled={probing}>
          {probing ? "측정 중…" : "IPC 측정"}
        </button>
        <button type="button" onClick={clearEntries}>
          지우기
        </button>
      </header>
      {entries.length === 0 ? (
        <p className="hud-empty">기록 없음</p>
      ) : (
        <ol>
          {entries.map((entry) => (
            <Row key={entry.id} entry={entry} />
          ))}
        </ol>
      )}
    </aside>
  );
}

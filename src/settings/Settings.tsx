// 설정 화면(M9): 모양(테마·코드 글자 크기), blame 거터, diff 보기, 에디터로 열기의 에디터, 새 버전 알림.
// 명령 팔레트·메뉴·첫 화면·Mod+,로 열고(App이 그린다) 고르는 즉시 반영해 이 컴퓨터에 남긴다.
// 열려 있는 동안 화면 키를 막는다(라이선스 창과 같다): 화면들은 window에서 키를 받는다.
import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { blameGutterShown, toggleBlameGutter, useBlameGutter } from "../blame/gutterSetting";
import type { DiffMode } from "../commit/diff";
import { readDiffMode, writeDiffMode } from "../commit/diffMode";
import { editorChoices, type EditorChoices } from "../editor";
import { setUpdateCheck, useUpdate } from "../update/check";
import { FONT_SIZES, updateSettings, useSettings, type Theme } from "./store";

const OPEN_EVENT = "plx-open-settings";

/** 어느 화면에서든 설정 화면을 연다. */
export function openSettings() {
  window.dispatchEvent(new Event(OPEN_EVENT));
}

/** App: 창을 열라는 요청을 받는다. */
export function useSettingsRequest(open: () => void) {
  useEffect(() => {
    window.addEventListener(OPEN_EVENT, open);
    return () => window.removeEventListener(OPEN_EVENT, open);
  }, [open]);
}

/** 설정 화면 키: Mod+,(macOS ⌘, · 나머지 Ctrl+,), 입력란 안에서도 */
export function isSettingsKey(e: Pick<globalThis.KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">, mac: boolean) {
  return (mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey) && !e.altKey && !e.shiftKey && e.key === ",";
}

const THEMES: { value: Theme; label: string }[] = [
  { value: "system", label: "시스템 따라가기" },
  { value: "light", label: "밝게" },
  { value: "dark", label: "어둡게" },
];

/** 에디터 칸: 자동, 찾은 에디터, 직접 쓰기 */
const AUTO = "";
const CUSTOM = "\u0000custom";

export function Settings({ onClose }: { onClose(): void }) {
  const settings = useSettings();
  const gutter = useBlameGutter();
  const update = useUpdate();
  const [diffMode, setDiffMode] = useState<DiffMode>(readDiffMode);
  const [choices, setChoices] = useState<EditorChoices | { error: string } | null>(null);
  const box = useRef<HTMLDivElement>(null);

  // 열 때 포커스를 가져오고, 닫으면 전에 포커스가 있던 곳(아래 화면)으로 돌려준다.
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    box.current?.querySelector<HTMLElement>("select, input, button")?.focus({ preventScroll: true });
    return () => previous?.focus({ preventScroll: true });
  }, []);

  // 찾은 에디터는 열 때마다 묻는다(그사이 설치했거나 다른 셸에서 plx를 불러 PATH가 바뀌었을 수 있다).
  useEffect(() => {
    let stale = false;
    editorChoices().then(
      (found) => !stale && setChoices(found),
      (e: unknown) => !stale && setChoices({ error: String(e) }),
    );
    return () => {
      stale = true;
    };
  }, []);

  const found = choices && "found" in choices ? choices.found : [];
  const chosenIsListed = settings.editor === AUTO || found.some((choice) => choice.spec === settings.editor);
  const [custom, setCustom] = useState(!chosenIsListed);
  const [draft, setDraft] = useState(chosenIsListed ? "" : settings.editor);
  // 목록이 늦게 와서 고른 값이 목록에 있으면 직접 쓰기를 접는다(처음 그릴 때는 목록이 없다).
  useEffect(() => {
    if (custom && draft === settings.editor && found.some((choice) => choice.spec === settings.editor)) {
      setCustom(false);
      setDraft("");
    }
    // 목록이 올 때만 본다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [choices]);

  const editorValue = custom ? CUSTOM : settings.editor;
  const chooseEditor = (value: string) => {
    if (value === CUSTOM) {
      setCustom(true);
      setDraft(settings.editor);
      return;
    }
    setCustom(false);
    updateSettings({ editor: value });
  };

  const onKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.nativeEvent.isComposing) return;
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
        return;
      }
      // 입력·고르기·Tab은 브라우저에 맡기되 아래 화면에는 보내지 않는다.
      e.stopPropagation();
    },
    [onClose],
  );

  const autoName =
    choices === null
      ? "찾는 중…"
      : "error" in choices
        ? "찾지 못함"
        : choices.auto
          ? `지금은 ${choices.auto}`
          : "찾은 에디터 없음";

  return (
    <div className="overlay" role="dialog" aria-label="설정" onClick={onClose}>
      {/* 글이나 빈 곳을 눌러도 포커스가 창 안에 남게(tabIndex) 한다. 창 밖(body)으로 가면 키가 아래 화면에 간다. */}
      <div
        ref={box}
        className="help settings"
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <h2>
          설정 <span className="muted">(바로 반영된다 · Esc 닫기)</span>
        </h2>

        <section>
          <h3>모양</h3>
          <div className="settings-row">
            <span className="settings-label">테마</span>
            <div className="settings-choices" role="radiogroup" aria-label="테마">
              {THEMES.map(({ value, label }) => (
                <label key={value}>
                  <input
                    type="radio"
                    name="theme"
                    value={value}
                    checked={settings.theme === value}
                    onChange={() => updateSettings({ theme: value })}
                  />
                  {label}
                </label>
              ))}
            </div>
          </div>
          <div className="settings-row">
            <label className="settings-label" htmlFor="settings-font">
              코드 글자 크기
            </label>
            <div className="settings-choices">
              <button
                type="button"
                aria-label="작게"
                disabled={settings.codeFontSize <= FONT_SIZES.min}
                onClick={() => updateSettings({ codeFontSize: settings.codeFontSize - 1 })}
              >
                −
              </button>
              <select
                id="settings-font"
                value={settings.codeFontSize}
                onChange={(e) => updateSettings({ codeFontSize: Number(e.target.value) })}
              >
                {Array.from({ length: FONT_SIZES.max - FONT_SIZES.min + 1 }, (_, i) => FONT_SIZES.min + i).map((size) => (
                  <option key={size} value={size}>
                    {size}px{size === FONT_SIZES.default ? " (기본)" : ""}
                  </option>
                ))}
              </select>
              <button
                type="button"
                aria-label="크게"
                disabled={settings.codeFontSize >= FONT_SIZES.max}
                onClick={() => updateSettings({ codeFontSize: settings.codeFontSize + 1 })}
              >
                +
              </button>
              <span className="muted">blame·diff 코드</span>
            </div>
          </div>
        </section>

        <section>
          <h3>blame · 커밋 상세</h3>
          <div className="settings-row">
            <span className="settings-label">blame</span>
            <label className="settings-choices">
              <input
                type="checkbox"
                checked={gutter}
                onChange={(e) => e.target.checked !== blameGutterShown() && toggleBlameGutter()}
              />
              거터에 줄마다 커밋 보이기 <span className="muted">(끄면 커서 줄 끝에 한 줄 · b)</span>
            </label>
          </div>
          <div className="settings-row">
            <span className="settings-label">diff 보기</span>
            <div className="settings-choices" role="radiogroup" aria-label="diff 보기">
              {(
                [
                  ["split", "분할"],
                  ["unified", "통합"],
                ] as const
              ).map(([value, label]) => (
                <label key={value}>
                  <input
                    type="radio"
                    name="diff-mode"
                    value={value}
                    checked={diffMode === value}
                    onChange={() => {
                      writeDiffMode(value);
                      setDiffMode(value);
                    }}
                  />
                  {label}
                </label>
              ))}
              <span className="muted">새로 연 커밋 상세부터 · u</span>
            </div>
          </div>
        </section>

        <section>
          <h3>에디터로 열기 (o)</h3>
          <div className="settings-row">
            <label className="settings-label" htmlFor="settings-editor">
              에디터
            </label>
            <div className="settings-choices">
              <select id="settings-editor" value={editorValue} onChange={(e) => chooseEditor(e.target.value)}>
                <option value={AUTO}>자동 ({autoName})</option>
                {found.map((choice) => (
                  <option key={choice.spec} value={choice.spec}>
                    {choice.name} ({choice.spec})
                  </option>
                ))}
                <option value={CUSTOM}>직접 쓰기…</option>
              </select>
            </div>
          </div>
          {custom && (
            <div className="settings-row">
              <label className="settings-label" htmlFor="settings-editor-command">
                명령
              </label>
              <div className="settings-field">
                <input
                  id="settings-editor-command"
                  type="text"
                  spellCheck={false}
                  placeholder="subl {file}:{line}"
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  onBlur={() => updateSettings({ editor: draft })}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") updateSettings({ editor: draft });
                  }}
                />
                <p className="muted">
                  이름이나 경로(<code>zed</code>, <code>/usr/local/bin/idea</code>), 또는 <code>{"{file}"}</code>·
                  <code>{"{line}"}</code>·<code>{"{col}"}</code>을 넣은 명령. 비우면 자동이다.
                </p>
              </div>
            </div>
          )}
          <p className="muted settings-note">
            자동은 <code>PLX_EDITOR</code>, <code>VISUAL</code>·<code>EDITOR</code>, PATH의 VS Code·Cursor·Zed 순으로
            고른다. PATH는 마지막으로 <code>plx</code>를 부른 셸의 것을 먼저 본다.
            {choices && "error" in choices && <span className="error-text"> 목록을 받지 못했다: {choices.error}</span>}
            {choices && "found" in choices && !choices.auto && choices.autoError && (
              <span className="error-text"> {choices.autoError}</span>
            )}
          </p>
        </section>

        <section>
          <h3>새 버전</h3>
          <div className="settings-row">
            <span className="settings-label">알림</span>
            <label className="settings-choices">
              <input type="checkbox" checked={update.enabled} onChange={(e) => setUpdateCheck(e.target.checked)} />
              하루 한 번 GitHub에 최신 버전을 묻는다
              {update.current && <span className="muted"> (지금 v{update.current})</span>}
            </label>
          </div>
        </section>

        <p className="help-footer muted">설정은 이 컴퓨터에 남는다.</p>
      </div>
    </div>
  );
}

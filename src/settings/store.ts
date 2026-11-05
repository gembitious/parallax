// 앱 설정(M9): 테마, 코드 글자 크기, 에디터로 열기(`o`)의 에디터, blame 선택(M11: 공백 무시, 옮긴 줄 따라가기). 설정 화면(Settings.tsx)이 고치고 이 컴퓨터에 남긴다
// (브라우저 저장소 한 칸 `plx.settings`). 창이 뜨기 전의 테마·글자 크기는 public/theme.js가 같은 칸을 읽어 먼저 놓는다.
// 다른 설정(blame 거터, diff 보기, 새 버전 알림)은 원래 자리(blame/gutterSetting.ts 등)에 두고 설정 화면이 그곳을 고친다.
import { useSyncExternalStore } from "react";

export type Theme = "system" | "light" | "dark";

/** 옮기거나 복사한 줄을 어디까지 따라가는가(plx-core/src/blame.rs Moves): 끔, 파일 안(-M), 다른 파일까지(-M -C) */
export type BlameMoves = "off" | "file" | "files";

/** git blame 선택(plx-core/src/blame.rs BlameOptions) */
export type BlameOptions = { ignoreWhitespace: boolean; moves: BlameMoves };

export type Settings = {
  theme: Theme;
  /** 코드 뷰(blame·diff)의 글자 크기(px) */
  codeFontSize: number;
  /** 에디터로 열기의 에디터(`PLX_EDITOR`와 같은 형식). 비었으면 자동(`PLX_EDITOR`, PATH, 설치 위치) */
  editor: string;
  /** blame이 공백만 바꾼 변경을 지나간다(`-w`) */
  blameIgnoreWhitespace: boolean;
  /** blame이 옮긴 줄을 따라간다 */
  blameMoves: BlameMoves;
};

const KEY = "plx.settings";
export const FONT_SIZES = { min: 10, max: 20, default: 13 } as const;
export const DEFAULTS: Settings = {
  theme: "system",
  codeFontSize: FONT_SIZES.default,
  editor: "",
  blameIgnoreWhitespace: false,
  blameMoves: "off",
};

/** 저장한 값 읽기: 틀린 칸만 기본값으로 */
export function parseSettings(raw: unknown): Settings {
  const o = typeof raw === "object" && raw !== null && !Array.isArray(raw) ? (raw as Record<string, unknown>) : {};
  const theme = o.theme === "light" || o.theme === "dark" ? o.theme : "system";
  const size = o.codeFontSize;
  const codeFontSize =
    typeof size === "number" && Number.isInteger(size) && size >= FONT_SIZES.min && size <= FONT_SIZES.max
      ? size
      : FONT_SIZES.default;
  const editor = typeof o.editor === "string" ? o.editor.trim().slice(0, 1000) : "";
  const blameIgnoreWhitespace = o.blameIgnoreWhitespace === true;
  const blameMoves = o.blameMoves === "file" || o.blameMoves === "files" ? o.blameMoves : "off";
  return { theme, codeFontSize, editor, blameIgnoreWhitespace, blameMoves };
}

/** 코드 줄 높이: 13px에 20px(지금까지의 값)과 같은 비율 */
export const lineHeightFor = (fontSize: number) => Math.round((fontSize * 20) / 13);

function read(): Settings {
  try {
    return parseSettings(JSON.parse(localStorage.getItem(KEY) ?? "null"));
  } catch {
    return DEFAULTS;
  }
}

let settings = read();
const listeners = new Set<() => void>();

export const getSettings = (): Settings => settings;

export function updateSettings(patch: Partial<Settings>): void {
  const next = parseSettings({ ...settings, ...patch });
  if ((Object.keys(next) as (keyof Settings)[]).every((key) => next[key] === settings[key])) return;
  const fontChanged = next.codeFontSize !== settings.codeFontSize;
  settings = next;
  try {
    localStorage.setItem(KEY, JSON.stringify(settings));
  } catch {
    // 저장하지 못해도 이번 실행에는 반영된다.
  }
  apply();
  // CodeMirror는 창 크기가 바뀔 때 줄 높이를 다시 잰다(글자 크기는 CSS 변수라 스스로 알지 못한다).
  if (fontChanged) window.dispatchEvent(new Event("resize"));
  for (const listener of listeners) listener();
}

/** blame 요청에 줄 선택 */
export const blameOptions = (s: Settings = settings): BlameOptions => ({
  ignoreWhitespace: s.blameIgnoreWhitespace,
  moves: s.blameMoves,
});

/** 두 선택이 같은가 */
export const sameBlameOptions = (a: BlameOptions, b: BlameOptions) =>
  a.ignoreWhitespace === b.ignoreWhitespace && a.moves === b.moves;

/** 머리줄 이름표: 켠 선택을 git 인자로(`-w -M`). 기본이면 빈 글 */
export function blameOptionsLabel(o: BlameOptions): string {
  const args = [o.ignoreWhitespace ? "-w" : "", o.moves === "off" ? "" : "-M", o.moves === "files" ? "-C" : ""];
  return args.filter(Boolean).join(" ");
}

export const subscribeSettings = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

export function useSettings(): Settings {
  return useSyncExternalStore(subscribeSettings, getSettings);
}

// 테마: 시스템을 따라가면 OS 설정이 바뀔 때도 바꾼다.
const darkQuery = typeof window !== "undefined" ? window.matchMedia?.("(prefers-color-scheme: dark)") : undefined;

/** 지금 어두운 테마인가 */
export const isDark = (): boolean =>
  settings.theme === "dark" || (settings.theme === "system" && (darkQuery?.matches ?? false));

const darkListeners = new Set<(dark: boolean) => void>();
let dark = false;

/** 어두운·밝은 테마가 바뀌면 부른다(코드 뷰의 다크 테마, 그래프 레인 색) */
export function subscribeDark(listener: (dark: boolean) => void): () => void {
  darkListeners.add(listener);
  return () => {
    darkListeners.delete(listener);
  };
}

function apply(): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const nextDark = isDark();
  root.dataset.theme = nextDark ? "dark" : "light";
  root.style.setProperty("--code-font-size", `${settings.codeFontSize}px`);
  root.style.setProperty("--code-line-height", `${lineHeightFor(settings.codeFontSize)}px`);
  if (nextDark !== dark) {
    dark = nextDark;
    for (const listener of darkListeners) listener(dark);
  }
}

dark = isDark();
apply();
darkQuery?.addEventListener("change", apply);

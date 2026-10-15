// 읽기 전용 코드 뷰(CodeMirror 6). blame 거터, 줄 번호, 검색, 구문 강조(언어는 필요할 때 불러온다).
import { cursorLineDown, cursorLineUp, defaultKeymap } from "@codemirror/commands";
import { HighlightStyle, LanguageDescription, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { findNext, findPrevious, highlightSelectionMatches, openSearchPanel, search, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState, Prec } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { tags as t } from "@lezer/highlight";
import { blameGutter, type GutterContext } from "./gutter";

/** 화면 단축키. true를 돌려주면 처리한 것이다. CLAUDE.md 키맵을 따른다. */
export type EditorKeys = {
  copySha(): boolean;
  enter(): boolean;
  back(): boolean;
  help(): boolean;
  escape(): boolean;
};

export type EditorOptions = {
  doc: string;
  path: string;
  /** 처음 선택할 줄(1부터) */
  line: number | null;
  gutter: GutterContext;
  keys: EditorKeys;
};

// 색은 styles.css의 변수라 밝은·어두운 테마를 CSS만으로 바꾼다.
const highlightStyle = HighlightStyle.define([
  { tag: [t.keyword, t.modifier, t.controlKeyword, t.operatorKeyword], color: "var(--syn-keyword)" },
  { tag: [t.string, t.special(t.string), t.regexp, t.character], color: "var(--syn-string)" },
  { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: "var(--syn-comment)", fontStyle: "italic" },
  { tag: [t.number, t.bool, t.null, t.atom], color: "var(--syn-constant)" },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.macroName], color: "var(--syn-function)" },
  { tag: [t.typeName, t.className, t.namespace, t.tagName], color: "var(--syn-type)" },
  { tag: [t.propertyName, t.attributeName], color: "var(--syn-property)" },
  { tag: [t.meta, t.annotation, t.processingInstruction], color: "var(--syn-meta)" },
  { tag: [t.heading, t.strong], fontWeight: "bold" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.link, textDecoration: "underline" },
  { tag: t.invalid, color: "var(--syn-invalid)" },
]);

const theme = EditorView.theme({
  "&": { height: "100%", backgroundColor: "var(--bg)", color: "var(--fg)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", fontSize: "13px", lineHeight: "20px" },
  ".cm-content": { caretColor: "var(--fg)", padding: "0" },
  ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--muted)", border: "none" },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 10px 0 6px", minWidth: "4ch" },
  ".cm-activeLine": { backgroundColor: "var(--line-active)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--line-active)" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground": {
    backgroundColor: "var(--selection)",
  },
  ".cm-searchMatch": { backgroundColor: "var(--search-match)" },
  ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: "var(--search-current)" },
  ".cm-selectionMatch": { backgroundColor: "var(--selection-match)" },
  ".cm-panels": { backgroundColor: "var(--panel)", color: "var(--fg)" },
  ".cm-panels.cm-panels-top": { borderBottom: "1px solid var(--border)" },
  ".cm-panel.cm-search": { fontFamily: "inherit", fontSize: "12px" },
});

// 검색 패널 문구. 읽기 전용이라 바꾸기 칸은 보이지 않는다.
const phrases = EditorState.phrases.of({
  Find: "찾기",
  next: "다음",
  previous: "이전",
  all: "모두",
  "match case": "대소문자",
  regexp: "정규식",
  "by word": "단어 단위",
  close: "닫기",
  "Go to line": "줄로 이동",
  go: "이동",
  "current match": "현재 일치",
  "on line": "줄",
  "Selection deleted": "",
});

const darkQuery = window.matchMedia?.("(prefers-color-scheme: dark)");

export type Editor = { view: EditorView; destroy(): void };

export function createEditor(parent: HTMLElement, options: EditorOptions): Editor {
  const language = new Compartment();
  const colorScheme = new Compartment();
  const { keys } = options;
  const run = (f: () => boolean) => () => f();

  const doc = options.doc;
  const state = EditorState.create({
    doc,
    extensions: [
      // git blame과 줄 번호를 맞춘다: 줄은 \n으로만 나눈다(외톨이 \r은 줄을 나누지 않는다).
      EditorState.lineSeparator.of("\n"),
      EditorState.readOnly.of(true),
      EditorState.tabSize.of(4),
      phrases,
      blameGutter(options.gutter),
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightActiveLine(),
      highlightSpecialChars(),
      drawSelection(),
      highlightSelectionMatches(),
      search({ top: true }),
      syntaxHighlighting(highlightStyle),
      language.of([]),
      colorScheme.of(EditorView.darkTheme.of(darkQuery?.matches ?? false)),
      theme,
      Prec.highest(
        keymap.of([
          { key: "j", run: cursorLineDown },
          { key: "k", run: cursorLineUp },
          { key: "y", run: run(keys.copySha) },
          { key: "/", run: openSearchPanel },
          { key: "n", run: findNext },
          { key: "N", run: findPrevious },
          { key: "Enter", run: run(keys.enter) },
          { key: "q", run: run(keys.back) },
          { key: "?", run: run(keys.help) },
          { key: "Escape", run: run(keys.escape) },
        ]),
      ),
      keymap.of([...searchKeymap, ...defaultKeymap]),
      EditorView.contentAttributes.of({ "aria-label": options.path }),
    ],
    selection: { anchor: lineStart(doc, options.line) },
  });

  const view = new EditorView({
    state,
    parent,
    scrollTo: options.line ? EditorView.scrollIntoView(state.selection.main.head, { y: "center" }) : undefined,
  });

  const onScheme = (e: MediaQueryListEvent) =>
    view.dispatch({ effects: colorScheme.reconfigure(EditorView.darkTheme.of(e.matches)) });
  darkQuery?.addEventListener("change", onScheme);
  let destroyed = false;

  // 구문 강조는 내용을 먼저 그린 뒤에 붙인다. 언어 모듈은 따로 나뉜 청크라 처음에는 불러오고 평가하는 시간이 들고,
  // 붙이는 순간 구문 분석도 돈다. 첫 페인트 앞에 끼면 내용 표시가 늦어진다.
  const description = LanguageDescription.matchFilename(languages, options.path);
  if (description) {
    requestAnimationFrame(() =>
      setTimeout(() => {
        if (destroyed) return;
        description
          .load()
          .then((support) => {
            if (!destroyed) view.dispatch({ effects: language.reconfigure(support) });
          })
          .catch((e) => console.warn("언어 모듈을 불러오지 못했다", description.name, e));
      }),
    );
  }

  return {
    view,
    destroy() {
      destroyed = true;
      darkQuery?.removeEventListener("change", onScheme);
      view.destroy();
    },
  };
}

/** `line`번째 줄(1부터, 범위 밖이면 가까운 줄)의 시작 위치. */
function lineStart(doc: string, line: number | null): number {
  if (!line || line <= 1) return 0;
  let pos = 0;
  for (let i = 1; i < line; i++) {
    const next = doc.indexOf("\n", pos);
    if (next < 0) break;
    pos = next + 1;
  }
  return pos;
}

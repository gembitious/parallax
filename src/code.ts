// 읽기 전용 코드 뷰의 공통 부분: 테마(styles.css 변수), 구문 강조, 검색 문구, 다크 모드, 언어 모듈 지연 로딩,
// 우클릭한 줄과 고른 줄. blame 뷰(blame/editor.ts)와 diff 뷰(commit/diff.ts)가 함께 쓴다.
import { HighlightStyle, LanguageDescription, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

// 색은 styles.css의 변수라 밝은·어두운 테마를 CSS만으로 바꾼다.
export const highlightStyle = HighlightStyle.define([
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

export const codeTheme = EditorView.theme({
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
export const searchPhrases = EditorState.phrases.of({
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

/** 우클릭한 자리의 글자 위치. 거터(줄 밖)면 그 높이 줄의 맨 앞. 코드 뷰 밖이면 null */
export function pointerPos(view: EditorView, e: MouseEvent): number | null {
  if ((e.target as Element | null)?.closest?.(".cm-content")) {
    const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
    if (pos !== null) return pos;
  }
  if (!(e.target as Element | null)?.closest?.(".cm-editor")) return null;
  return view.lineBlockAtHeight(e.clientY - view.documentTop).from;
}

/**
 * 우클릭: 그 자리로 커서를 옮기고(고른 글 안을 눌렀으면 고른 것을 둔다) 포커스를 준다. CodeMirror는 오른쪽 단추로
 * 커서를 옮기지 않는다. 우클릭 메뉴는 그 줄(고른 줄)의 일을 보인다.
 */
export function pointCursor(view: EditorView, e: MouseEvent) {
  const pos = pointerPos(view, e);
  const chosen = view.state.selection.main;
  if (pos !== null && !(chosen.from < chosen.to && pos >= chosen.from && pos <= chosen.to)) {
    view.dispatch({ selection: { anchor: pos } });
  }
  view.focus();
}

/** 고른 줄들(1부터, 끝이 줄 맨 앞이면 그 줄은 뺀다). 고른 것이 없으면 커서 줄. */
export function selectedLines(state: EditorState): { start: number; end: number } {
  const { from, to } = state.selection.main;
  const doc = state.doc;
  const start = doc.lineAt(from).number;
  const last = doc.lineAt(to);
  return { start, end: to > from && to === last.from ? Math.max(start, last.number - 1) : last.number };
}

const darkQuery = window.matchMedia?.("(prefers-color-scheme: dark)");

/**
 * 코드 뷰 하나(또는 diff 양쪽)에 붙이는 공통 확장과, 뷰를 만든 뒤 다크 모드와 언어를 따라가게 하는 연결.
 * 구문 강조는 내용을 먼저 그린 뒤에 붙인다. 언어 모듈은 따로 나뉜 청크라 처음에는 불러오고 평가하는 시간이 들고,
 * 붙이는 순간 구문 분석도 돈다. 첫 페인트 앞에 끼면 내용 표시가 늦어진다.
 */
export function codeSupport(path: string): { extensions: Extension; attach(views: EditorView[]): () => void } {
  const language = new Compartment();
  const scheme = new Compartment();
  const extensions: Extension = [
    EditorState.readOnly.of(true),
    EditorState.tabSize.of(4),
    searchPhrases,
    syntaxHighlighting(highlightStyle),
    language.of([]),
    scheme.of(EditorView.darkTheme.of(darkQuery?.matches ?? false)),
    codeTheme,
  ];
  const attach = (views: EditorView[]) => {
    let detached = false;
    const onScheme = (e: MediaQueryListEvent) => {
      for (const view of views) view.dispatch({ effects: scheme.reconfigure(EditorView.darkTheme.of(e.matches)) });
    };
    darkQuery?.addEventListener("change", onScheme);
    const description = LanguageDescription.matchFilename(languages, path);
    if (description) {
      requestAnimationFrame(() =>
        setTimeout(() => {
          if (detached) return;
          description
            .load()
            .then((support) => {
              if (detached) return;
              for (const view of views) view.dispatch({ effects: language.reconfigure(support) });
            })
            .catch((e) => console.warn("언어 모듈을 불러오지 못했다", description.name, e));
        }),
      );
    }
    return () => {
      detached = true;
      darkQuery?.removeEventListener("change", onScheme);
    };
  };
  return { extensions, attach };
}

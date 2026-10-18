// blame 거터: 같은 커밋이 이어지는 줄 묶음의 첫 줄에 요약·작성자·상대 시간을 쓰고, 모든 줄에 나이 히트맵 막대를 그린다.
// blame은 점진적으로 오므로 모델이 바뀔 때마다(blameChanged) 보이는 줄의 표시만 다시 계산한다.
import { StateEffect, type EditorState } from "@codemirror/state";
import { EditorView, GutterMarker, gutter, type BlockInfo, type ViewUpdate } from "@codemirror/view";
import { aiLabel } from "./ai";
import { heatLevel, isUncommitted, relativeTime } from "./format";
import type { BlameModel } from "./model";

/** 모델에 블록이 더해졌다. */
export const blameChanged = StateEffect.define<null>();

export type GutterContext = {
  model: BlameModel;
  /** 상대 시간·히트맵 기준 시각(ms). 요청마다 고정해 표시가 흔들리지 않게 한다. */
  nowMs: number;
  /** 거터 위 마우스가 줄 `line`에 올라갔다(null이면 떠났다). `anchor`는 그 줄 거터 칸의 위치. */
  onHover(line: number | null, anchor: DOMRect | null): void;
  /** 거터를 눌렀다. */
  onClick(line: number): void;
};

class LineMarker extends GutterMarker {
  constructor(
    readonly key: string,
    readonly className: string,
    readonly texts: readonly [summary: string, author: string, time: string] | null,
  ) {
    super();
  }

  eq(other: GutterMarker): boolean {
    return other instanceof LineMarker && other.key === this.key;
  }

  toDOM(): Node {
    const el = document.createElement("div");
    el.className = this.className;
    if (this.texts) {
      const [summary, author, time] = this.texts;
      for (const [cls, text] of [
        ["cm-blame-summary", summary],
        ["cm-blame-author", author],
        ["cm-blame-time", time],
      ] as const) {
        const span = document.createElement("span");
        span.className = cls;
        span.textContent = text;
        el.append(span);
      }
    }
    return el;
  }
}

const lineOf = (view: EditorView, block: BlockInfo) => view.state.doc.lineAt(block.from).number;

/** 커서가 있는 줄 번호. */
const cursorLine = (state: EditorState) => state.doc.lineAt(state.selection.main.head).number;

export function blameGutter(ctx: GutterContext) {
  const { model, nowMs } = ctx;
  // 같은 모양의 표시는 하나만 만들어 DOM을 다시 쓰지 않게 한다. 커밋 정보는 바뀌지 않으므로 요청 동안 유효하다.
  const markers = new Map<string, LineMarker>();
  // 커서 줄과 그 줄의 커밋. 같은 커밋 줄을 거터에서 함께 표시하고, 커서 줄에는 묶음 중간이어도 글을 쓴다
  // (긴 묶음의 첫 줄이 화면 밖에 있어도 지금 줄의 커밋을 알 수 있게).
  let cursorFor: EditorState | null = null;
  let cursor = 0;
  let active: string | undefined;

  /**
   * `start`: 묶음의 첫 줄(위에 구분선), `label`: 글을 쓴다, `isActive`: 커서 줄과 같은 커밋,
   * `ai`: 그 줄을 쓴 AI 에이전트·사람(git-ai 노트, 작성자 번호). 에이전트 줄은 보라 줄을 긋고 작성자 자리에 도구 이름을 쓴다.
   */
  const markerFor = (sha: string, start: boolean, label: boolean, isActive: boolean, ai: number | null): LineMarker => {
    const key = `${sha}:${+start}${+label}${+isActive}:${ai ?? ""}`;
    let marker = markers.get(key);
    if (marker) return marker;
    const commit = model.commits.get(sha);
    const uncommitted = isUncommitted(sha);
    const author = ai === null ? null : model.ai.authors[ai];
    const agent = author?.kind === "agent" ? author : null;
    const heat = uncommitted ? "uncommitted" : `heat-${commit ? heatLevel(commit.authorTime, nowMs) : 0}`;
    const className = ["cm-blame-line", heat, start ? "start" : "", isActive ? "active" : "", agent ? "ai" : ""]
      .join(" ")
      .trim();
    let texts: LineMarker["texts"] = null;
    if (label) {
      texts = uncommitted
        ? ["커밋 안 됨", "", ""]
        : [
            commit?.summary ?? "",
            agent ? `✦ ${aiLabel(agent)}` : (commit?.author ?? ""),
            commit ? relativeTime(commit.authorTime, nowMs) : "",
          ];
    }
    marker = new LineMarker(key, className, texts);
    markers.set(key, marker);
    return marker;
  };

  return gutter({
    class: "cm-blame-gutter",
    lineMarker(view, block) {
      const line = lineOf(view, block);
      const sha = model.sha(line);
      if (sha === undefined) return null;
      if (cursorFor !== view.state) {
        cursorFor = view.state;
        cursor = cursorLine(view.state);
        active = model.sha(cursor);
      }
      const start = model.isRunStart(line);
      return markerFor(sha, start, start || line === cursor, sha === active, model.aiIndexAt(line));
    },
    lineMarkerChange: (update: ViewUpdate) =>
      update.selectionSet || update.transactions.some((tr) => tr.effects.some((e) => e.is(blameChanged))),
    domEventHandlers: {
      mousemove(view, block, event) {
        const target = (event.target as HTMLElement).closest(".cm-gutterElement");
        ctx.onHover(lineOf(view, block), target?.getBoundingClientRect() ?? null);
        return false;
      },
      mouseleave() {
        ctx.onHover(null, null);
        return false;
      },
      mousedown(view, block) {
        const line = lineOf(view, block);
        view.dispatch({ selection: { anchor: view.state.doc.line(line).from } });
        view.focus();
        ctx.onClick(line);
        return true;
      },
    },
  });
}

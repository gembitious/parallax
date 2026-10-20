// 커서 줄 끝의 blame 한 줄(GitLens의 current line blame). 거터를 끈 동안 커서가 있는 줄의 커밋(작성자·상대 시간·
// 요약)을 줄 끝에 흐리게 쓴다. 블록이 오거나(blameChanged) 커서가 옮겨 가면 다시 그린다. 마우스를 올리면 커밋 카드,
// 누르면 카드 고정(거터와 같다).
import { RangeSet } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import { aiLabel } from "./ai";
import { isUncommitted, relativeTime } from "./format";
import { blameChanged, type GutterContext } from "./gutter";

class LineBlame extends WidgetType {
  constructor(
    readonly line: number,
    readonly text: string,
    readonly className: string,
    readonly ctx: GutterContext,
  ) {
    super();
  }

  eq(other: LineBlame): boolean {
    return other.line === this.line && other.text === this.text && other.className === this.className;
  }

  toDOM(): HTMLElement {
    const el = document.createElement("span");
    el.className = this.className;
    el.textContent = this.text;
    el.setAttribute("aria-hidden", "true");
    el.addEventListener("mouseenter", () => this.ctx.onHover(this.line, el.getBoundingClientRect()));
    el.addEventListener("mouseleave", () => this.ctx.onHover(null, null));
    el.addEventListener("mousedown", (e) => {
      // 코드 뷰가 커서를 옮기지 않게(이 줄은 이미 커서 줄이다)
      e.preventDefault();
      this.ctx.onClick(this.line);
    });
    return el;
  }

  ignoreEvent(): boolean {
    return true;
  }
}

function decorations(view: EditorView, ctx: GutterContext): DecorationSet {
  const { model, nowMs } = ctx;
  const head = view.state.selection.main.head;
  const line = view.state.doc.lineAt(head);
  const block = model.block(line.number);
  if (!block) return RangeSet.empty;
  let text: string;
  let className = "cm-inline-blame";
  if (isUncommitted(block.sha)) {
    text = "커밋 안 됨";
    className += " uncommitted";
  } else {
    const commit = model.commits.get(block.sha);
    const author = model.aiAt(line.number);
    const who = author?.kind === "agent" ? `✦ ${aiLabel(author)}` : (commit?.author ?? "");
    if (author?.kind === "agent") className += " ai";
    text = [who, commit ? relativeTime(commit.authorTime, nowMs) : "", commit?.summary ?? ""].filter(Boolean).join(" · ");
  }
  const widget = Decoration.widget({ widget: new LineBlame(line.number, text, className, ctx), side: 1 });
  return Decoration.set([widget.range(line.to)]);
}

export function inlineBlame(ctx: GutterContext) {
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;

      constructor(view: EditorView) {
        this.decorations = decorations(view, ctx);
      }

      update(update: ViewUpdate) {
        if (update.selectionSet || update.transactions.some((tr) => tr.effects.some((e) => e.is(blameChanged)))) {
          this.decorations = decorations(update.view, ctx);
        }
      }
    },
    { decorations: (plugin) => plugin.decorations },
  );
}

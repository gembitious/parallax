// 거터 hover 카드: 요약, 전체 메시지, 작성자·날짜, 변경 파일 수, sha 복사, 이 줄로 바로 하는 일(커밋 상세, 부모 blame,
// 라인·파일 히스토리, 그래프).
// blame 결과에 든 정보는 바로 보여 주고, 전체 메시지·변경 파일 수는 commit_info로 받아 채운다.
import { useLayoutEffect, useRef, useState } from "react";
import { formatDate, isUncommitted, parseTz, relativeTime, shortSha } from "./format";
import type { AiAuthor } from "./ai";
import type { BlameBlock, BlameCommit, CommitInfo } from "./types";

export type InfoState = CommitInfo | "loading" | { error: string } | undefined;

/** 카드의 단추가 하는 일. 그 줄의 일이다(blame 화면의 키와 같다). */
export type CardAction = "commit" | "parent" | "lineHistory" | "history" | "graph";

const ACTIONS: { action: CardAction; label: string; keys: string; title: string }[] = [
  { action: "commit", label: "커밋 상세", keys: "Enter", title: "이 커밋의 상세(바뀐 파일과 diff)" },
  { action: "parent", label: "부모 blame", keys: ",", title: "이 줄을 바꾸기 전 리비전의 blame" },
  { action: "lineHistory", label: "라인 히스토리", keys: "L", title: "이 줄(고른 줄)을 바꾼 커밋" },
  { action: "history", label: "파일 히스토리", keys: "H", title: "이 파일을 바꾼 커밋" },
  { action: "graph", label: "그래프", keys: "G", title: "이 커밋을 고른 커밋 그래프" },
];

type Props = {
  commit: BlameCommit | undefined;
  block: BlameBlock;
  info: InfoState;
  /** 지금 보는 파일의 저장소 기준 경로. 커밋 당시 경로가 다르면(rename) 알려 준다. */
  path: string;
  anchor: DOMRect;
  pinned: boolean;
  nowMs: number;
  /** 이 줄을 쓴 AI 에이전트·사람(git-ai 노트). 노트가 없으면 null */
  ai?: AiAuthor | null;
  onCopy(sha: string): void;
  /** 단추: 이 줄에서 할 일 */
  onAction(action: CardAction): void;
  onClose(): void;
  onPointerInside(inside: boolean): void;
};

const GAP = 6;
const MARGIN = 8;

export function CommitCard({
  commit,
  block,
  info,
  path,
  anchor,
  pinned,
  nowMs,
  ai = null,
  onCopy,
  onAction,
  onClose,
  onPointerInside,
}: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: anchor.right + GAP, top: anchor.top });

  // 창 안에 들어오게 자리를 잡는다. 아래가 모자라면 위로 올린다.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const left = Math.max(MARGIN, Math.min(anchor.right + GAP, window.innerWidth - width - MARGIN));
    let top = anchor.top;
    if (top + height > window.innerHeight - MARGIN) top = Math.max(MARGIN, anchor.bottom - height);
    setPos({ left, top });
  }, [anchor, info]);

  const uncommitted = isUncommitted(block.sha);
  const style = { left: pos.left, top: pos.top };
  const events = { onMouseEnter: () => onPointerInside(true), onMouseLeave: () => onPointerInside(false) };

  if (uncommitted) {
    return (
      <div ref={ref} className="card" style={style} role="dialog" aria-label="커밋 안 됨" {...events}>
        <div className="card-summary uncommitted">커밋 안 됨</div>
        <p className="card-note">작업트리에서 바뀌었고 아직 커밋하지 않은 줄이다.</p>
        <CardActions
          actions={["parent", "lineHistory", "history"]}
          off={block.previous ? [] : ["parent"]}
          onAction={onAction}
        />
      </div>
    );
  }

  const loaded = typeof info === "object" && "sha" in info ? info : null;
  const summary = loaded?.summary ?? commit?.summary ?? "";
  const body = loaded ? loaded.message.split("\n").slice(1).join("\n").trim() : "";
  const authorTz = commit ? parseTz(commit.authorTz) : 0;
  const committerDiffers =
    commit && (commit.committer !== commit.author || Math.abs(commit.committerTime - commit.authorTime) > 60);

  return (
    <div
      ref={ref}
      className={pinned ? "card pinned" : "card"}
      style={style}
      role="dialog"
      aria-label={`커밋 ${shortSha(block.sha)}`}
      {...events}
    >
      <div className="card-summary">{summary}</div>
      {body && <pre className="card-body">{body}</pre>}
      <dl className="card-meta">
        {commit && (
          <>
            <dt>작성</dt>
            <dd>
              {commit.author} <span className="muted">{commit.authorMail}</span>
              <br />
              {formatDate(commit.authorTime, authorTz)} <span className="muted">({relativeTime(commit.authorTime, nowMs)})</span>
            </dd>
          </>
        )}
        {commit && committerDiffers && (
          <>
            <dt>커밋</dt>
            <dd>
              {commit.committer} · {formatDate(commit.committerTime, parseTz(commit.committerTz))}
            </dd>
          </>
        )}
        {ai && <AiRow ai={ai} />}
        <dt>sha</dt>
        <dd>
          <code>{shortSha(block.sha)}</code>{" "}
          <button type="button" className="link" onClick={() => onCopy(block.sha)}>
            복사 (y)
          </button>
        </dd>
        <dt>파일</dt>
        <dd>
          {info === "loading" || info === undefined
            ? "…"
            : loaded
              ? `${loaded.changedFiles}개 바뀜${loaded.parents.length > 1 ? ` · merge (부모 ${loaded.parents.length}개)` : ""}`
              : `불러오지 못했다: ${(info as { error: string }).error}`}
        </dd>
        {block.path !== path && (
          <>
            <dt>경로</dt>
            <dd>
              이 커밋에서는 <code>{block.path}</code>
            </dd>
          </>
        )}
        {commit?.boundary && (
          <>
            <dt>경계</dt>
            <dd>이력의 시작(이보다 앞은 보지 않았다)</dd>
          </>
        )}
      </dl>
      <CardActions
        actions={ACTIONS.map((a) => a.action)}
        off={block.previous ? [] : ["parent"]}
        onAction={onAction}
      />
      {pinned && (
        <button type="button" className="card-close" onClick={onClose} aria-label="닫기">
          ×
        </button>
      )}
    </div>
  );
}

/**
 * 이 줄로 바로 하는 일. 키는 단추 안에 흐리게 보인다. 할 수 없는 일(부모가 없는 줄의 부모 blame)은 자리를 지킨 채
 * 흐리고 풍선 도움말에 까닭을 쓴다(꺼진 단추도 풍선 도움말이 뜨게 aria-disabled로 끈다).
 */
function CardActions({ actions, off, onAction }: { actions: CardAction[]; off: CardAction[]; onAction(action: CardAction): void }) {
  return (
    <div className="card-actions" role="group" aria-label="이 줄로 하는 일">
      {ACTIONS.filter((a) => actions.includes(a.action)).map(({ action, label, keys, title }) => {
        const disabled = off.includes(action);
        return (
          <button
            key={action}
            type="button"
            className={disabled ? "off" : undefined}
            aria-disabled={disabled || undefined}
            title={disabled ? "이 줄은 이 파일이 처음 생긴 커밋에서 왔다(부모에 이 파일이 없다)" : `${title} (${keys})`}
            onClick={() => !disabled && onAction(action)}
          >
            {label}
            <kbd>{keys}</kbd>
          </button>
        );
      })}
    </div>
  );
}

/** git-ai 노트의 이 줄 작성자 */
function AiRow({ ai }: { ai: AiAuthor }) {
  if (ai.kind === "human") {
    return (
      <>
        <dt>사람</dt>
        <dd>
          {ai.author} <span className="muted">(git-ai가 사람이 친 것으로 봄)</span>
        </dd>
      </>
    );
  }
  return (
    <>
      <dt>AI</dt>
      <dd className="card-ai">
        <span className="ai-tool">✦ {ai.tool || "AI"}</span>
        {ai.model && <span className="muted"> · {ai.model}</span>}
        {ai.human && <div>시킨 사람 {ai.human}</div>}
        {ai.stats && (
          <div className="muted">
            더함 {ai.stats.additions} · 지움 {ai.stats.deletions} · 받아들임 {ai.stats.accepted}
            {ai.stats.overridden > 0 && <> · 사람이 고침 {ai.stats.overridden}</>}
          </div>
        )}
        {ai.attributes.length > 0 && (
          <div className="muted">{ai.attributes.map(([k, v]) => `${k}=${v}`).join(" · ")}</div>
        )}
        {ai.messagesUrl && <div className="muted mono">{ai.messagesUrl}</div>}
      </dd>
    </>
  );
}

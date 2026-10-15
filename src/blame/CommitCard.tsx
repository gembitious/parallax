// 거터 hover 카드: 요약, 전체 메시지, 작성자·날짜, 변경 파일 수, sha 복사.
// blame 결과에 든 정보는 바로 보여 주고, 전체 메시지·변경 파일 수는 commit_info로 받아 채운다.
import { useLayoutEffect, useRef, useState } from "react";
import { formatDate, isUncommitted, parseTz, relativeTime, shortSha } from "./format";
import type { BlameBlock, BlameCommit, CommitInfo } from "./types";

export type InfoState = CommitInfo | "loading" | { error: string } | undefined;

type Props = {
  commit: BlameCommit | undefined;
  block: BlameBlock;
  info: InfoState;
  /** 지금 보는 파일의 저장소 기준 경로. 커밋 당시 경로가 다르면(rename) 알려 준다. */
  path: string;
  anchor: DOMRect;
  pinned: boolean;
  nowMs: number;
  onCopy(sha: string): void;
  /** 커밋 상세로 */
  onOpen(): void;
  onClose(): void;
  onPointerInside(inside: boolean): void;
};

const GAP = 6;
const MARGIN = 8;

export function CommitCard({ commit, block, info, path, anchor, pinned, nowMs, onCopy, onOpen, onClose, onPointerInside }: Props) {
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
        <dt>sha</dt>
        <dd>
          <code>{shortSha(block.sha)}</code>{" "}
          <button type="button" className="link" onClick={() => onCopy(block.sha)}>
            복사 (y)
          </button>{" "}
          <button type="button" className="link" onClick={onOpen}>
            상세 (Enter)
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
      {pinned && (
        <button type="button" className="card-close" onClick={onClose} aria-label="닫기">
          ×
        </button>
      )}
    </div>
  );
}

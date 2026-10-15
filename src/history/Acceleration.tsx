// 가속 설정: 저장소에 changed-path Bloom 필터가 없으면 히스토리 머리줄에 버튼을 보이고, 사용자가 확인하면 만든다.
// 저장소를 바꾸는 유일한 동작이다(CLAUDE.md). 백엔드: plx-core/src/commit_graph.rs.
import { Channel } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { formatCount } from "../blame/format";
import { call } from "../ipc";

/** plx-core/src/commit_graph.rs GraphStatus */
export type GraphStatus = {
  graph: boolean;
  commits: number;
  bloom: boolean;
  bloomVersion: number | null;
  headCovered: boolean;
  /** 쓸 때 필터 버전(git 2.46부터 2) */
  writeVersion: number;
};

/** plx-core/src/commit_graph.rs Progress */
type Progress = { stage: string; percent: number | null };

const accelerated = (s: GraphStatus) => s.graph && s.bloom && s.headCovered;

type State =
  | { kind: "unknown" }
  | { kind: "known"; status: GraphStatus }
  | { kind: "confirming"; status: GraphStatus }
  | { kind: "running"; progress: Progress | null }
  | { kind: "failed"; message: string };

/**
 * `repo`(작업트리 경로)의 가속 상태. 가속돼 있으면 `가속됨` 표만 그린다. `request`가 바뀌면(명령 팔레트) 버튼을 누른
 * 것처럼 확인 창을 연다.
 */
export function Acceleration({
  repo,
  request = 0,
  onDone,
}: {
  repo: string;
  request?: number;
  onDone(message: string): void;
}) {
  const [state, setState] = useState<State>({ kind: "unknown" });
  const latest = useRef({ state, onDone });
  latest.current = { state, onDone };

  useEffect(() => {
    if (request === 0) return;
    const { state: now, onDone: tell } = latest.current;
    if (now.kind === "known" && !accelerated(now.status)) setState({ kind: "confirming", status: now.status });
    else if (now.kind === "known") tell("이미 가속돼 있다(commit-graph와 Bloom 필터가 있다).");
    else if (now.kind === "running") tell("가속 설정 중이다.");
    else if (now.kind === "failed") tell(`가속 상태를 모른다: ${now.message}`);
    else if (now.kind === "unknown") tell("가속 상태를 읽는 중이다. 잠시 뒤 다시 한다.");
  }, [request]);

  useEffect(() => {
    let disposed = false;
    call<GraphStatus>("repo_acceleration", { repo }).then(
      (status) => !disposed && setState({ kind: "known", status }),
      (e: unknown) => !disposed && setState({ kind: "failed", message: String(e) }),
    );
    return () => {
      disposed = true;
    };
  }, [repo]);

  const run = () => {
    const channel = new Channel<Progress>();
    channel.onmessage = (progress) => setState({ kind: "running", progress });
    setState({ kind: "running", progress: null });
    call<GraphStatus>("accelerate", { repo, onProgress: channel }).then(
      (status) => {
        setState({ kind: "known", status });
        onDone(
          `가속 설정을 마쳤다: 커밋 ${formatCount(status.commits)}개, Bloom 필터 v${status.bloomVersion ?? "?"}. 다음 요청부터 빨라진다.`,
        );
      },
      (e: unknown) => {
        setState({ kind: "failed", message: String(e) });
        onDone(`가속 설정 실패: ${String(e)}`);
      },
    );
  };

  switch (state.kind) {
    case "unknown":
      return null;
    case "failed":
      return (
        <span className="badge warn" title={state.message}>
          가속 상태 모름
        </span>
      );
    case "running":
      return (
        <span className="badge" title={state.progress?.stage}>
          가속 설정 중…{state.progress?.percent != null ? ` ${state.progress.percent}%` : ""}
        </span>
      );
    case "known":
    case "confirming": {
      const { status } = state;
      if (accelerated(status)) {
        return (
          <span
            className="badge"
            title={`commit-graph 커밋 ${formatCount(status.commits)}개, changed-path Bloom 필터 v${status.bloomVersion}`}
          >
            가속됨
          </span>
        );
      }
      return (
        <>
          <button
            type="button"
            className="toggle warn"
            onClick={() => setState({ kind: "confirming", status })}
            title="commit-graph와 Bloom 필터를 만들어 히스토리를 빠르게 한다"
          >
            가속 설정
          </button>
          {state.kind === "confirming" && (
            <Confirm status={status} onRun={run} onCancel={() => setState({ kind: "known", status })} />
          )}
        </>
      );
    }
  }
}

function Confirm({ status, onRun, onCancel }: { status: GraphStatus; onRun(): void; onCancel(): void }) {
  const command = `git${status.writeVersion === 2 ? " -c commitGraph.changedPathsVersion=2" : ""} commit-graph write --reachable --changed-paths`;
  const why = !status.graph
    ? "commit-graph가 없다."
    : !status.bloom
      ? "commit-graph에 변경 경로 Bloom 필터가 없다."
      : "마지막으로 쓴 뒤 생긴 커밋이 그래프에 없다.";
  return (
    <div
      className="overlay"
      role="dialog"
      aria-label="가속 설정"
      onClick={onCancel}
      onKeyDown={(e) => {
        // 히스토리 화면의 키(j/k, q …)가 아래 목록으로 가지 않게 여기서 멈춘다.
        e.stopPropagation();
        if (e.key === "Escape") {
          e.preventDefault();
          onCancel();
        }
      }}
    >
      <div className="help confirm" onClick={(e) => e.stopPropagation()}>
        <h2>가속 설정</h2>
        <p>
          {why} 파일·라인 히스토리는 Bloom 필터가 있으면 몇 배 빨라진다(필터가 없으면 git이 커밋마다 트리를
          비교한다).
        </p>
        <p>실행할 명령:</p>
        <pre className="mono">{command}</pre>
        <ul>
          <li>
            <code>.git/objects/info</code> 아래에 파일만 생긴다. 커밋·브랜치·작업트리는 바뀌지 않는다.
          </li>
          <li>이후 git gc도 필터를 유지한다.</li>
          <li>큰 저장소는 몇십 초 걸린다(git/git 커밋 8만 개 약 10초). 그동안 히스토리는 그대로 쓸 수 있다.</li>
          {status.writeVersion === 1 && (
            <li>
              이 git은 필터 v1만 쓴다. 다른 플랫폼과 .git을 함께 쓰면 한글 경로의 필터가 맞지 않을 수 있어 git 2.46
              이상을 권한다.
            </li>
          )}
        </ul>
        <div className="confirm-buttons">
          <button type="button" className="toggle" onClick={onCancel}>
            취소 (Esc)
          </button>
          <button type="button" className="toggle primary" onClick={onRun} autoFocus>
            실행 (Enter)
          </button>
        </div>
      </div>
    </div>
  );
}

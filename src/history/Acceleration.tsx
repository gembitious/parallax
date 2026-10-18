// 가속 설정: 저장소에 changed-path Bloom 필터가 없으면 히스토리·그래프 머리줄에 버튼을 보이고, 사용자가 확인하면
// 만든다. 저장소를 바꾸는 유일한 동작이다(CLAUDE.md). 백엔드: plx-core/src/commit_graph.rs.
import { Channel } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { formatCount } from "../blame/format";
import { call } from "../ipc";

/** plx-core/src/commit_graph.rs GraphStatus */
export type GraphStatus = {
  /** git이 읽는 commit-graph가 있다 */
  graph: boolean;
  commits: number;
  /** git이 쓰는 Bloom 필터가 모든 층에 있다 */
  bloom: boolean;
  bloomVersion: number | null;
  headCovered: boolean;
  /** 지금 쓰면 git이 만들 필터 버전 */
  writeVersion: number;
  /** 가속 설정을 할 수 없거나 해도 소용없는 까닭(얕은 클론 등) */
  blocked: string | null;
  /** 이 앱이 지금 쓰고 있다 */
  writing: boolean;
};

/** plx-core/src/commit_graph.rs Progress */
type Progress = { stage: string; percent: number | null };

const accelerated = (s: GraphStatus) => s.graph && s.bloom;

/** 쓰기를 시작하거나 끝냈다: 다른 화면(히스토리·그래프)의 가속 표시도 상태를 다시 읽는다. */
const CHANGED = "plx-acceleration-changed";

/** 이 창에서 쓰는 중인 저장소. 같은 저장소의 다른 화면은 끝날 때까지 "설정 중"을 보이고 버튼을 감춘다. */
const writes = new Map<string, Promise<GraphStatus>>();

type State =
  | { kind: "unknown" }
  | { kind: "known"; status: GraphStatus }
  | { kind: "confirming"; status: GraphStatus }
  | { kind: "running"; progress: Progress | null }
  | { kind: "failed"; message: string };

/**
 * `repo`(작업트리 경로)의 가속 상태. 가속돼 있으면 `가속됨` 표만 그린다. `request`가 바뀌면(명령 팔레트) 버튼을 누른
 * 것처럼 확인 창을 연다. 상태를 아직 읽는 중이면 기억해 뒀다가 읽은 뒤 연다. 상태는 처음과, 쓰기를 시작·끝냈을 때,
 * 창으로 돌아왔을 때(밖에서 `git gc`를 했을 수 있다) 읽는다. 확인 창을 열고 닫으면 `onDialog`로 알리고(화면이 다시
 * 열기를 미루고 늦게 온 이동을 버린다), 화면이 숨으면(`active`가 꺼지면) 닫는다.
 */
export function Acceleration({
  repo,
  request = 0,
  active = true,
  onDone,
  onDialog,
}: {
  repo: string;
  request?: number;
  active?: boolean;
  onDone(message: string): void;
  onDialog?(open: boolean): void;
}) {
  const [state, setState] = useState<State>({ kind: "unknown" });
  const latest = useRef({ state, onDone, onDialog });
  latest.current = { state, onDone, onDialog };
  /** 상태를 읽기 전에 온 요청 */
  const pending = useRef(false);
  /** 읽을 때마다 올린다. 늦게 온 옛 읽기는 버린다. */
  const reads = useRef(0);

  /** 요청(팔레트)에 답한다: 할 수 있으면 확인 창, 아니면 왜 열지 않는지 알린다. */
  const answer = (now: State) => {
    const tell = latest.current.onDone;
    if (now.kind === "known") {
      const status = now.status;
      if (status.writing) tell("가속 설정 중이다.");
      else if (accelerated(status) && status.headCovered) tell("이미 가속돼 있다(commit-graph와 Bloom 필터가 있다).");
      else if (status.blocked) tell(`가속 설정을 할 수 없다: ${status.blocked}`);
      else setState({ kind: "confirming", status });
    } else if (now.kind === "running") tell("가속 설정 중이다.");
    else if (now.kind === "failed") {
      tell(`가속 상태를 모른다: ${now.message}`);
      refresh();
    }
  };

  /** 상태를 다시 읽는다. 이 창에서 쓰는 중이면 끝날 때(CHANGED)까지 "설정 중"이다. 확인 창은 연 채로 둔다. */
  const refresh = () => {
    const id = ++reads.current;
    if (writes.has(repo)) {
      setState((prev) => (prev.kind === "running" ? prev : { kind: "running", progress: null }));
      return;
    }
    const settle = (next: State) => {
      if (id !== reads.current) return;
      setState((prev) =>
        prev.kind === "confirming" && next.kind === "known" ? { kind: "confirming", status: next.status } : next,
      );
      if (pending.current) {
        pending.current = false;
        answer(next);
      }
    };
    call<GraphStatus>("repo_acceleration", { repo }).then(
      (status) => settle(status.writing ? { kind: "running", progress: null } : { kind: "known", status }),
      (e: unknown) => settle({ kind: "failed", message: String(e) }),
    );
  };

  useEffect(() => {
    if (request === 0) return;
    const now = latest.current.state;
    if (now.kind === "unknown") pending.current = true;
    else answer(now);
  }, [request]);

  const confirming = state.kind === "confirming";
  useEffect(() => {
    if (!confirming) return;
    latest.current.onDialog?.(true);
    return () => latest.current.onDialog?.(false);
  }, [confirming]);

  // 숨는 화면의 확인 창은 닫는다(돌아왔을 때 포커스 없이 떠 있지 않게).
  useEffect(() => {
    if (!active) setState((prev) => (prev.kind === "confirming" ? { kind: "known", status: prev.status } : prev));
  }, [active]);

  useEffect(() => {
    refresh();
    const onChanged = () => refresh();
    window.addEventListener(CHANGED, onChanged);
    window.addEventListener("focus", onChanged);
    return () => {
      reads.current++;
      window.removeEventListener(CHANGED, onChanged);
      window.removeEventListener("focus", onChanged);
    };
  }, [repo]);

  const run = () => {
    if (writes.has(repo)) return;
    const channel = new Channel<Progress>();
    channel.onmessage = (progress) =>
      setState((prev) => (prev.kind === "running" ? { kind: "running", progress } : prev));
    setState({ kind: "running", progress: null });
    const write = call<GraphStatus>("accelerate", { repo, onProgress: channel });
    writes.set(repo, write);
    window.dispatchEvent(new Event(CHANGED));
    write
      .then(
        (status) =>
          latest.current.onDone(
            `가속 설정을 마쳤다: 커밋 ${formatCount(status.commits)}개, Bloom 필터 v${status.bloomVersion ?? "?"}. 다음 요청부터 빨라진다.`,
          ),
        (e: unknown) => latest.current.onDone(`가속 설정 실패: ${String(e)}`),
      )
      .finally(() => {
        // 실패해도 상태를 다시 읽는다: 버튼이 돌아와 다시 할 수 있다.
        writes.delete(repo);
        window.dispatchEvent(new Event(CHANGED));
      });
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
        <span className="badge" title={state.progress?.stage ?? "commit-graph와 Bloom 필터를 쓰는 중"}>
          가속 설정 중…{state.progress?.percent != null ? ` ${state.progress.percent}%` : ""}
        </span>
      );
    case "known":
    case "confirming": {
      const { status } = state;
      const dialog = state.kind === "confirming" && (
        <Confirm status={status} onRun={run} onCancel={() => setState({ kind: "known", status })} />
      );
      if (accelerated(status)) {
        const uncovered = status.headCovered
          ? ""
          : ". HEAD가 그래프에 없다: 마지막으로 쓴 뒤의 커밋은 필터 없이 걷는다(git gc나 명령 팔레트의 가속 설정으로 다시 쓴다)";
        return (
          <>
            <span
              className="badge"
              title={`commit-graph 커밋 ${formatCount(status.commits)}개, changed-path Bloom 필터 v${status.bloomVersion}${uncovered}`}
            >
              가속됨
            </span>
            {dialog}
          </>
        );
      }
      if (status.blocked) {
        return (
          <span className="badge" title={status.blocked}>
            가속 불가
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
          {dialog}
        </>
      );
    }
  }
}

function Confirm({ status, onRun, onCancel }: { status: GraphStatus; onRun(): void; onCancel(): void }) {
  const why = !status.graph
    ? "commit-graph가 없다."
    : !status.bloom
      ? "commit-graph에 git이 쓰는 changed-path Bloom 필터가 없다."
      : "HEAD 커밋이 그래프에 없다(마지막으로 쓴 뒤의 커밋은 필터 없이 걷는다).";
  return (
    <div
      className="overlay"
      role="dialog"
      aria-label="가속 설정"
      onClick={onCancel}
      onKeyDown={(e) => {
        // 히스토리 화면의 키(j/k, q …)가 아래 목록으로 가지 않게 여기서 멈춘다.
        e.stopPropagation();
        // 팔레트에서 Enter로 고른 뒤 누르고 있으면 반복 입력이 포커스된 "실행"을 누른다. 새로 누른 키만 받는다.
        if (e.repeat && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          return;
        }
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
        <pre className="mono">git commit-graph write --reachable --changed-paths</pre>
        <ul>
          <li>
            <code>.git/objects/info</code> 아래에 파일만 생긴다. 커밋·브랜치·작업트리는 바뀌지 않는다.
          </li>
          <li>이후 git gc·fetch가 commit-graph를 다시 쓸 때도 필터를 유지한다.</li>
          <li>큰 저장소는 몇십 초 걸린다(git/git 커밋 8만 개 약 10초). 그동안 히스토리는 그대로 쓸 수 있다.</li>
          {status.writeVersion === 2 ? (
            <li>
              필터 v2로 쓴다(저장소의 필터나 commitGraph.changedPathsVersion을 따른다). git 2.46보다 낮은 git은 v2
              그래프를 다시 쓰다(git gc) 비정상 종료한다.
            </li>
          ) : (
            <li>
              필터는 git 기본값인 v1이다. 한글 경로의 필터 값은 CPU에 따라 달라, Linux ARM64(Apple Silicon의 Docker
              등)의 git과 이 .git을 함께 쓰면 그쪽 git의 한글 경로 히스토리가 틀릴 수 있다.
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

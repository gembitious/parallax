// 멈춰 있는 merge·rebase·cherry-pick·revert·am·bisect(M15)를 상태 막대에 보이는 글. 백엔드 plx-core/src/progress.rs가
// 읽은 것을 받아 이름표·풍선 도움말을 만든다. 순수 함수라 단위 테스트한다(progress.test.ts).
import { shortSha } from "../blame/format";

/** plx-core/src/progress.rs Operation */
export type Operation = "merge" | "rebase" | "cherryPick" | "revert" | "am" | "bisect";

/** plx-core/src/progress.rs Progress */
export type Progress = {
  operation: Operation;
  step: number | null;
  total: number | null;
  branch: string | null;
  onto: string | null;
  applying: { sha: string; summary: string | null }[];
  sequence: boolean;
};

const NAME: Record<Operation, string> = {
  merge: "merge",
  rebase: "rebase",
  cherryPick: "cherry-pick",
  revert: "revert",
  am: "am",
  bisect: "bisect",
};

/** 이어 가거나 그만두는 git 명령(앱은 저장소를 바꾸지 않는다: 터미널·lazygit의 몫이라 알리기만 한다) */
const CONTINUE: Record<Operation, string | null> = {
  merge: "git merge --continue · --abort",
  rebase: "git rebase --continue · --skip · --abort",
  cherryPick: "git cherry-pick --continue · --skip · --abort",
  revert: "git revert --continue · --skip · --abort",
  am: "git am --continue · --skip · --abort",
  bisect: "git bisect good · bad · reset",
};

/** 상태 막대의 이름표: `rebase 2/3 · 충돌 1` */
export function progressLabel(progress: Progress, conflicts: number): string {
  const step = progress.step !== null && progress.total !== null ? ` ${progress.step}/${progress.total}` : " 중";
  const name = `${NAME[progress.operation]}${step}`;
  return conflicts > 0 ? `${name} · 충돌 ${conflicts}` : name;
}

/** 적용 중인 커밋 한 줄: `abc1234 요약` */
export const applyingLine = (commit: Progress["applying"][number]) =>
  commit.summary === null ? `${shortSha(commit.sha)} (커밋 없음)` : `${shortSha(commit.sha)} ${commit.summary}`;

/** 무엇이 어디서 멈췄는지 */
export function progressHeadline(progress: Progress): string {
  const { operation, branch, onto } = progress;
  switch (operation) {
    case "rebase": {
      const what = branch ? `${branch} 브랜치` : "detached HEAD";
      return onto ? `${what}를 ${shortSha(onto)} 위로 rebase하는 중` : `${what}를 rebase하는 중`;
    }
    case "merge":
      return progress.applying.length > 1 ? `커밋 ${progress.applying.length}개를 합치는 중(octopus)` : "merge하는 중";
    case "cherryPick":
      return progress.sequence ? "여러 커밋을 cherry-pick하는 중" : "cherry-pick하는 중";
    case "revert":
      return progress.sequence ? "여러 커밋을 revert하는 중" : "revert하는 중";
    case "am":
      return "메일 패치를 적용하는 중(git am)";
    case "bisect":
      return branch ? `bisect하는 중(${branch}에서 시작)` : "bisect하는 중";
  }
}

/** 상태 막대 풍선 도움말 */
export function progressTitle(progress: Progress, conflicts: number): string {
  const lines = [progressHeadline(progress)];
  if (progress.step !== null && progress.total !== null) lines.push(`${progress.total}개 가운데 ${progress.step}번째`);
  const label = progress.operation === "merge" ? "합치는 커밋" : "적용 중인 커밋";
  for (const commit of progress.applying) lines.push(`${label}: ${applyingLine(commit)}`);
  if (conflicts > 0) lines.push(`충돌 파일 ${conflicts}개: 고친 뒤 git add로 해결했다고 알린다`);
  const next = CONTINUE[progress.operation];
  if (next) lines.push(`이어 가기·그만두기는 터미널에서: ${next}`);
  lines.push("누르면 메뉴");
  return lines.join("\n");
}

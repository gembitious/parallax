// AI 출처(M5): 백엔드(plx-core/src/ai_notes.rs, git-ai의 refs/notes/ai)가 준 (커밋, 경로)별 구간으로 blame 줄을
// 쓴 에이전트를 찾는다. 구간은 그 커밋 버전의 줄 번호라 블록의 원래 줄로 찾는다(git-ai blame과 같다). 순수 로직(ai.test.ts).

/** plx-core ai_notes::Author */
export type AiAuthor =
  | {
      kind: "agent";
      /** 예: claude, cursor, codex */
      tool: string;
      model: string;
      id: string;
      /** 에이전트에게 시킨 사람 */
      human: string | null;
      messagesUrl: string | null;
      attributes: [string, string][];
      /** 옛 형식 프롬프트에만 */
      stats: { additions: number; deletions: number; accepted: number; overridden: number } | null;
    }
  | { kind: "human"; author: string };

/** plx-core ai_notes::AiAttribution */
export type AiAttribution = {
  notes: boolean;
  authors: AiAuthor[];
  files: { sha: string; path: string; spans: [number, number, number][] }[];
};

/** 요청 하나 */
export type AiItem = { sha: string; path: string };

const key = (sha: string, path: string) => `${sha}\u0000${path}`;

export class AiOverlay {
  readonly authors: AiAuthor[] = [];
  /** 받은 응답마다 오른다(화면이 다시 그릴지) */
  version = 0;
  private readonly byKey = new Map<string, [number, number, number][]>();
  private readonly authorIndex = new Map<string, number>();
  private readonly asked = new Set<string>();

  /** 아직 묻지 않은 (커밋, 경로)면 true를 돌려주고 물은 것으로 적는다. */
  ask(sha: string, path: string): boolean {
    const k = key(sha, path);
    if (this.asked.has(k)) return false;
    this.asked.add(k);
    return true;
  }

  /** 응답을 합친다. 작성자 번호는 응답마다 다르므로 이 목록의 번호로 바꾼다. */
  merge(result: AiAttribution): void {
    const remap = result.authors.map((author) => {
      const id = JSON.stringify(author);
      let index = this.authorIndex.get(id);
      if (index === undefined) {
        index = this.authors.length;
        this.authors.push(author);
        this.authorIndex.set(id, index);
      }
      return index;
    });
    for (const file of result.files) {
      const spans = file.spans
        .map(([start, end, who]): [number, number, number] => [start, end, remap[who]])
        .sort((a, b) => a[0] - b[0]);
      this.byKey.set(key(file.sha, file.path), spans);
    }
    this.version++;
  }

  /** 커밋 `sha`의 `path` 버전 `line`번째 줄을 쓴 이. 노트에 없으면 null. */
  at(sha: string, path: string, line: number): AiAuthor | null {
    const index = this.indexAt(sha, path, line);
    return index === null ? null : this.authors[index];
  }

  indexAt(sha: string, path: string, line: number): number | null {
    const spans = this.byKey.get(key(sha, path));
    if (!spans) return null;
    let lo = 0;
    let hi = spans.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const [start, end, who] = spans[mid];
      if (line < start) hi = mid - 1;
      else if (line > end) lo = mid + 1;
      else return who;
    }
    return null;
  }

  get empty(): boolean {
    return this.byKey.size === 0;
  }
}

/** 화면에 쓸 이름: 에이전트는 도구 이름(모델), 사람은 이름 */
export function aiLabel(author: AiAuthor): string {
  return author.kind === "agent" ? author.tool || "AI" : author.author;
}

/** 줄별 작성자 목록에서 에이전트 줄 수와 도구별 줄 수(많은 순) */
export function aiSummary(authors: (AiAuthor | null)[]): { lines: number; tools: [string, number][] } {
  const tools = new Map<string, number>();
  let lines = 0;
  for (const author of authors) {
    if (author?.kind !== "agent") continue;
    lines++;
    const name = author.model ? `${author.tool} (${author.model})` : author.tool;
    tools.set(name, (tools.get(name) ?? 0) + 1);
  }
  return { lines, tools: [...tools].sort((a, b) => b[1] - a[1]) };
}

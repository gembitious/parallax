// 점진적으로 채워지는 blame 결과. 블록은 줄 순서와 무관하게 온다(git이 원인을 찾는 순서).
import { AiOverlay, type AiAuthor } from "./ai";
import type { BlameBlock, BlameCommit, Chunk } from "./types";

export class BlameModel {
  readonly lineCount: number;
  readonly blocks: BlameBlock[] = [];
  readonly commits = new Map<string, BlameCommit>();
  /** 원인을 아는 줄 수 */
  blamedLines = 0;
  /** add할 때마다 오른다. 화면이 다시 그릴지 판단한다. */
  version = 0;
  /** AI 출처(git-ai 노트). 받는 대로 채운다. */
  readonly ai = new AiOverlay();
  /** 줄(1부터) → 블록 번호 + 1. 0은 아직 모름. 큰 파일에서도 줄마다 객체를 만들지 않는다. */
  private readonly lineBlock: Uint32Array;

  constructor(lineCount: number) {
    this.lineCount = lineCount;
    this.lineBlock = new Uint32Array(lineCount + 1);
  }

  add(chunk: Chunk): void {
    for (const commit of chunk.commits) this.commits.set(commit.sha, commit);
    for (const block of chunk.blocks) {
      this.blocks.push(block);
      const index = this.blocks.length;
      const end = Math.min(this.lineCount, block.finalLine + block.numLines - 1);
      for (let line = Math.max(1, block.finalLine); line <= end; line++) {
        if (this.lineBlock[line] === 0) this.blamedLines++;
        this.lineBlock[line] = index;
      }
    }
    this.version++;
  }

  block(line: number): BlameBlock | undefined {
    if (!(line >= 1 && line <= this.lineCount)) return undefined;
    const index = this.lineBlock[line];
    return index === 0 ? undefined : this.blocks[index - 1];
  }

  /** 줄을 쓴 AI 에이전트·사람(git-ai 노트). 블록의 원래 커밋·경로·줄로 찾는다. */
  aiAt(line: number): AiAuthor | null {
    const block = this.block(line);
    return block ? this.ai.at(block.sha, block.path, block.origLine + (line - block.finalLine)) : null;
  }

  aiIndexAt(line: number): number | null {
    const block = this.block(line);
    return block ? this.ai.indexAt(block.sha, block.path, block.origLine + (line - block.finalLine)) : null;
  }

  sha(line: number): string | undefined {
    return this.block(line)?.sha;
  }

  commit(line: number): BlameCommit | undefined {
    const sha = this.sha(line);
    return sha === undefined ? undefined : this.commits.get(sha);
  }

  /**
   * 같은 커밋이 이어지는 줄 묶음의 첫 줄인가. 거터는 첫 줄에만 글을 쓴다.
   * 위 줄을 아직 모르면 첫 줄로 본다(위 줄이 채워지면 다시 계산한다).
   */
  isRunStart(line: number): boolean {
    const sha = this.sha(line);
    return sha !== undefined && (line === 1 || this.sha(line - 1) !== sha);
  }

  get complete(): boolean {
    return this.blamedLines >= this.lineCount;
  }
}

// 성능 측정 기록. HUD가 이 기록을 그린다.
// 시각은 모두 epoch ms(백엔드와 같은 벽시계)라 프로세스 사이의 구간도 뺄셈으로 잴 수 있다.
import { invoke } from "@tauri-apps/api/core";

export type Phase = { label: string; ms: number };

export type Entry = {
  id: number;
  /**
   * cold: 콜드 스타트, plx: 상주 인스턴스로 온 호출, blame: blame 요청(첫 줄 표시까지),
   * commit: 커밋 상세(첫 diff 표시까지), history: 파일 히스토리(첫 50개 표시까지), graph: 커밋 그래프(첫 화면 표시까지),
   * files: Quick Open(글자마다 결과 표시까지), ipc: 백엔드 요청, probe: 측정 버튼
   */
  kind: "cold" | "plx" | "blame" | "commit" | "history" | "graph" | "files" | "ipc" | "probe";
  label: string;
  totalMs: number;
  phases: Phase[];
  /** 성능 예산(docs/MILESTONES.md). 넘으면 HUD에서 빨갛게 표시한다. */
  budgetMs?: number;
  detail?: string;
  ok: boolean;
};

export type PerfInfo = {
  enabled: boolean;
  logging: boolean;
  processStartMs: number;
  mainStartMs: number;
};

const MAX_ENTRIES = 50;

let info: PerfInfo = { enabled: import.meta.env.DEV, logging: false, processStartMs: 0, mainStartMs: 0 };
let entries: Entry[] = [];
let nextId = 1;
const listeners = new Set<() => void>();

function notify() {
  for (const listener of listeners) listener();
}

export const nowMs = (): number => performance.timeOrigin + performance.now();

export async function initPerf(): Promise<PerfInfo> {
  info = await invoke<PerfInfo>("perf_info");
  notify();
  return info;
}

export const getPerfInfo = (): PerfInfo => info;
export const getEntries = (): Entry[] => entries;

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function record(entry: Omit<Entry, "id">): void {
  if (!info.enabled) return;
  const full: Entry = { ...entry, id: nextId++ };
  entries = [full, ...entries].slice(0, MAX_ENTRIES);
  notify();
  // PLX_PERF_LOG가 있을 때만 백엔드에 남긴다(bench/app-latency.sh가 읽는다). 이 호출 자체는 재지 않는다.
  if (info.logging) invoke("perf_record", { entry: full }).catch(() => {});
}

export function clearEntries(): void {
  entries = [];
  notify();
}

/** 이어진 시각들 → 구간 길이. [["a", t0], ["b", t1], ["c", t2]] → [{a: t1-t0}, {b: t2-t1}] */
export function phasesBetween(points: [string, number][]): Phase[] {
  return points.slice(0, -1).map(([label, t], i) => ({ label, ms: points[i + 1][1] - t }));
}

/**
 * 다음 페인트가 끝난 시각. rAF 콜백은 페인트 직전에 돌고, 그 안에서 보낸 메시지는 페인트 뒤에 처리된다.
 * 창이 숨겨져 있으면 페인트가 없으므로 timeoutMs 뒤 null.
 */
export function afterPaint(timeoutMs = 3000): Promise<number | null> {
  return new Promise((resolve) => {
    let done = false;
    const finish = (value: number | null) => {
      if (done) return;
      done = true;
      resolve(value);
    };
    requestAnimationFrame(() => {
      const channel = new MessageChannel();
      channel.port1.onmessage = () => finish(nowMs());
      channel.port2.postMessage(null);
    });
    setTimeout(() => finish(null), timeoutMs);
  });
}

export function median(values: number[]): number {
  return percentile(values, 0.5);
}

export function percentile(values: number[], p: number): number {
  if (values.length === 0) return NaN;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(p * sorted.length))];
}

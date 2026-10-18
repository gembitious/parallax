// 성능 측정 기록. HUD가 이 기록을 그린다.
// 시각은 모두 epoch ms(백엔드와 같은 벽시계)라 프로세스 사이의 구간도 뺄셈으로 잴 수 있다.
import { invoke } from "@tauri-apps/api/core";

export type Phase = { label: string; ms: number };

export type Entry = {
  id: number;
  /**
   * cold: 콜드 스타트, plx: 상주 인스턴스로 온 호출, blame: blame 요청(첫 줄 표시까지),
   * commit: 커밋 상세(첫 diff 표시까지), history: 파일 히스토리(첫 50개 표시까지), graph: 커밋 그래프(첫 화면 표시까지),
   * files: Quick Open(글자마다 결과 표시까지), scroll: 스크롤 한 번의 프레임(간격 p95), ipc: 백엔드 요청, probe: 측정 버튼
   */
  kind: "cold" | "plx" | "blame" | "commit" | "history" | "graph" | "files" | "scroll" | "ipc" | "probe";
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

/** 콜드 스타트 구간을 나누는 JS 쪽 지점(epoch ms). modulesMs: 불러온 모듈을 모두 실행한 시각(main.tsx) */
export const bootMarks = { modulesMs: 0 };

export async function initPerf(): Promise<PerfInfo> {
  info = await invoke<PerfInfo>("perf_info");
  if (info.enabled) watchLongFrames();
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

/** Long Animation Frames 항목에서 읽는 것만(TypeScript DOM 타입에 아직 없다) */
type LongAnimationFrame = PerformanceEntry & {
  renderStart: number;
  styleAndLayoutStart: number;
  scripts: { duration: number; invoker: string; sourceFunctionName: string; forcedStyleAndLayoutDuration: number }[];
};

/** 긴 프레임 하나(epoch ms). 앞 작업 · 렌더 콜백(rAF 등) · 스타일·레이아웃·페인트로 나눈다. */
type LongFrame = { start: number; end: number; tasks: number; callbacks: number; render: number; script: string };

const LONG_FRAMES_LIMIT = 32;
const longFrames: LongFrame[] = [];
let longFrameObserver: PerformanceObserver | null = null;

function keepLongFrames(list: PerformanceEntryList) {
  const origin = performance.timeOrigin;
  for (const entry of list as LongAnimationFrame[]) {
    const start = origin + entry.startTime;
    const end = start + entry.duration;
    const renderStart = entry.renderStart > 0 ? origin + entry.renderStart : end;
    const styleStart = entry.styleAndLayoutStart > 0 ? origin + entry.styleAndLayoutStart : end;
    const top = entry.scripts.reduce<LongAnimationFrame["scripts"][number] | null>(
      (a, s) => (a === null || s.duration > a.duration ? s : a),
      null,
    );
    const forced = top && top.forcedStyleAndLayoutDuration >= 1 ? `, 강제 레이아웃 ${top.forcedStyleAndLayoutDuration.toFixed(0)}` : "";
    const name = top ? [top.invoker, top.sourceFunctionName].filter(Boolean).join(" ") : "";
    longFrames.push({
      start,
      end,
      tasks: renderStart - start,
      callbacks: styleStart - renderStart,
      render: end - styleStart,
      script: top ? `${name} ${top.duration.toFixed(0)}ms${forced}` : "",
    });
  }
  if (longFrames.length > LONG_FRAMES_LIMIT) longFrames.splice(0, longFrames.length - LONG_FRAMES_LIMIT);
}

/**
 * 50ms 넘게 걸린 프레임을 모은다. Chromium(Windows WebView2)에만 있다(WebKit은 재지 않는다).
 * 첫 화면이 늦은 기록에 그 사이 가장 긴 프레임을 덧붙여, 늦은 것이 스크립트인지 스타일·레이아웃·페인트인지 본다.
 */
function watchLongFrames(): void {
  if (longFrameObserver || typeof PerformanceObserver === "undefined") return;
  if (!PerformanceObserver.supportedEntryTypes?.includes("long-animation-frame")) return;
  longFrameObserver = new PerformanceObserver((list) => keepLongFrames(list.getEntries()));
  longFrameObserver.observe({ type: "long-animation-frame", buffered: true });
}

/**
 * from~to(epoch ms)와 겹친 가장 긴 프레임의 설명. 브라우저가 재지 않거나 긴 프레임이 없으면 null.
 * 예: "긴 프레임 702ms(작업 12 · 렌더 콜백 30 · 스타일·레이아웃·페인트 660, FrameRequestCallback 25ms)"
 */
export function longFrameIn(fromMs: number, toMs: number): string | null {
  if (!longFrameObserver) return null;
  // 끝난 프레임의 항목이 아직 콜백으로 오지 않았을 수 있다.
  keepLongFrames(longFrameObserver.takeRecords());
  const hits = longFrames.filter((f) => f.end > fromMs && f.start < toMs);
  if (hits.length === 0) return null;
  const f = hits.reduce((a, b) => (b.end - b.start > a.end - a.start ? b : a));
  const count = hits.length > 1 ? `${hits.length}개, 가장 긴 것 ` : "";
  const parts = `작업 ${f.tasks.toFixed(0)} · 렌더 콜백 ${f.callbacks.toFixed(0)} · 스타일·레이아웃·페인트 ${f.render.toFixed(0)}`;
  return `긴 프레임 ${count}${(f.end - f.start).toFixed(0)}ms(${parts}${f.script ? `, ${f.script}` : ""})`;
}

export function median(values: number[]): number {
  return percentile(values, 0.5);
}

export function percentile(values: number[], p: number): number {
  if (values.length === 0) return NaN;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(p * sorted.length))];
}

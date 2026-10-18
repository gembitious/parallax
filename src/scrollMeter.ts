// 스크롤 측정: 스크롤하는 동안 프레임마다 "스크롤 이벤트 → 그 위치를 그림(렌더·캔버스)"과 프레임 간격을 모아,
// 멈추면 성능 HUD에 한 줄로 남긴다. 예산은 60fps(프레임 16.7ms, docs/MILESTONES.md "그래프 스크롤").
// 요약 계산은 순수 함수라 단위 테스트한다(scrollMeter.test.ts).
import { getPerfInfo, nowMs, percentile, record } from "./perf";

/** 60fps 한 프레임 */
export const FRAME_BUDGET_MS = 1000 / 60;
/** 스크롤 이벤트가 이만큼 없으면 멈춘 것으로 보고 기록한다 */
const IDLE_MS = 250;

export type ScrollStats = {
  /** 그린 프레임 수 */
  frames: number;
  /** 스크롤 이벤트 → 그 위치를 그림(ms) */
  workP50: number;
  workP95: number;
  workMax: number;
  /** 스크롤한 프레임의 간격(ms, 페인트 포함) */
  intervalP50: number;
  intervalP95: number;
  /** 간격이 한 프레임 반을 넘은 프레임(빠진 프레임) */
  late: number;
  /** 아직 오지 않은 행이 보인 프레임 */
  blank: number;
  /** 다시 그린 프레임 수와 그 가운데 캔버스 그리기(ms) */
  redraws: number;
  canvasP95: number;
};

export function summarize(
  work: readonly number[],
  intervals: readonly number[],
  blank: number,
  canvas: readonly number[] = [],
): ScrollStats {
  const max = work.reduce((m, v) => Math.max(m, v), 0);
  return {
    redraws: canvas.length,
    canvasP95: percentile([...canvas], 0.95),
    frames: work.length,
    workP50: percentile([...work], 0.5),
    workP95: percentile([...work], 0.95),
    workMax: max,
    intervalP50: percentile([...intervals], 0.5),
    intervalP95: percentile([...intervals], 0.95),
    late: intervals.filter((v) => v > FRAME_BUDGET_MS * 1.5).length,
    blank,
  };
}

/** 성능 HUD가 켜져 있을 때만 잰다. 화면마다 하나. */
export class ScrollMeter {
  private work: number[] = [];
  private intervals: number[] = [];
  private canvas: number[] = [];
  private blank = 0;
  /** 아직 그리지 않은 첫 스크롤 이벤트 시각 */
  private pendingSince: number | null = null;
  private scrolledSinceFrame = false;
  private lastFrame: number | null = null;
  private raf = 0;
  private idle: number | undefined;
  private startTop = 0;
  private top = 0;

  /** `label`: HUD 줄 이름, `rowHeight`: 지나간 행 수를 셀 때 */
  constructor(
    private readonly label: () => string,
    private readonly rowHeight: number,
  ) {}

  /** 스크롤 이벤트(`top`은 새 scrollTop) */
  scrolled(top: number): void {
    if (!getPerfInfo().enabled) return;
    if (this.work.length === 0 && this.pendingSince === null) this.startTop = this.top;
    this.top = top;
    if (this.pendingSince === null) this.pendingSince = nowMs();
    this.scrolledSinceFrame = true;
    if (this.raf === 0) this.raf = requestAnimationFrame(this.tick);
    window.clearTimeout(this.idle);
    this.idle = window.setTimeout(() => this.flush(), IDLE_MS);
  }

  /**
   * 스크롤한 위치를 그렸다(useLayoutEffect 끝, 다시 그릴 것이 없으면 스크롤 이벤트 안). `blank`면 아직 오지 않은
   * 행이 보였다. `canvasMs`는 이 프레임에 캔버스를 다시 그린 시간(다시 그리지 않았으면 null).
   */
  drawn(blank: boolean, canvasMs: number | null = null): void {
    if (this.pendingSince === null) return;
    this.work.push(nowMs() - this.pendingSince);
    this.pendingSince = null;
    if (blank) this.blank++;
    if (canvasMs !== null) this.canvas.push(canvasMs);
  }

  dispose(): void {
    window.clearTimeout(this.idle);
    cancelAnimationFrame(this.raf);
    this.raf = 0;
  }

  private tick = (t: number) => {
    if (this.lastFrame !== null && this.scrolledSinceFrame) this.intervals.push(t - this.lastFrame);
    this.scrolledSinceFrame = false;
    this.lastFrame = t;
    this.raf = requestAnimationFrame(this.tick);
  };

  private flush(): void {
    cancelAnimationFrame(this.raf);
    this.raf = 0;
    this.lastFrame = null;
    const { work, intervals, blank, canvas } = this;
    const rows = Math.round(Math.abs(this.top - this.startTop) / this.rowHeight);
    this.work = [];
    this.intervals = [];
    this.canvas = [];
    this.blank = 0;
    this.pendingSince = null;
    if (work.length < 2) return;
    const s = summarize(work, intervals, blank, canvas);
    record({
      kind: "scroll",
      label: `${this.label()} 스크롤`,
      totalMs: s.intervalP95,
      budgetMs: FRAME_BUDGET_MS,
      phases: [
        { label: "그리기 중앙값", ms: s.workP50 },
        { label: "그리기 p95", ms: s.workP95 },
        { label: "간격 중앙값", ms: s.intervalP50 },
      ],
      detail:
        `프레임 ${s.frames}개 · 행 ${rows}개 지나감 · 그리기 최대 ${s.workMax.toFixed(1)}ms · ` +
        `다시 그린 프레임 ${s.redraws}개(캔버스 p95 ${s.redraws > 0 ? s.canvasP95.toFixed(1) : "-"}ms) · ` +
        `빠진 프레임 ${s.late}개 · 빈 행이 보인 프레임 ${s.blank}개`,
      ok: true,
    });
  }
}

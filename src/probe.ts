// HUD의 "IPC 측정" 버튼. 플랫폼별 IPC·Channel 기본 비용을 잰다(M1 이후 예산 판단의 기준).
import { Channel, invoke } from "@tauri-apps/api/core";
import { median, nowMs, percentile, record } from "./perf";

type ProbeChunk = { index: number; sentMs: number; payload: string };

const PING_COUNT = 30;
const CHUNK_COUNT = 1000;
const CHUNK_SIZE = 100;

export async function runProbe(): Promise<void> {
  const rtts: number[] = [];
  for (let i = 0; i < PING_COUNT; i++) {
    const start = nowMs();
    await invoke("perf_ping");
    rtts.push(nowMs() - start);
  }
  record({
    kind: "probe",
    label: `IPC 왕복 ×${PING_COUNT}`,
    totalMs: median(rtts),
    phases: [
      { label: "중앙값", ms: median(rtts) },
      { label: "p90", ms: percentile(rtts, 0.9) },
      { label: "최대", ms: Math.max(...rtts) },
    ],
    ok: true,
  });

  let received = 0;
  let firstMs = NaN;
  let lastMs = NaN;
  let allReceived: () => void = () => {};
  const done = new Promise<void>((resolve) => (allReceived = resolve));
  const channel = new Channel<ProbeChunk>();
  channel.onmessage = () => {
    const t = nowMs();
    if (received === 0) firstMs = t;
    lastMs = t;
    received += 1;
    if (received === CHUNK_COUNT) allReceived();
  };
  const start = nowMs();
  await invoke("perf_stream", { count: CHUNK_COUNT, size: CHUNK_SIZE, onChunk: channel });
  const respondedMs = nowMs();
  // 청크와 명령 응답의 도착 순서는 보장되지 않는다. 남은 청크를 잠깐 기다린다.
  await Promise.race([done, new Promise((resolve) => setTimeout(resolve, 2000))]);
  record({
    kind: "probe",
    label: `Channel ${CHUNK_COUNT}×${CHUNK_SIZE}B`,
    totalMs: lastMs - start,
    phases: [
      { label: "첫 청크", ms: firstMs - start },
      { label: "마지막 청크", ms: lastMs - start },
      { label: "응답", ms: respondedMs - start },
    ],
    detail: `${received}/${CHUNK_COUNT}개 수신`,
    ok: received === CHUNK_COUNT,
  });
}

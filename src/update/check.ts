// 새 버전 알림(M8-2): 하루 한 번 GitHub Releases의 최신 버전(게시한 것만, 초안·시험판 제외)을 묻고 이 앱보다 새것이면
// 상태 막대·첫 화면에 알린다. 누르면 릴리스 페이지를 기본 브라우저로 연다(plx-core/src/release.rs). 설치는 사용자가 한다.
// 끌 수 있고(명령 팔레트) 끄면 묻지 않는다. 마지막으로 물은 때와 답은 이 컴퓨터에 남겨 다시 띄울 때마다 묻지 않는다.
import { useSyncExternalStore } from "react";
import { call } from "../ipc";

const LATEST_URL = "https://api.github.com/repos/gembitious/parallax/releases/latest";
const KEY = "plx.update";
const DAY_MS = 24 * 60 * 60 * 1000;
/** 앱이 뜬 뒤 첫 물음까지(첫 화면을 그리는 동안 묻지 않는다) */
const FIRST_DELAY_MS = 5000;
/** 상주하는 동안 하루가 지났는지 보는 간격 */
const POLL_MS = 60 * 60 * 1000;
const TIMEOUT_MS = 10_000;

type Version = [number, number, number];

/** `v1.2.3`·`1.2.3`. 다른 모양(시험판 `-rc1` 등)은 null */
export function parseVersion(text: string): Version | null {
  const m = /^v?(\d{1,9})\.(\d{1,9})\.(\d{1,9})$/.exec(text.trim());
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
}

/** `tag`가 `current`보다 새 버전인가 */
export function isNewer(tag: string, current: string): boolean {
  const a = parseVersion(tag);
  const b = parseVersion(current);
  if (!a || !b) return false;
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i];
  return false;
}

type Saved = { enabled: boolean; checkedAt: number; latest: string | null };

function readSaved(): Saved {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (typeof v === "object" && v !== null) {
      const o = v as Record<string, unknown>;
      return {
        enabled: o.enabled !== false,
        checkedAt: typeof o.checkedAt === "number" && Number.isFinite(o.checkedAt) ? o.checkedAt : 0,
        latest: typeof o.latest === "string" && parseVersion(o.latest) ? o.latest : null,
      };
    }
  } catch {
    // 처음처럼
  }
  return { enabled: true, checkedAt: 0, latest: null };
}

function writeSaved(): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(saved));
  } catch {
    // 다음 실행에 다시 묻는다.
  }
}

let saved = readSaved();
let current: string | null = null;
let allowed = false;
let snapshot: UpdateState = { enabled: saved.enabled, newer: null, current: null };
const listeners = new Set<() => void>();

export type UpdateState = {
  /** 알림이 켜져 있다 */
  enabled: boolean;
  /** 이 앱보다 새 버전의 태그(`v0.2.0`). 없거나 끄면 null */
  newer: string | null;
  /** 이 앱의 버전(앱 정보를 받기 전에는 null) */
  current: string | null;
};

function publish(): void {
  const newer = saved.enabled && current && saved.latest && isNewer(saved.latest, current) ? saved.latest : null;
  snapshot = { enabled: saved.enabled, newer, current };
  for (const listener of listeners) listener();
}

async function fetchLatest(): Promise<string> {
  const response = await fetch(LATEST_URL, {
    headers: { Accept: "application/vnd.github+json" },
    signal: AbortSignal.timeout(TIMEOUT_MS),
  });
  // 게시한 릴리스가 없으면 404다.
  if (response.status === 404) return "";
  if (!response.ok) throw new Error(`GitHub 응답 ${response.status}`);
  const body: unknown = await response.json();
  const tag = typeof body === "object" && body !== null ? (body as { tag_name?: unknown }).tag_name : undefined;
  if (typeof tag !== "string") throw new Error("릴리스 태그가 없는 응답");
  return tag;
}

let inFlight: Promise<void> | null = null;

/** 묻는다. `force`가 아니면 하루 안에 물었을 때 묻지 않는다. 실패하면 던진다(앞 답은 그대로). */
async function check(force: boolean): Promise<void> {
  if (!force && Date.now() - saved.checkedAt < DAY_MS) return;
  inFlight ??= fetchLatest()
    .then((tag) => {
      saved = { ...saved, checkedAt: Date.now(), latest: parseVersion(tag) ? tag : null };
      writeSaved();
      publish();
    })
    .finally(() => {
      inFlight = null;
    });
  return inFlight;
}

/** 앱 정보를 받으면 App이 부른다. `allow`가 아니면(개발 빌드, `PLX_UPDATE_CHECK=0`) 스스로 묻지 않는다. */
export function startUpdateChecks(version: string, allow: boolean): () => void {
  current = version;
  allowed = allow;
  publish();
  if (!allow) return () => {};
  const tick = () => {
    if (saved.enabled) check(false).catch((e: unknown) => console.warn("새 버전을 묻지 못했다", e));
  };
  const first = window.setTimeout(tick, FIRST_DELAY_MS);
  const poll = window.setInterval(tick, POLL_MS);
  return () => {
    window.clearTimeout(first);
    window.clearInterval(poll);
  };
}

/** 명령 팔레트의 "새 버전 확인": 바로 묻고 결과를 글로 돌려준다 */
export async function checkNow(): Promise<{ text: string; error: boolean }> {
  try {
    await check(true);
  } catch (e) {
    return { text: `새 버전을 묻지 못했다: ${e instanceof Error ? e.message : String(e)}`, error: true };
  }
  if (snapshot.newer) return { text: `새 버전 ${snapshot.newer}이 있다. 상태 막대에서 눌러 받는다.`, error: false };
  return { text: `최신 버전이다(${current ? `v${current}` : "이 앱"})`, error: false };
}

/** 알림 켜기·끄기. 켜면 하루가 지났을 때 묻는다. */
export function setUpdateCheck(enabled: boolean): void {
  saved = { ...saved, enabled };
  writeSaved();
  publish();
  if (enabled && allowed) check(false).catch((e: unknown) => console.warn("새 버전을 묻지 못했다", e));
}

export function openRelease(tag: string): void {
  call("open_release", { tag }).catch((e: unknown) => console.error(String(e)));
}

const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

export const updateState = (): UpdateState => snapshot;

export function useUpdate(): UpdateState {
  return useSyncExternalStore(subscribe, () => snapshot);
}

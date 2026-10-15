// blame 화면의 시간·sha 표기와 커밋 나이 히트맵.

/** 미커밋 줄에 git이 붙이는 sha(0으로만 된 40자). */
export const isUncommitted = (sha: string): boolean => sha.length > 0 && /^0+$/.test(sha);

export const shortSha = (sha: string): string => sha.slice(0, 8);

/** 거터·카드에 쓰는 상대 시간. 시계가 어긋나 미래 시각이면 "방금"으로 본다. */
export function relativeTime(epochSec: number, nowMs: number): string {
  const sec = Math.max(0, Math.floor(nowMs / 1000 - epochSec));
  if (sec < 60) return "방금";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}분 전`;
  const hour = Math.floor(min / 60);
  if (hour < 24) return `${hour}시간 전`;
  const day = Math.floor(hour / 24);
  if (day < 7) return `${day}일 전`;
  if (day < 30) return `${Math.floor(day / 7)}주 전`;
  if (day < 365) return `${Math.min(11, Math.max(1, Math.floor(day / 30)))}개월 전`;
  return `${Math.floor(day / 365)}년 전`;
}

/** git의 시간대 표기("+0900", "-0130") → UTC와의 차이(초). 알아볼 수 없으면 0. */
export function parseTz(tz: string): number {
  const m = /^([+-])(\d{2})(\d{2})$/.exec(tz);
  if (!m) return 0;
  const sec = Number(m[2]) * 3600 + Number(m[3]) * 60;
  return m[1] === "-" ? -sec : sec;
}

const pad = (n: number) => String(n).padStart(2, "0");

/** 커밋한 곳의 시간대로 쓴 날짜: "2023-05-12 14:03 +0900". */
export function formatDate(epochSec: number, tzOffsetSec: number): string {
  const d = new Date((epochSec + tzOffsetSec) * 1000);
  const sign = tzOffsetSec < 0 ? "-" : "+";
  const abs = Math.abs(tzOffsetSec);
  const tz = `${sign}${pad(Math.floor(abs / 3600))}${pad(Math.floor((abs % 3600) / 60))}`;
  return (
    `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())} ` +
    `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())} ${tz}`
  );
}

/** 히트맵 단계 수. 0이 가장 오래됨, HEAT_LEVELS - 1이 가장 최근. */
export const HEAT_LEVELS = 8;
const COLDEST_DAYS = 5 * 365;

/**
 * 커밋 나이 → 히트맵 단계. 하루 안이면 가장 뜨겁고 5년이 넘으면 가장 차갑다. 그 사이는 로그 눈금이다.
 * 파일 안의 상대 나이가 아니라 지금 기준이라, blame이 점진적으로 채워지는 동안 색이 바뀌지 않는다.
 */
export function heatLevel(epochSec: number, nowMs: number): number {
  const days = Math.max(0, nowMs / 1000 - epochSec) / 86400;
  const t = Math.min(1, Math.log1p(days) / Math.log1p(COLDEST_DAYS));
  return Math.round((1 - t) * (HEAT_LEVELS - 1));
}

/** "1,234" */
export const formatCount = (n: number): string => n.toLocaleString("ko-KR");

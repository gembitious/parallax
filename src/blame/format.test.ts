import { describe, expect, it } from "vitest";
import { HEAT_LEVELS, formatDate, heatLevel, isUncommitted, parseTz, relativeTime, shortSha } from "./format";

const NOW = Date.UTC(2026, 9, 15, 12, 0, 0); // 2026-10-15 12:00 UTC
const ago = (sec: number) => NOW / 1000 - sec;
const DAY = 86400;

describe("relativeTime", () => {
  it.each([
    [0, "방금"],
    [59, "방금"],
    [60, "1분 전"],
    [59 * 60, "59분 전"],
    [3600, "1시간 전"],
    [23 * 3600, "23시간 전"],
    [DAY, "1일 전"],
    [6 * DAY, "6일 전"],
    [7 * DAY, "1주 전"],
    [29 * DAY, "4주 전"],
    [30 * DAY, "1개월 전"],
    [364 * DAY, "11개월 전"],
    [365 * DAY, "1년 전"],
    [3 * 365 * DAY + 10 * DAY, "3년 전"],
  ])("%i초 전 → %s", (sec, expected) => {
    expect(relativeTime(ago(sec), NOW)).toBe(expected);
  });

  it("미래 시각은 방금", () => {
    expect(relativeTime(ago(-3 * DAY), NOW)).toBe("방금");
  });
});

describe("parseTz·formatDate", () => {
  it("시간대", () => {
    expect(parseTz("+0900")).toBe(9 * 3600);
    expect(parseTz("-0130")).toBe(-(3600 + 30 * 60));
    expect(parseTz("+0000")).toBe(0);
    expect(parseTz("bogus")).toBe(0);
  });

  it("커밋한 곳의 시간대로 쓴다", () => {
    const t = Date.UTC(2023, 4, 12, 5, 3) / 1000; // 2023-05-12 05:03 UTC
    expect(formatDate(t, parseTz("+0900"))).toBe("2023-05-12 14:03 +0900");
    expect(formatDate(t, parseTz("-0130"))).toBe("2023-05-12 03:33 -0130");
    // 날짜가 넘어가는 경우
    expect(formatDate(Date.UTC(2023, 11, 31, 20, 0) / 1000, parseTz("+0900"))).toBe("2024-01-01 05:00 +0900");
  });
});

describe("heatLevel", () => {
  it("최근일수록 뜨겁고 5년 넘으면 가장 차갑다", () => {
    expect(heatLevel(ago(0), NOW)).toBe(HEAT_LEVELS - 1);
    expect(heatLevel(ago(5 * 365 * DAY), NOW)).toBe(0);
    expect(heatLevel(ago(20 * 365 * DAY), NOW)).toBe(0);
    expect(heatLevel(ago(-DAY), NOW)).toBe(HEAT_LEVELS - 1);
  });

  it("나이가 많을수록 단계가 내려간다", () => {
    const ages = [0, 1, 7, 30, 90, 365, 2 * 365, 5 * 365].map((d) => heatLevel(ago(d * DAY), NOW));
    for (let i = 1; i < ages.length; i++) expect(ages[i]).toBeLessThanOrEqual(ages[i - 1]);
    expect(new Set(ages).size).toBeGreaterThanOrEqual(6);
  });
});

describe("sha", () => {
  it("미커밋·짧은 sha", () => {
    expect(isUncommitted("0".repeat(40))).toBe(true);
    expect(isUncommitted("1" + "0".repeat(39))).toBe(false);
    expect(isUncommitted("")).toBe(false);
    expect(shortSha("1b019344c559bac26fac7ba37d9e3b80d57986cf")).toBe("1b019344");
  });
});

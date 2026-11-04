// 커밋 상세 diff의 보기(분할·통합). 화면마다 바꿀 수 있고(`u`), 마지막으로 고른 것이 새로 연 화면의 보기다
// (이 컴퓨터에 남긴다). 설정 화면도 여기를 고친다.
import type { DiffMode } from "./diff";

const KEY = "plx.diff.mode";

export function readDiffMode(): DiffMode {
  try {
    return localStorage.getItem(KEY) === "unified" ? "unified" : "split";
  } catch {
    return "split";
  }
}

export function writeDiffMode(mode: DiffMode) {
  try {
    localStorage.setItem(KEY, mode);
  } catch {
    // 저장하지 못해도 이번 실행에는 반영된다.
  }
}

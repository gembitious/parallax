// blame 거터 보이기. 켜면 줄마다 커밋(요약·작성자·나이 막대)을 거터에 쓰고, 끄면 커서 줄 끝에만 한 줄을 쓴다
// (inline.ts). 모든 탭이 함께 따르고 다음 실행에도 남는다(이 컴퓨터의 설정이라 브라우저 저장소에 둔다).
import { useSyncExternalStore } from "react";

const KEY = "plx.blame.gutter";
const listeners = new Set<() => void>();

function read(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

let shown = read();

export const blameGutterShown = () => shown;

export function toggleBlameGutter(): void {
  shown = !shown;
  try {
    localStorage.setItem(KEY, shown ? "1" : "0");
  } catch {
    // 저장하지 못해도 이번 실행에는 반영된다.
  }
  for (const listener of listeners) listener();
}

/** 바뀌면 부른다(메뉴 막대의 이름) */
export const subscribeBlameGutter = (listener: () => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

export function useBlameGutter(): boolean {
  return useSyncExternalStore(subscribeBlameGutter, blameGutterShown);
}

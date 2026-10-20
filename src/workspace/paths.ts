// 경로 복사(파일 트리·탭·커밋 상세의 우클릭 메뉴): 저장소 기준 경로와 전체 경로.
import { copyText } from "../clipboard";
import { toast } from "../Toasts";

/** 저장소(작업트리)의 전체 경로와 저장소 기준 경로(`/`로 나눈다)를 잇는다. Windows 경로면 `\`로 잇는다. */
export function absolutePath(repo: string, path: string): string {
  const windows = /^[A-Za-z]:[\\/]|^\\\\/.test(repo);
  if (windows) return `${repo.replace(/\//g, "\\").replace(/\\+$/, "")}\\${path.replace(/\//g, "\\")}`;
  return `${repo.replace(/\/+$/, "")}/${path}`;
}

/** 복사하고 알린다. */
export function copyPath(text: string): void {
  copyText(text).then(
    () => toast(`복사했다: ${text}`),
    (e: unknown) => toast(String(e), true),
  );
}

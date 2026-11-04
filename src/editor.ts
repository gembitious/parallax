// 에디터로 열기(`o`): 작업트리 파일을 VS Code·Zed 같은 에디터의 그 줄로 연다(리비전의 줄은 작업트리 줄로 옮긴다).
// 백엔드: src-tauri/src/git.rs open_in_editor, plx-core/src/editor.rs(에디터 고르기: 설정에서 고른 것, PLX_EDITOR, PATH의
// code·zed …).
import { call } from "./ipc";
import { getSettings } from "./settings/store";

export type EditorTarget = {
  /** 저장소 안의 아무 경로 */
  repo: string;
  /** 저장소 기준 경로. 없으면 작업트리 폴더를 연다 */
  path: string | null;
  /** `line`이 이 리비전의 줄이면 그 sha(작업트리 줄로 옮긴다). 작업트리면 null */
  rev: string | null;
  line: number | null;
};

type Opened = { editor: string; line: number | null };

/** 에디터로 열고 알림 한 줄을 돌려준다(실패하면 그 이유). */
export async function openInEditor(target: EditorTarget): Promise<string> {
  try {
    const opened = await call<Opened>("open_in_editor", { ...target, editor: getSettings().editor || null });
    const where = target.path === null ? "작업트리 폴더" : `${target.path}${opened.line ? `:${opened.line}` : ""}`;
    return `${opened.editor}에서 열었다: ${where}`;
  } catch (e) {
    return String(e);
  }
}

export type EditorChoices = {
  /** 고르지 않았을 때 쓸 에디터(PLX_EDITOR, PATH, 설치 위치). 없으면 null */
  auto: string | null;
  autoError: string | null;
  /** PATH에서 찾은 에디터들(spec은 설정에 넣을 값) */
  found: { name: string; spec: string }[];
};

/** 설정 화면의 에디터 목록 */
export const editorChoices = () => call<EditorChoices>("editor_choices");

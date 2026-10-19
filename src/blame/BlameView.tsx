// blame 뷰: 파일 내용을 먼저 그리고, 거터를 blame 결과로 점진적으로 채운다.
// 에디터(CodeMirror)는 React 밖에서 직접 다룬다. 이벤트 처리기는 한 번 만들어지므로 최신 상태는 ref로 읽는다.
import { useEffect, useMemo, useRef, useState } from "react";
import { Help } from "../Help";
import { copyText } from "../clipboard";
import { call } from "../ipc";
import { afterPaint, getPerfInfo, nowMs, record } from "../perf";
import { CommitCard, type InfoState } from "./CommitCard";
import { createEditor, restoreSearch, searchStateOf, type Editor, type SearchState } from "./editor";
import { formatCount, isUncommitted, shortSha } from "./format";
import { blameChanged } from "./gutter";
import { blameGutterShown, toggleBlameGutter, useBlameGutter } from "./gutterSetting";
import { BlameModel } from "./model";
import { aiSummary, type AiAttribution, type AiItem } from "./ai";
import type { Navigation } from "../nav";
import { headMoved, watchFile, watchRepo } from "../watch";
import { WorktreePicker } from "../worktree/WorktreePicker";
import { useQuickOpen } from "../quickopen/QuickOpen";
import { commonCommands, usePalette, type Command } from "../palette/Palette";
import { routeKey } from "../keys";
import { openInEditor } from "../editor";
import { toggleHud } from "../Hud";
import { gotoLine, openSearchPanel } from "@codemirror/search";
import { runScopeHandlers } from "@codemirror/view";
import { blameSwitch } from "../worktree/switch";
import { openBlame, type BlameTarget } from "./session";
import type { BlameBlock, BlameMeta, BlameStats, CommitInfo, Neighbor, Version } from "./types";
import { useMounted } from "../mounted";


type Props = {
  /** 열 파일과 리비전(작업트리면 rev가 null) */
  target: BlameTarget;
  /** 처음 선택할 줄 */
  line: number | null;
  /** 보이는 화면인가(보이는 탭의 스택 맨 위). 아래 화면·다른 탭의 화면은 숨긴 채 살려 둔다. */
  visible: boolean;
  /** 키를 받는 화면인가(보이고, 에디터 칸에 포커스가 있다) */
  active: boolean;
  /** 탐색 스택 깊이. 1이면 맨 아래다. */
  depth: number;
  /** 이 방향으로 가면 바로 아래 화면이다(`<`로 왔으면 `>`). 새로 쌓지 않고 뒤로 간다. */
  backDirection?: "older" | "newer";
  /** 콜드 스타트의 첫 요청. 창이 보일 때까지 페인트가 없어 blame 예산과 비교하지 않는다. */
  cold?: boolean;
  /** 첫 화면(내용 또는 오류)을 DOM에 넣었다. App이 plx 호출의 첫 페인트를 잰다. */
  onShown?(committedMs: number): void;
  /** `q`: 뒤로. 더 돌아갈 곳이 없으면 창을 숨긴다. */
  onBack(): void;
  onNavigate(navigation: Navigation): void;
  /** 다시 열기 전 화면의 검색(검색어·패널). 이 화면에서 되살린다. */
  search?: SearchState;
  /** 저장소가 바뀌었다(작업트리 blame의 HEAD 이동): 이 화면을 `line` 줄로 새로 연다. `search`는 새 화면으로 넘긴다. */
  onReload?(line: number | null, search?: SearchState): void;
  /** 파일을 열었다: 저장소(작업트리)와 저장소 기준 경로. 절대 경로로 연 화면(콜드 스타트)의 자리를 고친다. */
  onLocated?(repo: string, path: string): void;
  /** 상태 막대에 보일 커서 줄·열과 파일 정보. 커서가 옮겨 가거나 다시 보일 때 부른다. */
  onStatus?(status: EditorStatus): void;
};

/** 상태 막대의 줄·열을 누르면 보이는 코드 뷰가 줄 이동 칸을 연다. */
const GOTO_LINE_EVENT = "plx-goto-line";
export const requestGotoLine = () => window.dispatchEvent(new Event(GOTO_LINE_EVENT));

/** 상태 막대에 보이는 코드 뷰의 상태 */
export type EditorStatus = {
  line: number;
  col: number;
  /** 파일 인코딩(UTF-8, EUC-KR …). 바이너리·열지 못한 파일은 null */
  encoding: string | null;
  crlf: boolean;
};

type Progress =
  | { kind: "loading" }
  | { kind: "streaming"; blamed: number }
  | { kind: "done" }
  | { kind: "failed"; message: string };

type Hover = { line: number; anchor: DOMRect; pinned: boolean };

/** 에디터 키와 거터가 부르는 동작. 키 처리기는 true를 돌려주면 처리한 것이다. */
type Actions = {
  hover(line: number | null, anchor: DOMRect | null): void;
  pin(line: number): void;
  copySha(): boolean;
  enter(): boolean;
  parent(): boolean;
  older(): boolean;
  newer(): boolean;
  history(): boolean;
  lineHistory(): boolean;
  graph(): boolean;
  worktrees(): boolean;
  files(): boolean;
  card(): boolean;
  openEditor(): boolean;
  palette(): boolean;
  search(): boolean;
  back(): boolean;
  help(): boolean;
  escape(): boolean;
  toggleGutter(): boolean;
};

type NoEditorAction = "back" | "help" | "history" | "worktrees" | "graph" | "openEditor" | "older" | "newer" | "escape";

/** 코드 뷰가 없는 화면(바이너리, 열지 못한 파일)에서도 먹는 키. 나머지는 줄이 있어야 한다. */
const NO_EDITOR_KEYS = new Map<string, NoEditorAction>([
  ["q", "back"],
  ["?", "help"],
  ["H", "history"],
  ["W", "worktrees"],
  ["G", "graph"],
  ["o", "openEditor"],
  ["<", "older"],
  [">", "newer"],
  ["Escape", "escape"],
]);

const SHOW_DELAY_MS = 150;
const HIDE_DELAY_MS = 250;
const TOAST_MS = 1800;
/** 성능 예산(docs/MILESTONES.md): blame 첫 줄 표시 ≤ git 첫 출력 + 30ms, 캐시 히트 재표시 ≤ 16ms */
const FIRST_LINE_SLACK_MS = 30;
const CACHE_HIT_BUDGET_MS = 16;

/** git blame과 같은 줄 수로 보이게 한다: CRLF는 LF로, 끝의 개행 하나는 빈 줄을 만들지 않게 뗀다. */
export function displayText(text: string): { text: string; crlf: boolean } {
  const crlf = text.includes("\r\n");
  let out = crlf ? text.replace(/\r\n/g, "\n") : text;
  if (out.endsWith("\n")) out = out.slice(0, -1);
  return { text: out, crlf };
}

export function BlameView({
  target,
  line,
  visible,
  active,
  depth,
  backDirection,
  cold = false,
  search,
  onShown,
  onBack,
  onNavigate,
  onReload,
  onLocated,
  onStatus,
}: Props) {
  const { path, rev } = target;
  const container = useRef<HTMLDivElement>(null);
  const editor = useRef<Editor | null>(null);
  const model = useRef<BlameModel | null>(null);
  const nowAtOpen = useRef(Date.now());
  const [meta, setMeta] = useState<BlameMeta | null>(null);
  const [crlf, setCrlf] = useState(false);
  const [fatal, setFatal] = useState<string | null>(null);
  const [progress, setProgress] = useState<Progress>({ kind: "loading" });
  const [hover, setHover] = useState<Hover | null>(null);
  const [infos, setInfos] = useState<ReadonlyMap<string, InfoState>>(() => new Map());
  const [help, setHelp] = useState(false);
  const [worktrees, setWorktrees] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  /** AI 출처 응답을 받을 때마다 오른다(머리줄 요약을 다시 센다). */
  const [aiVersion, setAiVersion] = useState(0);
  /** 코드 뷰의 검색 패널(`/`)이 열려 있다 */
  const [searching, setSearching] = useState(false);
  /** 파일이나 HEAD가 바뀌어 다시 열어야 한다(열린 창·검색을 닫을 때까지 미뤘다) */
  const [changed, setChanged] = useState(false);
  /** HEAD가 움직였다는 마지막 알림의 HEAD(없었으면 undefined) */
  const [movedTo, setMovedTo] = useState<string | null | undefined>(undefined);
  const showGutter = useBlameGutter();
  /** 상태 막대에 마지막으로 알린 것(다시 보일 때 다시 알린다) */
  const status = useRef<EditorStatus | null>(null);
  // 저장소를 아직 모르면(meta 전) 연 파일 경로로 찾는다(저장소 안의 아무 경로면 된다).
  const quickOpen = useQuickOpen({
    repo: meta?.repo ?? target.repo ?? path,
    current: meta?.path ?? null,
    onNavigate,
    onClosed: () => editor.current?.view.focus(),
  });

  // 명령 팔레트: 키로 하는 일 모두와 마우스로만 되던 카드 고정. 동작은 actions(아래)를 부른다.
  const run = (name: keyof Actions) => () => void (actions.current[name] as () => boolean)();
  const commands: Command[] = [
    { label: "이 줄 커밋의 상세", keys: "Enter", run: run("enter") },
    { label: "커밋 카드 보이기·닫기", keys: "i", detail: "전체 메시지, 날짜, 바뀐 파일 수", run: run("card") },
    { label: "부모 리비전 blame (이 줄을 바꾸기 전)", keys: ",", run: run("parent") },
    { label: "이전 리비전", keys: "<", run: run("older") },
    { label: "다음 리비전", keys: ">", run: run("newer") },
    { label: "파일 히스토리", keys: "H", run: run("history") },
    { label: "라인 히스토리 (선택한 줄)", keys: "L", run: run("lineHistory") },
    { label: "커밋 그래프 (이 줄의 커밋)", keys: "G", run: run("graph") },
    { label: "커밋 sha 복사", keys: "y", run: run("copySha") },
    { label: "파일 안에서 찾기", keys: "/", run: run("search") },
    { label: "에디터로 열기", keys: "o", detail: "작업트리 파일의 이 줄", run: run("openEditor") },
    {
      label: showGutter ? "blame 거터 숨기기" : "blame 거터 보이기",
      keys: "b",
      detail: showGutter ? "커서 줄 끝에만 blame 한 줄" : "줄마다 커밋·작성자·나이",
      run: run("toggleGutter"),
    },
    { label: "줄로 이동", keys: "Ctrl+G", run: () => void (editor.current && gotoLine(editor.current.view)) },
    ...commonCommands({
      files: run("files"),
      worktrees: run("worktrees"),
      help: run("help"),
      back: run("back"),
      hud: getPerfInfo().enabled ? toggleHud : undefined,
    }),
  ];
  const palette = usePalette(commands, () => editor.current?.view.focus());

  const timers = useRef({ show: 0, hide: 0, toast: 0 });
  const pointerInCard = useRef(false);
  /** 이동 요청을 보내 놓고 답을 기다리는 중(키를 여러 번 눌러도 한 번만 간다) */
  const navigating = useRef(false);
  const mounted = useMounted();
  /** 키를 가져가는 창(도움말·워크트리·Quick Open·팔레트)이 열려 있다 */
  const overlay = help || worktrees || quickOpen.isOpen || palette.isOpen;
  const latest = useRef({ hover, help, meta, infos, visible, active, fatal, overlay });
  latest.current = { hover, help, meta, infos, visible, active, fatal, overlay };

  const showToast = (message: string) => {
    setToast(message);
    window.clearTimeout(timers.current.toast);
    timers.current.toast = window.setTimeout(() => setToast(null), TOAST_MS);
  };

  /** 파일을 아직 열지 못했다(여는 중이거나 실패했다). */
  const notOpened = () => showToast(latest.current.fatal ? "파일을 열지 못했다" : "아직 파일을 여는 중이다");

  const loadInfo = (sha: string) => {
    const repo = latest.current.meta?.repo;
    if (!repo || isUncommitted(sha) || latest.current.infos.has(sha)) return;
    const put = (value: InfoState) => setInfos((m) => new Map(m).set(sha, value));
    put("loading");
    call<CommitInfo>("commit_info", { repo, rev: sha }).then(put, (e: unknown) => put({ error: String(e) }));
  };

  /** 줄 `n`의 거터 칸 위치(키보드로 카드를 열 때). 거터를 껐으면 줄 끝의 blame 한 줄, 그것도 없으면 줄 맨 앞. */
  const anchorOf = (n: number): DOMRect | null => {
    const view = editor.current?.view;
    if (!view) return null;
    const line = view.state.doc.line(n);
    const coords = view.coordsAtPos(line.from);
    if (!coords) return null;
    const gutter = view.dom.querySelector(".cm-blame-gutter")?.getBoundingClientRect();
    if (gutter) return new DOMRect(gutter.left, coords.top, gutter.width, coords.bottom - coords.top);
    const inline = view.dom.querySelector(".cm-inline-blame")?.getBoundingClientRect();
    if (inline && view.state.doc.lineAt(view.state.selection.main.head).number === n) return inline;
    const end = view.coordsAtPos(line.to) ?? coords;
    return new DOMRect(end.right, coords.top, 1, coords.bottom - coords.top);
  };

  const cursorLine = () => {
    const view = editor.current?.view;
    return view ? view.state.doc.lineAt(view.state.selection.main.head).number : 1;
  };

  // 작업트리 blame: HEAD가 바뀌거나(이 파일을 커밋하는 등) 파일을 고치면(에이전트·편집기) 커서 줄을 지켜 다시 연다.
  // 고정 리비전은 바뀌지 않는다. 열기를 시작할 때 요청 대상으로 구독한다: meta를 기다리면 다시 열다 실패한 화면
  // (체크아웃으로 파일이 사라짐)은 파일이 돌아와도 더는 바뀌지 않는다.
  useEffect(() => {
    if (rev !== null || !onReload) return;
    const stopRepo = watchRepo(target.repo ?? path, (change) => {
      if (change.headChanged) setMovedTo(change.head);
    });
    const stopFile = watchFile(target.repo ? `${target.repo}/${path}` : path, () => setChanged(true));
    return () => {
      stopRepo();
      stopFile();
    };
    // 요청마다 새 컴포넌트다(App이 key를 준다).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 다시 열기는 화면을 새로 만든다. 보이는 화면에 창(도움말·워크트리·Quick Open·팔레트·고정 카드)이나 검색 패널이
  // 열려 있으면 닫을 때까지 미룬다: 바로 열면 그 창과 치던 글자가 사라지고 남은 글자가 화면 키가 된다(`o`가 에디터를
  // 띄우고 `q`가 창을 숨겼다). 검색어는 새 화면으로 넘긴다(`n`/`N`이 이어서 찾는다).
  const busy = active && (overlay || !!hover?.pinned || searching);
  // HEAD 알림은 이 화면이 읽은 HEAD와 다를 때만 다시 연다(열기 바로 전의 커밋이 늦게 알려진 것은 거른다).
  const stale = changed || headMoved(movedTo, meta ? meta.head : undefined, fatal !== null || progress.kind === "failed");
  useEffect(() => {
    if (!stale || busy || !onReload) return;
    const view = editor.current?.view;
    onReload(view ? cursorLine() : line, view ? (searchStateOf(view) ?? undefined) : search);
    // 다시 열면 App이 key를 바꿔 새 컴포넌트가 된다. cursorLine은 ref만 읽는다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stale, busy]);

  const openCard = (n: number, anchor: DOMRect, pinned: boolean) => {
    const sha = model.current?.sha(n);
    if (sha === undefined) return false;
    window.clearTimeout(timers.current.show);
    window.clearTimeout(timers.current.hide);
    setHover({ line: n, anchor, pinned });
    loadInfo(sha);
    return true;
  };

  const closeCard = () => {
    window.clearTimeout(timers.current.show);
    window.clearTimeout(timers.current.hide);
    setHover(null);
    editor.current?.view.focus();
  };

  /**
   * 이동할 곳을 받아 App에 넘긴다. 없으면(null) 알린다. 답을 기다리는 동안(큰 저장소의 `<`는 몇 초 걸린다) 이 화면을
   * 닫았거나(`q`, 새 `plx` 호출) 다른 화면을 위에 열었거나, Quick Open·팔레트 같은 창을 열었으면(그 창에 치던 글자가
   * 새 화면의 키가 된다) 이동하지 않는다.
   */
  const navigate = (request: Promise<Navigation | null>, none = "") => {
    if (navigating.current) return;
    navigating.current = true;
    request
      .then((navigation) => {
        if (!mounted.current || !latest.current.active || latest.current.overlay) return;
        if (navigation) onNavigate(navigation);
        else if (none) showToast(none);
      })
      .catch((e: unknown) => showToast(String(e)))
      .finally(() => {
        navigating.current = false;
      });
  };

  const neighbor = (direction: "older" | "newer") => {
    if (direction === backDirection) {
      onBack();
      return true;
    }
    const m = latest.current.meta;
    if (!m) return true;
    const current: Version = { rev: m.rev, path: m.path };
    navigate(
      call<Neighbor | null>("blame_neighbor", { repo: m.repo, current, direction, line: cursorLine() }).then((found) =>
        found
          ? { kind: direction, target: { repo: m.repo, path: found.version.path, rev: found.version.rev }, line: found.line }
          : null,
      ),
      direction === "older" ? "더 이전 리비전이 없다" : "더 최근 리비전이 없다(작업트리다)",
    );
    return true;
  };

  // 에디터·거터가 부르는 동작. 렌더마다 새로 만들어 최신 상태를 쓴다.
  const actions = useRef<Actions>(null as unknown as Actions);
  actions.current = {
    hover(n, anchor) {
      const current = latest.current.hover;
      if (current?.pinned) return;
      window.clearTimeout(timers.current.show);
      if (n === null || anchor === null) {
        window.clearTimeout(timers.current.hide);
        timers.current.hide = window.setTimeout(() => {
          if (!pointerInCard.current) setHover((h) => (h?.pinned ? h : null));
        }, HIDE_DELAY_MS);
        return;
      }
      if (current?.line === n) return;
      // 카드가 이미 떠 있으면 바로 옮기고, 처음이면 잠깐 기다린다(지나가는 마우스에 깜빡이지 않게).
      if (current) openCard(n, anchor, false);
      else timers.current.show = window.setTimeout(() => openCard(n, anchor, false), SHOW_DELAY_MS);
    },
    pin(n) {
      const anchor = anchorOf(n);
      if (anchor) openCard(n, anchor, true);
    },
    copySha() {
      const n = cursorLine();
      const sha = model.current?.sha(n);
      if (sha === undefined) showToast("이 줄은 아직 blame 정보가 없다");
      else if (isUncommitted(sha)) showToast("커밋하지 않은 줄이다");
      else
        copyText(sha).then(
          () => showToast(`복사했다: ${shortSha(sha)}`),
          (e: unknown) => showToast(String(e)),
        );
      return true;
    },
    enter() {
      const m = latest.current.meta;
      const block = model.current?.block(cursorLine());
      if (!m || !block) showToast("이 줄은 아직 blame 정보가 없다");
      else if (isUncommitted(block.sha)) showToast("커밋하지 않은 줄이다");
      else onNavigate({ kind: "commit", repo: m.repo, rev: block.sha, path: block.path });
      return true;
    },
    parent() {
      const m = latest.current.meta;
      const n = cursorLine();
      const block = model.current?.block(n);
      if (!m || !block) {
        showToast("이 줄은 아직 blame 정보가 없다");
        return true;
      }
      if (!block.previous) {
        showToast("이 줄은 이 파일이 처음 생긴 커밋에서 왔다(부모에 이 파일이 없다)");
        return true;
      }
      // 미커밋 줄은 작업트리 버전의 줄, 커밋된 줄은 그 커밋 버전의 줄에서 부모로 옮긴다.
      const uncommitted = isUncommitted(block.sha);
      const from: Version = uncommitted ? { rev: null, path: m.path } : { rev: block.sha, path: block.path };
      const fromLine = uncommitted ? n : block.origLine + (n - block.finalLine);
      const to: Version = { rev: block.previous.sha, path: block.previous.path };
      navigate(
        call<number>("blame_map_line", { repo: m.repo, from, to, line: fromLine }).then((mapped) => ({
          kind: "parent",
          target: { repo: m.repo, path: to.path, rev: to.rev },
          line: mapped,
        })),
      );
      return true;
    },
    older() {
      return neighbor("older");
    },
    newer() {
      return neighbor("newer");
    },
    history() {
      const m = latest.current.meta;
      if (!m) notOpened();
      else onNavigate({ kind: "history", target: { repo: m.repo, path: m.path, rev: m.rev } });
      return true;
    },
    lineHistory() {
      const m = latest.current.meta;
      const view = editor.current?.view;
      if (!m || !view) {
        if (m?.binary) showToast("바이너리 파일은 라인 히스토리를 보지 않는다");
        else notOpened();
        return true;
      }
      // 선택한 줄들(끝이 줄 맨 앞이면 그 줄은 뺀다), 선택이 없으면 커서 줄
      const { from, to } = view.state.selection.main;
      const doc = view.state.doc;
      const start = doc.lineAt(from).number;
      const last = doc.lineAt(to);
      const end = to > from && to === last.from ? Math.max(start, last.number - 1) : last.number;
      onNavigate({ kind: "history", target: { repo: m.repo, path: m.path, rev: m.rev }, lines: { start, end } });
      return true;
    },
    graph() {
      const m = latest.current.meta;
      if (!m) {
        notOpened();
        return true;
      }
      // 커서 줄의 커밋(미커밋 줄이면 이 리비전, 작업트리면 HEAD)을 골라서 연다.
      const sha = model.current?.sha(cursorLine());
      const rev = sha !== undefined && !isUncommitted(sha) ? sha : m.rev;
      onNavigate({ kind: "graph", repo: m.repo, rev });
      return true;
    },
    worktrees() {
      setWorktrees(true);
      return true;
    },
    files() {
      quickOpen.open();
      return true;
    },
    card() {
      // 커서 줄의 커밋 카드를 고정해 연다(이미 그 줄이 고정돼 있으면 닫는다). 마우스의 거터 클릭과 같다.
      const n = cursorLine();
      const current = latest.current.hover;
      if (current?.pinned && current.line === n) closeCard();
      else if (model.current?.sha(n) === undefined) showToast("이 줄은 아직 blame 정보가 없다");
      else actions.current.pin(n);
      return true;
    },
    openEditor() {
      const m = latest.current.meta;
      if (!m) notOpened();
      else openInEditor({ repo: m.repo, path: m.path, rev: m.rev, line: cursorLine() }).then(showToast);
      return true;
    },
    palette() {
      palette.open();
      return true;
    },
    search() {
      const view = editor.current?.view;
      if (view) openSearchPanel(view);
      return true;
    },
    back() {
      onBack();
      return true;
    },
    help() {
      setHelp(true);
      return true;
    },
    toggleGutter() {
      toggleBlameGutter();
      return true;
    },
    escape() {
      if (latest.current.help) {
        setHelp(false);
        return true;
      }
      if (latest.current.hover) {
        closeCard();
        return true;
      }
      return false; // 검색 패널이 열려 있으면 닫게 넘긴다
    },
  };

  useEffect(() => {
    const t0 = nowMs();
    let metaMs = NaN;
    let chunkMs = NaN;
    let contentPainted: Promise<number | null> = Promise.resolve(null);
    let gutterPainted: Promise<number | null> = Promise.resolve(null);
    let gotMeta = false;
    // AI 출처(git-ai 노트): 블록이 오는 대로 새 (커밋, 원래 경로)를 묻는다. 한 번에 하나만 보내고 그동안 온 것은 모아
    // 다음에 보낸다. 저장소에 노트가 없으면 더 묻지 않는다.
    let aiRepo: string | null = null;
    let aiPending: AiItem[] = [];
    let aiBusy = false;
    let aiOff = false;
    const askAi = () => {
      const blame = model.current;
      if (aiBusy || aiOff || !aiRepo || !blame || aiPending.length === 0) return;
      const items = aiPending;
      aiPending = [];
      aiBusy = true;
      call<AiAttribution>("ai_attribution", { repo: aiRepo, items }).then(
        (result) => {
          aiBusy = false;
          if (model.current !== blame) return;
          if (!result.notes) {
            aiOff = true;
            return;
          }
          if (result.files.length > 0) {
            blame.ai.merge(result);
            editor.current?.view.dispatch({ effects: blameChanged.of(null) });
            setAiVersion((v) => v + 1);
          }
          askAi();
        },
        (e: unknown) => {
          aiBusy = false;
          aiOff = true;
          console.error("AI 출처를 읽지 못했다", e);
        },
      );
    };
    const queueAi = (blocks: BlameBlock[]) => {
      const blame = model.current;
      if (!blame || aiOff) return;
      for (const block of blocks) {
        if (!isUncommitted(block.sha) && blame.ai.ask(block.sha, block.path)) {
          aiPending.push({ sha: block.sha, path: block.path });
        }
      }
      askAi();
    };
    let relPath = path;

    const report = async (stats: BlameStats, doneMs: number) => {
      const [content, gutter] = await Promise.all([contentPainted, gutterPainted]);
      const firstLine = gutter ?? content ?? doneMs;
      const phases = [
        { label: "내용 도착", ms: metaMs - t0 },
        { label: "내용 표시", ms: (content ?? NaN) - t0 },
        { label: "첫 묶음", ms: chunkMs - t0 },
        { label: "첫 줄 표시", ms: (gutter ?? NaN) - t0 },
        { label: "끝", ms: doneMs - t0 },
      ].filter((p) => Number.isFinite(p.ms));
      const budgetMs = cold
        ? undefined
        : stats.cached
          ? CACHE_HIT_BUDGET_MS
          : stats.firstByteMs === null
            ? undefined
            : Math.round(stats.firstByteMs + FIRST_LINE_SLACK_MS);
      record({
        kind: "blame",
        label: `blame ${relPath}${rev ? ` @${shortSha(rev)}` : ""}${stats.cached ? " (캐시)" : ""}${cold ? " (콜드 스타트)" : ""}`,
        totalMs: firstLine - t0,
        budgetMs,
        phases,
        detail: `${stats.cached ? "캐시" : `git 첫 출력 ${stats.firstByteMs?.toFixed(1) ?? "—"}`} · 블록 ${stats.blocks} · 커밋 ${stats.commits}`,
        ok: gutter !== null || stats.blocks === 0,
      });
    };

    const session = openBlame(target, (event) => {
      switch (event.kind) {
        case "meta": {
          gotMeta = true;
          metaMs = nowMs();
          const { kind: _kind, ...m } = event;
          relPath = m.path;
          aiRepo = m.repo;
          setMeta(m);
          if (target.repo === null) onLocated?.(m.repo, m.path);
          // 빈 검색어 Quick Open의 최근 연 파일
          call("files_touch", { repo: m.repo, path: m.path }).catch(() => {});
          const parent = container.current;
          if (m.content && parent) {
            const shown = displayText(m.content.text);
            setCrlf(shown.crlf);
            const blame = new BlameModel(m.content.lineCount);
            model.current = blame;
            const report = (n: number, col: number) => {
              status.current = { line: n, col, encoding: m.content?.encoding ?? null, crlf: shown.crlf };
              if (latest.current.visible) onStatus?.(status.current);
            };
            editor.current = createEditor(parent, {
              doc: shown.text,
              path: m.path,
              line,
              showGutter: blameGutterShown(),
              onCursor: report,
              gutter: {
                model: blame,
                nowMs: nowAtOpen.current,
                onHover: (n, anchor) => actions.current.hover(n, anchor),
                onClick: (n) => actions.current.pin(n),
              },
              keys: {
                copySha: () => actions.current.copySha(),
                enter: () => actions.current.enter(),
                parent: () => actions.current.parent(),
                older: () => actions.current.older(),
                newer: () => actions.current.newer(),
                history: () => actions.current.history(),
                lineHistory: () => actions.current.lineHistory(),
                graph: () => actions.current.graph(),
                worktrees: () => actions.current.worktrees(),
                files: () => actions.current.files(),
                card: () => actions.current.card(),
                openEditor: () => actions.current.openEditor(),
                toggleGutter: () => actions.current.toggleGutter(),
                palette: () => actions.current.palette(),
                back: () => actions.current.back(),
                help: () => actions.current.help(),
                escape: () => actions.current.escape(),
              },
              onSearchPanel: setSearching,
            });
            if (search) restoreSearch(editor.current.view, search);
            // 다른 칸(패널·트리)에 포커스가 있는 동안 다시 연 화면은 포커스를 가져가지 않는다.
            if (latest.current.active) editor.current.view.focus();
            setProgress({ kind: "streaming", blamed: 0 });
          } else {
            setProgress({ kind: "done" });
          }
          onShown?.(nowMs());
          contentPainted = afterPaint();
          break;
        }
        case "chunk": {
          const blame = model.current;
          if (!blame) break;
          blame.add(event);
          editor.current?.view.dispatch({ effects: blameChanged.of(null) });
          queueAi(event.blocks);
          if (Number.isNaN(chunkMs)) {
            chunkMs = nowMs();
            gutterPainted = afterPaint();
          }
          setProgress({ kind: "streaming", blamed: blame.blamedLines });
          break;
        }
        case "done":
          setProgress({ kind: "done" });
          void report(event.stats, nowMs());
          break;
        case "error":
          if (gotMeta) {
            setProgress({ kind: "failed", message: event.message });
          } else {
            setFatal(event.message);
            onShown?.(nowMs());
          }
          break;
        case "cancelled":
          break;
      }
    });

    const t = timers.current;
    return () => {
      session.cancel();
      editor.current?.destroy();
      editor.current = null;
      model.current = null;
      window.clearTimeout(t.show);
      window.clearTimeout(t.hide);
      window.clearTimeout(t.toast);
    };
    // 요청마다 새 컴포넌트다(App이 요청 id로 key를 준다). props가 바뀌어도 다시 열지 않는다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 키를 받는 화면이 되면(뒤로 와서, 탭을 골라서, 에디터를 눌러서) 에디터에 포커스를 준다. 키를 놓으면(숨거나
  // 다른 칸으로) 떠 있는 것을 닫는다(숨은 화면에 남은 창은 돌아왔을 때 포커스 없이 떠 있어 Esc로도 닫히지 않았다).
  useEffect(() => {
    if (active) {
      editor.current?.view.focus();
    } else {
      window.clearTimeout(timers.current.show);
      setHover(null);
      setHelp(false);
      setWorktrees(false);
      quickOpen.close();
      palette.close();
    }
    // close는 상태만 바꾼다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  // 거터 ↔ 커서 줄 끝의 blame 한 줄(모든 탭이 함께 바꾼다). 떠 있는 카드는 자리가 바뀌므로 닫는다.
  const shownGutter = useRef(showGutter);
  useEffect(() => {
    if (shownGutter.current === showGutter) return;
    shownGutter.current = showGutter;
    editor.current?.setGutter(showGutter);
    window.clearTimeout(timers.current.show);
    setHover(null);
  }, [showGutter]);

  // 상태 막대의 줄·열을 눌렀다: 보이는 코드 뷰가 줄 이동 칸을 연다.
  useEffect(() => {
    const onGoto = () => {
      const view = editor.current?.view;
      if (!latest.current.visible || !view) return;
      view.focus();
      gotoLine(view);
    };
    window.addEventListener(GOTO_LINE_EVENT, onGoto);
    return () => window.removeEventListener(GOTO_LINE_EVENT, onGoto);
  }, []);

  // 다시 보이면(탭을 골랐다) 상태 막대에 이 화면의 커서를 알린다.
  useEffect(() => {
    if (visible && status.current) onStatus?.(status.current);
    // onStatus는 보일 때의 것을 쓴다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible]);

  // 코드 뷰 밖(머리줄·카드·버튼을 누른 뒤, 클립보드 대신 쓰기 뒤)에 포커스가 있어도 화면 키가 먹게 한다.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!latest.current.active || routeKey(e, actions.current) !== "view") return;
      const target = e.target as HTMLElement | null;
      // 코드 뷰 안의 키는 코드 뷰의 키맵이 받는다.
      if (target?.closest(".cm-editor")) return;
      // 버튼에 포커스가 있으면 Enter·Space는 버튼이 받는다.
      if (target?.closest("button") && (e.key === "Enter" || e.key === " ")) return;
      const view = editor.current?.view;
      if (view) {
        // 코드 뷰의 키맵으로 처리하고 포커스를 돌려준다. 키가 입력 칸(검색 패널)을 열어 포커스를 가져갔으면 둔다.
        if (!runScopeHandlers(view, e, "editor")) return;
        e.preventDefault();
        if (!document.activeElement?.closest("input, textarea, .cm-editor")) view.focus();
        return;
      }
      const action = NO_EDITOR_KEYS.get(e.key);
      if (action && actions.current[action]()) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const blame = model.current;
  const lineCount = meta?.content?.lineCount ?? 0;
  // AI가 쓴 줄 수(git-ai 노트). 응답이 오거나 blame이 끝나면 다시 센다.
  const ai = useMemo(() => {
    const m = model.current;
    if (!m || m.ai.empty) return null;
    const authors = [];
    for (let line = 1; line <= m.lineCount; line++) authors.push(m.aiAt(line));
    return aiSummary(authors);
    // model은 ref라 aiVersion·진행 상태로 다시 센다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [aiVersion, progress.kind]);
  const cardBlock = hover && blame ? blame.block(hover.line) : undefined;

  return (
    <section className="blame">
      <header className="blame-header">
        <FilePath path={meta?.path ?? path} />
        <span className="badge" title={meta?.rev ?? "작업트리 파일"}>
          {meta?.rev ? shortSha(meta.rev) : rev ? shortSha(rev) : "작업트리"}
        </span>
        {meta?.content && meta.content.encoding !== "UTF-8" && (
          <span className="badge warn" title="파일 인코딩">
            {meta.content.encoding}
          </span>
        )}
        {crlf && <span className="badge">CRLF</span>}
        {ai && ai.lines > 0 && (
          <span
            className="badge ai"
            title={`git-ai 노트: AI가 쓴 줄\n${ai.tools.map(([tool, n]) => `${tool} ${formatCount(n)}줄`).join("\n")}`}
          >
            ✦ AI {formatCount(ai.lines)}줄
          </span>
        )}
        {depth > 1 && (
          <span className="badge" title="q로 앞 화면으로 돌아간다">
            뒤로 q · {depth - 1}
          </span>
        )}
        {stale && (
          <span className="badge warn" title="파일이나 HEAD가 바뀌었다. 열린 창·검색을 닫으면 다시 연다">
            바뀜
          </span>
        )}
        <span className="blame-status" aria-live="polite">
          <ProgressText progress={progress} blamed={blame?.blamedLines ?? 0} lines={lineCount} commits={blame?.commits.size ?? 0} />
        </span>
        <button
          type="button"
          className={showGutter ? "toggle on" : "toggle"}
          aria-pressed={showGutter}
          onClick={() => toggleBlameGutter()}
          title={showGutter ? "blame 거터 숨기기: 커서 줄 끝에만 blame 한 줄 (b)" : "blame 거터 보이기: 줄마다 커밋 (b)"}
        >
          blame 거터
        </button>
        <button type="button" className="icon" onClick={() => setHelp(true)} aria-label="단축키 (?)" title="단축키 (?)">
          ?
        </button>
      </header>
      <div className="blame-body">
        {/* CodeMirror가 직접 다루는 자리. React 자식과 섞지 않는다. */}
        <div className="blame-editor" ref={container} />
        {fatal && (
          <div className="blame-message error">
            <p>{fatal}</p>
          </div>
        )}
        {meta?.binary && (
          <div className="blame-message">
            <p>바이너리 파일이라 내용을 보여 주지 않는다.</p>
          </div>
        )}
      </div>
      {hover && cardBlock && meta && (
        <CommitCard
          commit={blame?.commits.get(cardBlock.sha)}
          block={cardBlock}
          info={infos.get(cardBlock.sha)}
          path={meta.path}
          anchor={hover.anchor}
          pinned={hover.pinned}
          ai={blame?.aiAt(hover.line) ?? null}
          nowMs={nowAtOpen.current}
          onCopy={(sha) =>
            copyText(sha).then(
              () => showToast(`복사했다: ${shortSha(sha)}`),
              (e: unknown) => showToast(String(e)),
            )
          }
          onOpen={() => onNavigate({ kind: "commit", repo: meta.repo, rev: cardBlock.sha, path: cardBlock.path })}
          onClose={closeCard}
          onPointerInside={(inside) => {
            pointerInCard.current = inside;
            if (inside) window.clearTimeout(timers.current.hide);
            else if (!latest.current.hover?.pinned) actions.current.hover(null, null);
          }}
        />
      )}
      {help && (
        <Help
          view="blame"
          onClose={() => {
            setHelp(false);
            editor.current?.view.focus();
          }}
        />
      )}
      {worktrees && (
        <WorktreePicker
          repo={meta?.repo ?? target.repo ?? target.path}
          onClose={() => {
            setWorktrees(false);
            editor.current?.view.focus();
          }}
          onPick={(worktree) => {
            setWorktrees(false);
            // 작업트리 blame은 그 워크트리의 같은 파일·줄, 리비전 blame은 같은 리비전(src/worktree/switch.ts)
            const next = blameSwitch(worktree, meta, editor.current ? cursorLine() : null);
            if (next) onNavigate({ ...next, worktree: true });
            else editor.current?.view.focus();
          }}
        />
      )}
      {quickOpen.overlay}
      {palette.overlay}
      {toast && (
        <div className="toast" role="status">
          {toast}
        </div>
      )}
    </section>
  );
}

function FilePath({ path }: { path: string }) {
  const cut = path.lastIndexOf("/") + 1;
  return (
    <span className="file-path mono" title={path}>
      {cut > 0 && <span className="file-dir">{path.slice(0, cut)}</span>}
      <span className="file-name">{path.slice(cut)}</span>
    </span>
  );
}

function ProgressText({
  progress,
  blamed,
  lines,
  commits,
}: {
  progress: Progress;
  blamed: number;
  lines: number;
  commits: number;
}) {
  switch (progress.kind) {
    case "loading":
      return <>여는 중…</>;
    case "streaming":
      return (
        <>
          blame {formatCount(blamed)} / {formatCount(lines)}줄
        </>
      );
    case "done":
      return lines > 0 ? <>커밋 {formatCount(commits)}개</> : null;
    case "failed":
      return <span className="error-text">blame 실패: {progress.message}</span>;
  }
}

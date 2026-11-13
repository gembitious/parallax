// 이미지 diff(M17): 바뀐 파일의 두 버전이 그림이면 줄 diff 대신 전·후 그림을 보인다.
// 바이트는 `file_image`가 그대로 보내고(JSON이 아니다) Blob 주소로 <img>에 넣는다. SVG도 <img>라 스크립트가 돌지 않는다.
// 분할 보기는 나란히, 통합 보기는 같은 자리에 겹쳐 손잡이로 밀어 본다. 두 그림은 같은 배율이다(크기 차이가 보인다).
import { useEffect, useLayoutEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject, type SyntheticEvent } from "react";
import { call } from "../ipc";
import type { DiffMode } from "./diff";
import { formatBytes } from "./detail";
import { dimensions, fitScale, nudge, resized, type Size } from "./image";
import type { FileChange, ImageSide, ImageVersions } from "./types";

type Loaded = { url: string; size: Size | null; broken: boolean };
type SideState = "loading" | { error: string } | Loaded | null;

/** 한쪽 그림을 받아 Blob 주소로 만든다. 없는 쪽(추가·삭제)은 null. 화면을 떠나면 주소를 놓는다. */
function useSide(repo: string, side: ImageSide | null, id: string | null, path: string, worktree: boolean): [SideState, (s: SideState) => void] {
  const [state, setState] = useState<SideState>(side ? "loading" : null);
  useEffect(() => {
    if (!side) {
      setState(null);
      return;
    }
    let url: string | null = null;
    let disposed = false;
    setState("loading");
    call<ArrayBuffer>("file_image", { repo, id, path, worktree }).then(
      (bytes) => {
        if (disposed) return;
        url = URL.createObjectURL(new Blob([bytes], { type: side.mime }));
        setState({ url, size: null, broken: false });
      },
      (e: unknown) => !disposed && setState({ error: String(e) }),
    );
    return () => {
      disposed = true;
      if (url) URL.revokeObjectURL(url);
    };
    // side는 형식·크기가 같으면 같은 그림이다(id가 열쇠).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repo, id, path, worktree, side?.mime]);
  return [state, setState];
}

const isLoaded = (s: SideState): s is Loaded => s !== null && s !== "loading" && !("error" in s);
const sizeOf = (s: SideState): Size | null => (isLoaded(s) ? s.size : null);

/** 자리(가로·세로)를 잰다. 패널 크기를 바꾸면 다시 맞춘다. */
function useRoom(ref: RefObject<HTMLElement | null>): Size {
  const [room, setRoom] = useState<Size>({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setRoom({ width: el.clientWidth, height: el.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);
  return room;
}

/** 프레임 한쪽 여백(.image-frame의 padding 8 + 테두리 1)과 나란히 보기의 틈 */
const FRAME_PAD = 9;
const GAP = 12;

export function ImageDiff({
  repo,
  file,
  image,
  mode,
  actual,
}: {
  repo: string;
  file: FileChange;
  image: ImageVersions;
  mode: DiffMode;
  /** 실제 크기(1:1)로 보인다. 아니면 자리에 맞춰 줄인다. */
  actual: boolean;
}) {
  const [old, setOld] = useSide(repo, image.old, file.oldId, file.oldPath ?? file.path, false);
  const [next, setNext] = useSide(repo, image.new, file.newId, file.path, file.worktree);
  const [at, setAt] = useState(50);
  const stage = useRef<HTMLDivElement>(null);
  const room = useRoom(stage);
  const both = image.old !== null && image.new !== null;
  const swipe = mode === "unified" && both;

  const sizes = [sizeOf(old), sizeOf(next)];
  const split = both && !swipe;
  const fit = fitScale(
    {
      width: (split ? (room.width - GAP) / 2 : room.width) - FRAME_PAD * 2,
      height: room.height - FRAME_PAD * 2,
    },
    sizes,
  );
  const scale = actual ? 1 : fit;

  const onLoad = (set: (s: SideState) => void, state: SideState) => (e: SyntheticEvent<HTMLImageElement>) => {
    if (!isLoaded(state)) return;
    const img = e.currentTarget;
    set({ ...state, size: { width: img.naturalWidth, height: img.naturalHeight } });
  };
  const onError = (set: (s: SideState) => void, state: SideState) => () => {
    if (isLoaded(state)) set({ ...state, broken: true });
  };

  /** 그림 하나: 받는 중·실패·그리지 못함 알림, 아니면 배율대로 그린 <img> */
  const picture = (state: SideState, set: (s: SideState) => void, label: string) => {
    if (state === null) return null;
    if (state === "loading") return <span className="muted">{label} 그림을 읽는 중…</span>;
    if ("error" in state) return <span className="image-error">{state.error}</span>;
    if (state.broken) return <span className="muted">이 그림 형식은 화면이 그리지 못한다</span>;
    const size = state.size;
    return (
      <img
        src={state.url}
        alt={label}
        draggable={false}
        style={size ? { width: size.width * scale, height: size.height * scale } : { visibility: "hidden" }}
        onLoad={onLoad(set, state)}
        onError={onError(set, state)}
      />
    );
  };

  const label = (title: string, side: ImageSide | null, state: SideState) =>
    side && (
      <span className="image-label">
        <span className={title === "바뀌기 전" ? "deleted" : "added"}>{title}</span>
        <span className="muted">
          {[dimensions(sizeOf(state)), formatBytes(side.bytes), side.mime.replace("image/", "").replace("+xml", "").replace("x-icon", "ico").toUpperCase()].filter(Boolean).join(" · ")}
        </span>
      </span>
    );

  // 밀어 보기: 프레임을 누르고 끌면 손잡이가 따라온다.
  const dragging = useRef(false);
  const moveTo = (e: ReactPointerEvent<HTMLDivElement>) => {
    const box = e.currentTarget.getBoundingClientRect();
    if (box.width > 0) setAt(Math.round(((e.clientX - box.left) / box.width) * 1000) / 10);
  };
  const frameSize = (() => {
    const known = sizes.filter((s): s is Size => s !== null);
    if (known.length === 0) return null;
    return {
      width: Math.max(...known.map((s) => s.width)) * scale,
      height: Math.max(...known.map((s) => s.height)) * scale,
    };
  })();
  const grown = resized(sizes[0], sizes[1]);

  return (
    <div className="image-diff">
      <div className="image-bar">
        {label("바뀌기 전", image.old, old)}
        {label("바뀐 뒤", image.new, next)}
        {grown && <span className="badge warn">크기 {grown}</span>}
        {swipe && (
          <input
            type="range"
            className="image-swipe"
            min={0}
            max={100}
            step={0.5}
            value={at}
            aria-label="바뀌기 전 ↔ 바뀐 뒤 경계"
            title="왼쪽은 바뀌기 전, 오른쪽은 바뀐 뒤 (←/→로 옮긴다)"
            onChange={(e) => setAt(Number(e.currentTarget.value))}
            onKeyDown={(e) => {
              if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
                e.preventDefault();
                e.stopPropagation();
                setAt((v) => nudge(v, (e.key === "ArrowLeft" ? -1 : 1) * (e.shiftKey ? 10 : 2)));
              }
            }}
          />
        )}
        <span className="muted image-scale">{Math.round(scale * 100)}%</span>
      </div>
      <div ref={stage} className={actual ? "image-stage actual" : "image-stage"}>
        {swipe ? (
          <div
            className="image-frame swipe"
            style={frameSize ?? undefined}
            onPointerDown={(e) => {
              dragging.current = true;
              e.currentTarget.setPointerCapture(e.pointerId);
              moveTo(e);
            }}
            onPointerMove={(e) => dragging.current && moveTo(e)}
            onPointerUp={() => {
              dragging.current = false;
            }}
          >
            <div className="image-layer">{picture(old, setOld, "바뀌기 전")}</div>
            <div className="image-layer" style={{ clipPath: `inset(0 0 0 ${at}%)` }}>
              {picture(next, setNext, "바뀐 뒤")}
            </div>
            <div className="image-handle" style={{ left: `${at}%` }} />
          </div>
        ) : (
          <>
            {image.old && <div className="image-frame">{picture(old, setOld, "바뀌기 전")}</div>}
            {image.new && <div className="image-frame">{picture(next, setNext, "바뀐 뒤")}</div>}
          </>
        )}
      </div>
    </div>
  );
}

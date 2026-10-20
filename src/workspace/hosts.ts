// 화면 자리(host): 화면(blame·히스토리·그래프·커밋 상세)마다 하나인 DOM 요소. 작업 공간은 화면을 React portal로 이
// 자리에 그리고 자리를 그룹·패널 칸 사이로 옮긴다. React는 portal의 자식을 새로 만들지 않으므로 탭을 다른 그룹으로
// 옮기거나 패널과 탭 사이로 옮겨도 화면이 그대로다(코드 뷰·목록·캔버스를 다시 만들거나 다시 읽지 않는다).
// 요소를 떼었다 붙이면 브라우저가 스크롤 자리와 포커스를 잃을 수 있다. 그래서 자리마다 스크롤한 요소의 자리와 마지막
// 포커스를 기억해 두었다가 옮긴 뒤나 다시 보일 때 되돌린다(칸이 통째로 빠져 떼어진 뒤에는 읽을 수 없어 미리 기억한다).
// WebKit(macOS·Linux)은 스크롤 자리를 지킨 채 옮기거나 다시 보이지만 그 안의 sticky 요소(코드 뷰의 줄 번호·blame 거터)를
// 다음 스크롤까지 그리지 않아, 되돌린 뒤 자리를 한 번 바꿨다 돌려 놓는다(`nudge`).

/** 화면 하나를 놓을 곳: 칸(`group:<key>`, `panel`), 보이는가, 키를 받는가 */
export type Placement = { key: number; container: string; visible: boolean; active: boolean };

type Memory = {
  /** 스크롤한 요소와 그 자리(scrollTop, scrollLeft) */
  scrolls: Map<Element, [number, number]>;
  /** 마지막으로 포커스를 받은 요소 */
  focused: Element | null;
};

export class ViewHosts {
  private hosts = new Map<number, HTMLDivElement>();
  private memory = new WeakMap<HTMLDivElement, Memory>();

  /** `key` 화면의 자리. 처음이면 만든다(아직 문서 밖이다. `place`가 칸에 넣는다). */
  get(key: number): HTMLDivElement {
    const found = this.hosts.get(key);
    if (found) return found;
    const host = document.createElement("div");
    host.className = "stack-entry";
    host.hidden = true;
    const memory: Memory = { scrolls: new Map(), focused: null };
    // scroll은 거품이 일지 않아 잡는 단계로 받는다.
    host.addEventListener(
      "scroll",
      (e) => {
        // 숨은 동안(그 칸이 가려졌다)은 자리를 읽을 수 없다(0이다).
        const target = e.target;
        if (!(target instanceof Element) || !host.isConnected || host.hidden || target.clientHeight === 0) return;
        memory.scrolls.set(target, [target.scrollTop, target.scrollLeft]);
        // 화면이 버린 요소(커밋 상세가 파일마다 새로 만드는 diff 등)는 잊는다. 쥐고 있으면 그 에디터가 통째로 남는다.
        for (const el of memory.scrolls.keys()) if (!host.contains(el)) memory.scrolls.delete(el);
        if (memory.focused && !host.contains(memory.focused)) memory.focused = null;
      },
      true,
    );
    host.addEventListener("focusin", (e) => {
      memory.focused = e.target instanceof Element ? e.target : null;
    });
    this.hosts.set(key, host);
    this.memory.set(host, memory);
    return host;
  }

  /**
   * 화면들을 칸(`containers`)에 놓고 보이기·숨기기를 맞춘다. 칸이 바뀐 자리는 옮기고, 옮겼거나 다시 보이는 자리는 스크롤을,
   * 옮긴 자리는 포커스를 되돌린다(숨은 채 옮긴 자리는 보일 때 되돌린다). `placements`에 없는 화면(닫았다)의 자리는 버린다.
   * React가 portal에 그린 뒤, 화면들의 layout effect보다 먼저 부른다.
   */
  place(placements: readonly Placement[], containers: ReadonlyMap<string, HTMLElement>): void {
    const moved: HTMLDivElement[] = [];
    const restore: HTMLDivElement[] = [];
    const keep = new Set<number>();
    for (const { key, container, visible } of placements) {
      keep.add(key);
      const host = this.get(key);
      const parent = containers.get(container);
      const move = parent !== undefined && host.parentElement !== parent;
      if (move) {
        parent.appendChild(host);
        moved.push(host);
      }
      if (visible && (move || host.hidden)) restore.push(host);
      if (host.hidden === visible) host.hidden = !visible;
    }
    for (const [key, host] of this.hosts) {
      if (keep.has(key)) continue;
      host.remove();
      this.hosts.delete(key);
    }
    for (const host of restore) this.restoreScroll(host);
    // 옮기느라 잃은 포커스를 키를 받는 화면에 돌려준다(그 화면이 옮겨졌을 때만).
    const lost = document.activeElement === null || document.activeElement === document.body;
    const active = placements.find((placement) => placement.active);
    const host = active && this.hosts.get(active.key);
    if (!lost || !host || !moved.includes(host)) return;
    const focused = this.memory.get(host)?.focused;
    if (focused instanceof HTMLElement && host.contains(focused)) focused.focus({ preventScroll: true });
  }

  private restoreScroll(host: HTMLDivElement): void {
    const memory = this.memory.get(host);
    if (!memory) return;
    for (const [el, [top, left]] of memory.scrolls) {
      if (!host.contains(el)) {
        memory.scrolls.delete(el);
        continue;
      }
      if (el.scrollTop !== top) el.scrollTop = top;
      if (el.scrollLeft !== left) el.scrollLeft = left;
      nudge(el);
    }
  }
}

/** 스크롤 자리를 1px 옮겼다 돌려 놓는다(같은 작업 안이라 그려지는 것은 돌려 놓은 자리다). 스크롤할 수 없으면 그대로다. */
function nudge(el: Element): void {
  const { scrollTop: top, scrollLeft: left } = el;
  if (el.scrollWidth > el.clientWidth) {
    el.scrollLeft = left > 0 ? left - 1 : left + 1;
    el.scrollLeft = left;
  } else if (el.scrollHeight > el.clientHeight) {
    el.scrollTop = top > 0 ? top - 1 : top + 1;
    el.scrollTop = top;
  }
}

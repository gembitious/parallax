// 클립보드 쓰기. WebView에서 클립보드 API가 막혀 있으면 execCommand로 대신한다.
export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // 고르기(select)가 포커스를 옮기므로, 지운 뒤 원래 자리(코드 뷰·목록)로 돌려준다. 안 그러면 body에 남아
    // 화면 키가 먹지 않는다.
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.append(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    previous?.focus({ preventScroll: true });
    if (!ok) throw new Error("클립보드에 쓰지 못했다");
  }
}

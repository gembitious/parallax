// 클립보드 쓰기. WebView에서 클립보드 API가 막혀 있으면 execCommand로 대신한다.
export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.append(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    if (!ok) throw new Error("클립보드에 쓰지 못했다");
  }
}

import { describe, expect, it } from "vitest";
import { routeKey } from "./keys";

type Mods = Partial<Record<"ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "defaultPrevented", boolean>>;

/** `inside`: 키를 친 곳이 맞는 선택자 */
function key(k: string, mods: Mods = {}, inside: string | null = null) {
  const target = { closest: (selector: string) => (inside && selector.includes(inside) ? {} : null) };
  const event = {
    key: k,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    defaultPrevented: false,
    target: target as unknown as EventTarget,
    prevented: false,
    preventDefault() {
      event.prevented = true;
    },
    ...mods,
  };
  return event;
}

function route(e: ReturnType<typeof key>) {
  const opened: string[] = [];
  const result = routeKey(e, { files: () => opened.push("files"), palette: () => opened.push("palette") });
  return { result, opened, prevented: e.prevented };
}

describe("routeKey", () => {
  it("Quick Open·명령 팔레트는 어느 화면에서든 연다", () => {
    expect(route(key("f"))).toEqual({ result: "done", opened: ["files"], prevented: true });
    expect(route(key("p", { ctrlKey: true }))).toEqual({ result: "done", opened: ["files"], prevented: true });
    expect(route(key(":", { shiftKey: true }))).toEqual({ result: "done", opened: ["palette"], prevented: true });
    expect(route(key("P", { metaKey: true, shiftKey: true }))).toEqual({
      result: "done",
      opened: ["palette"],
      prevented: true,
    });
    // 입력란에서도 Ctrl+P는 연다(f·:는 글자다)
    expect(route(key("p", { ctrlKey: true }, "input")).result).toBe("done");
    expect(route(key("f", {}, "input")).result).toBe("skip");
    expect(route(key(":", {}, "textarea")).result).toBe("skip");
  });

  it("화면 키", () => {
    expect(route(key("j"))).toEqual({ result: "view", opened: [], prevented: false });
    expect(route(key("G", { shiftKey: true })).result).toBe("view");
    expect(route(key("Escape")).result).toBe("view");
  });

  it("먹힌 키, 수정 키, 입력란은 화면이 받지 않는다", () => {
    expect(route(key("f", { defaultPrevented: true }))).toEqual({ result: "skip", opened: [], prevented: false });
    expect(route(key("j", { ctrlKey: true })).result).toBe("skip");
    expect(route(key("j", { altKey: true })).result).toBe("skip");
    expect(route(key("c", { metaKey: true })).result).toBe("skip");
    expect(route(key("j", {}, "input")).result).toBe("skip");
  });
});

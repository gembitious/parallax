import { afterEach, describe, expect, it } from "vitest";
import type { Command } from "../palette/Palette";
import {
  publish,
  resetRegistry,
  retract,
  runIn,
  setTarget,
  slotEntry,
  subscribe,
  targetHandles,
  targetView,
  workspaceCommands,
  type Handles,
  type SlotEntry,
} from "./registry";

const handles = (name: string, calls: string[] = []): Handles => ({
  palette: () => calls.push(`${name}:palette`),
  help: () => calls.push(`${name}:help`),
  files: () => calls.push(`${name}:files`),
});

const command = (label: string, more: Partial<Command> = {}): Command => ({ label, run: () => {}, ...more });

const entry = (owner: object, title: string, commands: Command[], h = handles(title)): SlotEntry => ({
  owner,
  title,
  commands,
  handles: h,
});

/** 알림 수를 센다 */
function counter() {
  let count = 0;
  const stop = subscribe(() => {
    count += 1;
  });
  return { get: () => count, stop };
}

afterEach(resetRegistry);

describe("명령 모음", () => {
  it("보이는 메뉴(이름·단축키)가 바뀔 때만 알리고, 실행할 함수는 새것으로 바꿔 둔다", () => {
    const owner = {};
    const notified = counter();
    publish("editor", entry(owner, "blame", [command("커밋", { id: "commit", keys: "Enter" })]));
    expect(notified.get()).toBe(1);
    let ran = "";
    publish("editor", entry(owner, "blame", [command("커밋", { id: "commit", keys: "Enter", run: () => (ran = "새것") })]));
    expect(notified.get()).toBe(1);
    slotEntry("editor")?.commands[0].run();
    expect(ran).toBe("새것");
    publish("editor", entry(owner, "blame", [command("커밋 상세", { id: "commit", keys: "Enter" })]));
    expect(notified.get()).toBe(2);
    // 같은 명령이라도 다른 화면(탭을 옮겼다)이 내면 알린다
    publish("editor", entry({}, "blame", [command("커밋 상세", { id: "commit", keys: "Enter" })]));
    expect(notified.get()).toBe(3);
    notified.stop();
  });

  it("거두기는 그 칸을 낸 화면만 한다(숨는 화면이 대신 들어온 화면의 명령을 지우지 않는다)", () => {
    const hiding = {};
    const shown = {};
    publish("editor", entry(hiding, "blame", [command("a")]));
    publish("editor", entry(shown, "blame", [command("b")]));
    retract("editor", hiding);
    expect(slotEntry("editor")?.commands.map((c) => c.label)).toEqual(["b"]);
    retract("editor", shown);
    expect(slotEntry("editor")).toBeUndefined();
  });

  it("명령을 보낼 화면과 창: 화면이 없으면 작업 공간의 창", () => {
    const calls: string[] = [];
    publish("workspace", entry({}, "작업 공간", [command("트리", { id: "tree", app: true })], handles("작업 공간", calls)));
    publish("panel", entry({}, "히스토리", [command("커밋 상세", { id: "commit" })], handles("히스토리", calls)));
    expect(workspaceCommands().map((c) => c.id)).toEqual(["tree"]);
    expect(targetView()).toBeNull();
    targetHandles()?.palette();
    setTarget("panel", () => {});
    expect(targetView()?.entry.title).toBe("히스토리");
    targetHandles()?.files();
    // 칸은 정했는데 그 칸에 낸 화면이 없다(아직 여는 중)
    setTarget("editor", () => {});
    expect(targetView()).toBeNull();
    targetHandles()?.help();
    expect(calls).toEqual(["작업 공간:palette", "히스토리:files", "작업 공간:help"]);
  });

  it("메뉴에서 실행하면 그 칸에 포커스를 준 뒤 실행한다", () => {
    const calls: string[] = [];
    setTarget("panel", (slot) => calls.push(`focus ${slot}`));
    runIn("editor", { run: () => calls.push("run") });
    expect(calls).toEqual(["focus editor", "run"]);
  });
});

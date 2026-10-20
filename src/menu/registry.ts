// 화면마다의 명령 모음. 명령 팔레트(화면마다), 메뉴 막대, 도구 막대가 같은 목록을 쓴다(CLAUDE.md 결정 기록 M6-3).
// 화면은 보이는 동안 자기 칸에 명령을 내고(publish) 숨거나 닫히면 거둔다(retract). 작업 공간은 자기 명령(트리·패널·탭)과
// 키를 받는 칸(target), 그 칸에 포커스를 주는 함수를 낸다. 메뉴 이름·명령 이름·단축키가 바뀔 때만 구독자(도구 막대,
// macOS 메뉴 막대)에 알린다. 실행할 함수는 렌더마다 새것으로 바꿔 둔다(지금 상태를 본다).
import { useEffect, useState, useSyncExternalStore } from "react";
import type { Command } from "../palette/Palette";

/** 명령을 내는 칸: 작업 공간, 보이는 탭의 화면(에디터), 열린 패널의 화면 */
export type Slot = "workspace" | ViewSlot;
export type ViewSlot = "editor" | "panel";

/** 칸마다 다른 창. 메뉴 막대·도구 막대가 키를 받는 화면의 것을 연다. */
export type Handles = {
  /** 명령 팔레트 */
  palette(): void;
  /** 단축키 도움말 */
  help(): void;
  /** 파일 찾기(Quick Open) */
  files(): void;
  /** 작업 공간: 저장소 그래프(아래 패널) */
  graph?(): void;
  /** 작업 공간: 탭 닫기, 탭이 없으면 창 숨기기 */
  closeTab?(): void;
};

export type SlotEntry = {
  owner: object;
  /** 메뉴 막대의 화면 메뉴 이름(blame, 히스토리 …) */
  title: string;
  commands: Command[];
  handles: Handles;
};

const slots = new Map<Slot, SlotEntry>();
const signatures = new Map<Slot, string>();
let target: ViewSlot | null = null;
let focusSlot: (slot: ViewSlot) => void = () => {};
let version = 0;
const listeners = new Set<() => void>();

const notify = () => {
  version += 1;
  for (const listener of listeners) listener();
};

/** 메뉴에 보이는 것(이름·단축키·자리). 실행할 함수는 넣지 않는다. */
export const signatureOf = (entry: Pick<SlotEntry, "title" | "commands">): string =>
  [entry.title, ...entry.commands.map((c) => [c.id ?? "", c.label, c.keys ?? "", c.detail ?? "", c.app ? "a" : ""].join("\u0001"))].join(
    "\u0002",
  );

export function publish(slot: Slot, entry: SlotEntry): void {
  const signature = signatureOf(entry);
  const same = slots.get(slot)?.owner === entry.owner && signatures.get(slot) === signature;
  slots.set(slot, entry);
  if (same) return;
  signatures.set(slot, signature);
  notify();
}

/** `owner`가 낸 명령을 거둔다. 그사이 다른 화면이 그 칸을 차지했으면 그대로 둔다. */
export function retract(slot: Slot, owner: object): void {
  if (slots.get(slot)?.owner !== owner) return;
  slots.delete(slot);
  signatures.delete(slot);
  notify();
}

/**
 * 작업 공간: 명령을 보낼 칸(키를 받는 칸, 트리면 보이는 화면의 칸, 화면이 없으면 null)과 칸에 포커스를 주는 함수.
 * `focus`는 화면에 포커스가 옮겨 간 뒤에 돌아와야 한다(runIn).
 */
export function setTarget(next: ViewSlot | null, focus: (slot: ViewSlot) => void): void {
  focusSlot = focus;
  if (next === target) return;
  target = next;
  notify();
}

export function slotEntry(slot: Slot): SlotEntry | undefined {
  return slots.get(slot);
}

/** 작업 공간의 명령(트리·패널·탭). 화면마다의 명령 팔레트에도 넣는다. */
export const workspaceCommands = (): Command[] => slots.get("workspace")?.commands ?? [];

/** 키를 받는 화면(그 칸과 낸 것). 없으면 null */
export function targetView(): { slot: ViewSlot; entry: SlotEntry } | null {
  if (target === null) return null;
  const entry = slots.get(target);
  return entry ? { slot: target, entry } : null;
}

/** 키를 받는 화면, 없으면 작업 공간의 창(명령 팔레트·도움말·파일 찾기) */
export function targetHandles(): Handles | null {
  return targetView()?.entry.handles ?? slots.get("workspace")?.handles ?? null;
}

/**
 * 메뉴·도구 막대에서 화면의 명령을 실행한다. 그 칸에 포커스가 없었으면(트리) 먼저 준다: 작업 공간은 그 화면이 키를
 * 받는 화면이 된 뒤(화면이 코드 뷰·목록에 포커스를 준 뒤)에 돌아온다. 키를 받지 않는 화면은 늦게 온 이동을 버리고,
 * 나중에 포커스를 가져가면 명령이 연 창(팔레트)의 포커스를 빼앗는다.
 */
export function runIn(slot: ViewSlot, command: Pick<Command, "run">): void {
  focusSlot(slot);
  command.run();
}

export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** 메뉴에 보이는 것이 바뀔 때마다 오르는 수. 도구 막대가 다시 그린다. */
export function useRegistryVersion(): number {
  return useSyncExternalStore(subscribe, () => version);
}

/** 화면: 보이는 동안 자기 칸에 명령을 낸다. 렌더마다 부른다(실행할 함수가 새 상태를 본다). */
export function usePublish(slot: Slot, visible: boolean, title: string, commands: Command[], handles: Handles): void {
  const [owner] = useState(() => ({}));
  useEffect(() => {
    if (visible) publish(slot, { owner, title, commands, handles });
    else retract(slot, owner);
  });
  useEffect(() => () => retract(slot, owner), [slot, owner]);
}

/** 테스트: 처음 상태로 */
export function resetRegistry(): void {
  slots.clear();
  signatures.clear();
  target = null;
  focusSlot = () => {};
  notify();
}

// 작업 공간 명령(트리·패널·탭). 화면마다의 명령 팔레트에도 넣는다(palette/Palette.tsx commonCommands): 작업 공간이
// 렌더마다 지금 상태로 바꿔 둔다.
import type { Command } from "../palette/Palette";

let current: Command[] = [];

export function setWorkspaceCommands(commands: Command[]): void {
  current = commands;
}

export const workspaceCommands = (): Command[] => current;

// 메뉴 막대를 지금 상태로 만든다: 명령 모음(작업 공간, 키를 받는 화면), 최근 저장소, 앱 동작(src/menu/model.ts).
// macOS 시스템 메뉴 막대(native.ts)와 Windows·Linux의 HTML 메뉴 막대(src/workspace/Toolbar.tsx)가 같이 쓴다.
import { invoke } from "@tauri-apps/api/core";
import { blameGutterShown, toggleBlameGutter } from "../blame/gutterSetting";
import { toggleHud } from "../Hud";
import { openNotices } from "../notices/Notices";
import { getPerfInfo } from "../perf";
import { installCli, openPath, openRepoDialog, recentRepos } from "../start/open";
import { IS_MAC } from "../workspace/keys";
import { buildMenus, type BarMenu, type MenuHandles } from "./model";
import { runIn, slotEntry, targetView } from "./registry";

export const hideWindow = () => invoke("hide_window").catch((e) => console.error("창 숨기기 실패", e));
const quit = () => invoke("quit_app").catch((e) => console.error("종료 실패", e));

export function currentMenus(mac = IS_MAC): BarMenu[] {
  const workspace = slotEntry("workspace");
  const target = targetView();
  // 키를 받는 화면의 창(팔레트·도움말·파일 찾기)은 그 칸에 포커스를 주고 연다: 트리에 있던 포커스가 그대로면 화면이
  // 키를 받지 않아 팔레트에서 고른 이동을 버린다.
  const inTarget = (open: () => void) => (target ? () => runIn(target.slot, { run: open }) : open);
  const own = target?.entry.handles ?? workspace?.handles;
  const handles: MenuHandles | null =
    workspace && own
      ? {
          palette: inTarget(own.palette),
          help: inTarget(own.help),
          files: inTarget(own.files),
          graph: workspace.handles.graph ?? (() => {}),
          closeTab: workspace.handles.closeTab ?? hideWindow,
        }
      : null;
  return buildMenus({
    mac,
    workspace: workspace?.commands ?? [],
    handles,
    view: target ? { title: target.entry.title, commands: target.entry.commands, run: (command) => runIn(target.slot, command) } : null,
    recent: recentRepos()
      .filter((repo) => !repo.missing)
      .map(({ name, display, path }) => ({ name, display, path })),
    tabs: workspace?.commands.some((command) => command.id === "closeTab") ?? false,
    gutterShown: blameGutterShown(),
    hud: getPerfInfo().enabled,
    hideWindow,
    actions: {
      openRepo: openRepoDialog,
      openRecent: openPath,
      installCli: () => void installCli(),
      notices: openNotices,
      quit,
      toggleGutter: toggleBlameGutter,
      hud: toggleHud,
    },
  });
}

// 첫 화면: 띄울 화면 없이 앱을 열었을 때(Finder·Dock·시작 메뉴). 저장소 폴더를 고르거나 최근 저장소를 연다.
// 폴더·파일을 창에 끌어 놓아도 열린다(App이 받는다). 터미널 명령 plx는 여기서 설치한다(쓰지 않아도 된다).
import { useEffect, useRef, useState } from "react";
import { isComposing } from "../keys";
import { openNotices } from "../notices/Notices";
import { OPEN_REPO_KEYS, startKey } from "./keys";
import {
  cliStatus,
  forgetRecent,
  installCli,
  openPath,
  openRepoDialog,
  useRecentRepos,
  type CliStatus,
} from "./open";

export function Start({ version }: { version: string | null }) {
  const { repos, loaded } = useRecentRepos();
  const [selected, setSelected] = useState(0);
  const [cli, setCli] = useState<CliStatus | null>(null);
  const [installing, setInstalling] = useState(false);
  const root = useRef<HTMLElement>(null);
  const list = useRef<HTMLUListElement>(null);

  useEffect(() => {
    root.current?.focus({ preventScroll: true });
    cliStatus().then(setCli, (e) => console.error("터미널 명령 상태를 받지 못했다", e));
  }, []);
  useEffect(() => setSelected((i) => Math.max(0, Math.min(i, repos.length - 1))), [repos.length]);
  useEffect(() => {
    list.current?.children[selected]?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  const latest = useRef({ repos, selected });
  latest.current = { repos, selected };
  const open = (index: number) => {
    const repo = latest.current.repos[index];
    if (repo && !repo.missing) openPath(repo.path);
  };

  // 화면들처럼 window에서 받는다(포커스가 목록 밖에 있어도 먹는다). 라이선스 창은 키를 먼저 먹는다.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || isComposing(e)) return;
      // 단추에 포커스가 있으면 Enter·Space는 그 단추의 것이다.
      if ((e.target as Element | null)?.closest?.("button, input, textarea")) return;
      const action = startKey(e);
      const { repos, selected } = latest.current;
      if (!action || repos.length === 0) return;
      e.preventDefault();
      if (action === "down") setSelected(Math.min(repos.length - 1, selected + 1));
      else if (action === "up") setSelected(Math.max(0, selected - 1));
      else if (action === "open") open(selected);
      else if (repos[selected]) forgetRecent(repos[selected].path);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // open은 latest만 읽는다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const install = async () => {
    setInstalling(true);
    const status = await installCli();
    setInstalling(false);
    if (status) setCli(status);
    root.current?.focus({ preventScroll: true });
  };

  return (
    <main className="start-screen" ref={root} tabIndex={-1}>
      <div className="start-panel">
        <p className="start-title">
          <span className="brand">parallax</span>
          {version && <span className="muted"> v{version}</span>}
        </p>
        <p className="muted">로컬 git 이력을 읽기 전용으로 본다.</p>

        <div className="start-actions">
          <button type="button" className="start-open" onClick={openRepoDialog}>
            저장소 열기… <kbd>{OPEN_REPO_KEYS}</kbd>
          </button>
          <span className="muted">폴더나 파일을 이 창에 끌어 놓아도 열린다.</span>
        </div>

        <section className="start-section" aria-label="최근 저장소">
          <h2>최근 저장소</h2>
          {loaded && repos.length === 0 ? (
            <p className="muted">아직 연 저장소가 없다.</p>
          ) : (
            <ul className="start-recent" role="listbox" aria-label="최근 저장소" ref={list}>
              {repos.map((repo, i) => (
                <li
                  key={repo.path}
                  role="option"
                  aria-selected={i === selected}
                  className={`start-recent-item${i === selected ? " selected" : ""}${repo.missing ? " missing" : ""}`}
                  title={repo.missing ? `${repo.path} (폴더가 없다)` : repo.path}
                  onMouseMove={() => i !== selected && setSelected(i)}
                  onClick={() => open(i)}
                >
                  <span className="start-recent-name">{repo.name}</span>
                  <span className="start-recent-path">{repo.display}</span>
                  {repo.missing && <span className="start-recent-missing">없음</span>}
                  <button
                    type="button"
                    className="start-forget"
                    aria-label={`${repo.name}을(를) 목록에서 빼기`}
                    title="목록에서 빼기 (Delete)"
                    onClick={(e) => {
                      e.stopPropagation();
                      forgetRecent(repo.path);
                    }}
                  >
                    ✕
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="start-section" aria-label="터미널에서 열기">
          <h2>터미널에서 열기</h2>
          <p className="muted">
            저장소 안에서 <code>plx</code>(그래프), <code>plx 파일:줄</code>(blame), <code>plx log 파일</code>
            (히스토리)을 실행하면 이 창에 열린다. 앱만 써도 된다.
          </p>
          {cli &&
            (cli.installed ? (
              <p>
                터미널 명령 설치됨: <code>{cli.location}</code>
              </p>
            ) : cli.blocked ? (
              <p className="warn-text">{cli.blocked}</p>
            ) : (
              <p className="start-install">
                <button type="button" onClick={install} disabled={installing}>
                  {installing ? "설치하는 중…" : "터미널 명령 설치"}
                </button>
                <span className="muted">{cli.plan}</span>
              </p>
            ))}
          {cli?.note && <p className="muted">{cli.note}</p>}
        </section>

        <p className="start-footer">
          <button type="button" className="link" onClick={openNotices}>
            오픈 소스 라이선스
          </button>
        </p>
      </div>
    </main>
  );
}

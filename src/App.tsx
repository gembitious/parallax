import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type AppInfo = { version: string; coreVersion: string };

export default function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    invoke<AppInfo>("app_info").then(setInfo, (e) => console.error("app_info 실패", e));
  }, []);

  return (
    <div className="app">
      <header className="titlebar">
        <span className="brand">parallax</span>
        {info && <span className="muted">v{info.version}</span>}
      </header>
      <main className="placeholder">
        <p>로컬 git 이력 뷰어</p>
        <p className="muted">터미널에서 plx를 실행하면 이 창에 대상 뷰가 열린다.</p>
      </main>
    </div>
  );
}

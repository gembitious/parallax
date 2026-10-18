// 시작 IPC를 먼저 보낸다: 맨 앞에 불러와야 아래 모듈을 실행하는 동안 답이 온다.
import { initialRequest } from "./boot";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";
import { bootMarks, nowMs } from "./perf";

// 여기까지 오면 불러온 모듈을 모두 실행했다(콜드 스타트 구간).
bootMarks.modulesMs = nowMs();

// StrictMode는 쓰지 않는다. 개발 빌드에서 effect가 두 번 돌면 성능 HUD의 요청 측정이 중복된다.
void initialRequest.then((request) => {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(<App initial={request} />);
});

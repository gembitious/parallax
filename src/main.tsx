import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

// StrictMode는 쓰지 않는다. 개발 빌드에서 effect가 두 번 돌면 성능 HUD의 요청 측정이 중복된다.
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(<App />);

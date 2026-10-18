// 화면이 아직 붙어 있는가. 비동기 응답(이동할 곳 찾기 등)이 닫힌 화면에서 탐색 스택을 바꾸지 않게 한다.
import { useEffect, useRef } from "react";

export function useMounted(): { readonly current: boolean } {
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  return mounted;
}

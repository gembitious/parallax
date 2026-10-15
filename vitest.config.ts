import { defineConfig } from "vitest/config";

// 프런트 단위 테스트(순수 로직). bench/repos의 참조 저장소에 든 테스트 파일까지 줍지 않게 src만 본다.
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
  },
});

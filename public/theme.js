// 창이 처음 그려지기 전에 테마와 코드 글자 크기를 놓는다(src/settings/settings.ts와 같은 칸을 읽는다).
// 모듈 스크립트보다 먼저 도는 보통 스크립트라, 시스템과 다른 테마를 고른 사람도 첫 화면이 깜빡이지 않는다.
(function () {
  var theme = "system";
  var size = 13;
  try {
    var saved = JSON.parse(localStorage.getItem("plx.settings") || "null");
    if (saved && (saved.theme === "light" || saved.theme === "dark")) theme = saved.theme;
    if (saved && saved.codeFontSize >= 10 && saved.codeFontSize <= 20 && saved.codeFontSize % 1 === 0) {
      size = saved.codeFontSize;
    }
  } catch (e) {
    // 기본값
  }
  var dark = theme === "dark" || (theme === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  var root = document.documentElement;
  root.dataset.theme = dark ? "dark" : "light";
  root.style.setProperty("--code-font-size", size + "px");
  root.style.setProperty("--code-line-height", Math.round((size * 20) / 13) + "px");
})();

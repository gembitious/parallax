import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";
import { enclosingSymbol } from "./symbol";

async function stateFor(file: string, doc: string): Promise<EditorState> {
  const description = LanguageDescription.matchFilename(languages, file);
  if (!description) throw new Error(file);
  const support = await description.load();
  return EditorState.create({ doc, extensions: [EditorState.lineSeparator.of("\n"), support] });
}

/** `line`(1부터) 줄 끝의 커서로 찾는다 */
function at(state: EditorState, line: number) {
  return enclosingSymbol(state, state.doc.line(line).to, 1000);
}

describe("enclosingSymbol", () => {
  it("TypeScript: 함수, 메서드, 화살표 함수, 객체 메서드, 가장 안쪽", async () => {
    const s = await stateFor(
      "a.ts",
      [
        "export function a(x: number) {", // 1
        "  return x;", // 2
        "}", // 3
        "class B {", // 4
        "  m() {", // 5
        "    const f = () => {", // 6
        "      return 2;", // 7
        "    };", // 8
        "  }", // 9
        "}", // 10
        "const o = {", // 11
        "  p() {", // 12
        "    return 1;", // 13
        "  },", // 14
        "};", // 15
        "", // 16
      ].join("\n"),
    );
    expect(at(s, 2)).toEqual({ kind: "function", name: "a", start: 1, end: 3 });
    expect(at(s, 1)).toMatchObject({ name: "a", start: 1, end: 3 });
    expect(at(s, 7)).toEqual({ kind: "function", name: "f", start: 6, end: 8 });
    expect(at(s, 9)).toEqual({ kind: "method", name: "m", start: 5, end: 9 });
    expect(at(s, 10)).toEqual({ kind: "class", name: "B", start: 4, end: 10 });
    expect(at(s, 13)).toEqual({ kind: "method", name: "p", start: 12, end: 14 });
    // 함수가 아닌 객체·빈 줄은 감싼 정의가 없다
    expect(at(s, 15)).toBeNull();
    expect(at(s, 16)).toBeNull();
  });

  it("Python: 데코레이터를 넣고 끝의 빈 줄은 뺀다", async () => {
    const s = await stateFor("a.py", ["@dec", "def f(x):", "    return 1", "", "", "class C:", "    def m(self):", "        pass", ""].join("\n"));
    expect(at(s, 3)).toEqual({ kind: "function", name: "f", start: 1, end: 3 });
    expect(at(s, 8)).toEqual({ kind: "function", name: "m", start: 7, end: 8 });
    expect(at(s, 6)).toEqual({ kind: "class", name: "C", start: 6, end: 8 });
  });

  it("Rust: impl 안의 fn, impl 머리", async () => {
    const s = await stateFor("a.rs", ["impl Trait for S {", "    pub fn m(&self) {", "        1;", "    }", "}"].join("\n"));
    expect(at(s, 3)).toEqual({ kind: "function", name: "m", start: 2, end: 4 });
    expect(at(s, 5)).toEqual({ kind: "class", name: "impl Trait for S", start: 1, end: 5 });
  });

  it("C++·Go·Java: 선언자 안의 이름", async () => {
    const cpp = await stateFor("a.cpp", ["void S::m(int x) {", "  return;", "}"].join("\n"));
    expect(at(cpp, 2)).toEqual({ kind: "function", name: "S::m", start: 1, end: 3 });
    const go = await stateFor("a.go", ["package p", "func (s *S) M() {", "\treturn", "}"].join("\n"));
    expect(at(go, 3)).toEqual({ kind: "method", name: "M", start: 2, end: 4 });
    const java = await stateFor("A.java", ["class A {", "  void m() {", "    return;", "  }", "}"].join("\n"));
    expect(at(java, 3)).toEqual({ kind: "method", name: "m", start: 2, end: 4 });
  });

  it("구문 트리가 없는 파일은 null", () => {
    const s = EditorState.create({ doc: "function a() {\n}\n" });
    expect(enclosingSymbol(s, 3)).toBeNull();
  });
});

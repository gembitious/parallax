// 함수 히스토리(M16): 커서가 든 함수·메서드·클래스의 줄 범위를 구문 트리(Lezer)에서 찾는다.
// 범위는 그 리비전(작업트리 blame은 작업트리) 버전의 줄이고, 라인 히스토리(`git log -L <a>,<b>:<file>`)에 그대로 준다.
// git의 `-L :<funcname>:<file>`을 쓰지 않는 이유: git은 diff 드라이버(.gitattributes의 `diff=<언어>`)의 funcname 정규식으로
// 함수의 시작 줄만 찾고 끝은 "다음 funcname 줄 앞"이라, 드라이버가 없는 저장소(대부분)에서는 C 형식 줄만 함수로 보고
// 중첩·메서드·화살표 함수의 끝을 틀린다. 시작 커밋에서 범위를 정한 뒤로는 두 방법이 같다(git이 줄 범위를 옮기며 따라간다).
import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import type { EditorState } from "@codemirror/state";
import type { SyntaxNode } from "@lezer/common";

export type SymbolKind = "function" | "method" | "class" | "type" | "module";

export type EnclosingSymbol = {
  kind: SymbolKind;
  /** 보일 이름(`foo`, `S::m`, `impl Trait for S`) */
  name: string;
  /** 1부터, 양쪽 포함 */
  start: number;
  end: number;
};

/** 정의 노드 이름 → 종류. 언어마다 Lezer 문법의 노드 이름이다(javascript·typescript, python, rust, java, cpp, go, php). */
const DEFINITIONS: Record<string, SymbolKind> = {
  FunctionDeclaration: "function",
  FunctionDefinition: "function",
  FunctionItem: "function",
  FunctionDecl: "function",
  MethodDeclaration: "method",
  MethodDecl: "method",
  ConstructorDeclaration: "method",
  ClassDeclaration: "class",
  ClassDefinition: "class",
  ClassSpecifier: "class",
  StructSpecifier: "class",
  StructItem: "type",
  EnumItem: "type",
  EnumDeclaration: "type",
  InterfaceDeclaration: "type",
  TypeAliasDeclaration: "type",
  TraitItem: "type",
  TypeDecl: "type",
  ImplItem: "class",
  ModItem: "module",
  NamespaceDefinition: "module",
  ModuleDeclaration: "module",
};

/** 함수 값을 담은 선언·속성(`const f = () => …`, `{ m() {} }`, `{ q: function () {} }`)의 값 노드 */
const FUNCTION_VALUES = new Set(["ArrowFunction", "FunctionExpression", "ClassExpression"]);

/** 정의 노드의 바로 아래에서 이름으로 보는 노드 */
const NAME_NODES = new Set([
  "VariableDefinition",
  "PropertyDefinition",
  "TypeDefinition",
  "VariableName",
  "BoundIdentifier",
  "TypeIdentifier",
  "Definition",
  "DefName",
  "FieldName",
  "Name",
  "Identifier",
  "FieldIdentifier",
  "ScopedIdentifier",
  "PropertyName",
]);

/** 정의면 그 종류, 아니면 null */
function definitionKind(node: SyntaxNode): SymbolKind | null {
  const kind = DEFINITIONS[node.name];
  if (kind) return kind;
  // JS·TS: 값이 함수인 변수 선언과 객체 속성(메서드 꼴 `m() {}`은 Block을 바로 갖는다)
  if (node.name === "VariableDeclaration" || node.name === "Property") {
    for (let child = node.firstChild; child; child = child.nextSibling) {
      if (FUNCTION_VALUES.has(child.name)) return child.name === "ClassExpression" ? "class" : "function";
      if (node.name === "Property" && child.name === "Block") return "method";
    }
  }
  return null;
}

/** 정의의 이름. C·C++은 선언자(`FunctionDeclarator`, `PointerDeclarator` …) 안을, Go의 type은 TypeSpec 안을 본다. */
function definitionName(state: EditorState, node: SyntaxNode): string | null {
  if (node.name === "ImplItem") {
    // `impl Trait for S` 전체가 이름이다(본문 앞까지)
    const body = node.getChild("DeclarationList");
    return oneLine(state.sliceDoc(node.from, body ? body.from : node.to));
  }
  const find = (parent: SyntaxNode, depth: number): string | null => {
    for (let child = parent.firstChild; child; child = child.nextSibling) {
      if (NAME_NODES.has(child.name)) return oneLine(state.sliceDoc(child.from, child.to));
      if (depth < 3 && (child.name.endsWith("Declarator") || child.name === "TypeSpec")) {
        const inner = find(child, depth + 1);
        if (inner) return inner;
      }
      // 본문에 들어가기 전에 멈춘다(본문 안의 이름을 집지 않는다)
      if (/Body$|^Block$|^CompoundStatement$|DeclarationList$|^ParamList$/.test(child.name)) break;
    }
    return null;
  };
  return find(node, 0);
}

function oneLine(text: string): string {
  const line = text.replace(/\s+/g, " ").trim();
  return line.length > 80 ? `${line.slice(0, 79)}…` : line;
}

/**
 * `pos` 줄을 감싼 가장 안쪽의 함수·메서드·클래스. 구문 트리가 없거나(언어 모듈이 없는 파일, 아직 불러오는 중) 감싼 정의가
 * 없으면 null. 커서가 줄 앞 공백에 있어도 그 줄의 코드로 본다.
 */
export function enclosingSymbol(state: EditorState, pos: number, timeout = 200): EnclosingSymbol | null {
  const doc = state.doc;
  const line = doc.lineAt(pos);
  const indent = line.text.length - line.text.trimStart().length;
  const at = line.text.trim() ? line.from + indent : pos;
  // 파일 끝까지 분석해야 정의의 끝을 안다. 시간 안에 못 끝내면 지금까지의 트리(끝이 잘릴 수 있다)를 쓴다.
  const tree = ensureSyntaxTree(state, doc.length, timeout) ?? syntaxTree(state);
  if (tree.length === 0) return null;
  // 줄 앞(`export function`의 `export`처럼 정의 밖일 수 있다)과 줄 끝 두 곳에서 찾아 더 안쪽(좁은) 것을 고른다.
  const trimmedEnd = line.from + line.text.trimEnd().length;
  let best: EnclosingSymbol | null = null;
  for (const [from, side] of [
    [at, 1],
    [trimmedEnd, -1],
  ] as const) {
    const found = enclosingAt(state, tree.resolveInner(from, side), line.number);
    if (found && (!best || found.end - found.start < best.end - best.start)) best = found;
  }
  return best;
}

function enclosingAt(state: EditorState, inner: SyntaxNode, lineNumber: number): EnclosingSymbol | null {
  const doc = state.doc;
  for (let node: SyntaxNode | null = inner; node; node = node.parent) {
    const kind = definitionKind(node);
    if (!kind) continue;
    // 파이썬 데코레이터는 정의에 든다(DecoratedStatement가 감싼다)
    const outer = node.parent?.name === "DecoratedStatement" ? node.parent : node;
    // 끝의 공백·빈 줄(파이썬 Body는 다음 줄 맨 앞까지다)은 뺀다
    let to = outer.to;
    while (to > outer.from && /\s/.test(state.sliceDoc(to - 1, to))) to--;
    const start = doc.lineAt(outer.from).number;
    const end = doc.lineAt(Math.max(outer.from, to)).number;
    if (end < lineNumber || start > lineNumber) continue;
    return { kind, name: definitionName(state, node) ?? oneLine(doc.lineAt(node.from).text), start, end };
  }
  return null;
}

export const SYMBOL_KIND_LABEL: Record<SymbolKind, string> = {
  function: "함수",
  method: "메서드",
  class: "클래스",
  type: "타입",
  module: "모듈",
};

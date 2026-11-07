//! 작업트리 blame의 바뀐 줄 표시(M13): HEAD 버전과 작업트리 내용의 줄 diff를 작업트리 줄 번호의 표시로 바꾼다.
//!
//! VS Code의 줄 변경 표시와 같다: 새로 생긴 줄(HEAD 쪽이 비었다)은 추가, 바뀐 줄(양쪽이 있다)은 수정, 지운 줄(작업트리
//! 쪽이 비었다)은 그 자리 앞 줄의 아래 끝에 표시한다. diff는 `,`(부모 리비전 blame)의 줄 옮기기와 같은 것이다
//! ([`LineMap`], git과 같은 hunk: 골든 테스트가 `git diff -U0`과 견준다).

use serde::Serialize;

use crate::linemap::LineMap;

/// 두 쪽 합이 이보다 크면 표시하지 않는다(커밋 상세가 diff를 보이지 않는 크기와 같다). 첫 화면을 늦추지 않게.
pub const MAX_BYTES: usize = 4 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LineChangeKind {
    Added,
    Modified,
    Deleted,
}

/// 바뀐 줄 묶음. 줄은 작업트리 내용의 줄(1부터)이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineChange {
    pub kind: LineChangeKind,
    /// 추가·수정: 첫 줄. 지움: 지운 자리 바로 앞 줄(맨 앞을 지웠으면 0)
    pub start: u32,
    /// 추가·수정: 마지막 줄(포함). 지움: `start`와 같다.
    pub end: u32,
    /// 지운 줄 수(수정이면 HEAD 쪽 줄 수, 추가면 0)
    pub removed: u32,
}

/// `before`(HEAD 버전, 없으면 새 파일)와 `after`(작업트리, clean 모양)의 바뀐 줄. 너무 크면 `None`.
pub fn line_changes(before: Option<&[u8]>, after: &[u8]) -> Option<Vec<LineChange>> {
    if before.map_or(0, <[u8]>::len) + after.len() > MAX_BYTES {
        return None;
    }
    let map = LineMap::new(before.unwrap_or_default(), after);
    Some(
        map.hunks()
            .iter()
            .map(|(old, new)| {
                let removed = old.end - old.start;
                if new.is_empty() {
                    LineChange {
                        kind: LineChangeKind::Deleted,
                        start: new.start,
                        end: new.start,
                        removed,
                    }
                } else {
                    LineChange {
                        kind: if removed == 0 {
                            LineChangeKind::Added
                        } else {
                            LineChangeKind::Modified
                        },
                        start: new.start + 1,
                        end: new.end,
                        removed,
                    }
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use LineChangeKind::*;

    fn text(lines: &[&str]) -> Vec<u8> {
        lines
            .iter()
            .flat_map(|l| format!("{l}\n").into_bytes())
            .collect()
    }

    fn change(kind: LineChangeKind, start: u32, end: u32, removed: u32) -> LineChange {
        LineChange {
            kind,
            start,
            end,
            removed,
        }
    }

    #[test]
    fn kinds() {
        let before = text(&["a", "b", "c", "d", "e"]);
        let after = text(&["x", "a", "B", "c", "e", "f", "g"]);
        assert_eq!(
            line_changes(Some(&before), &after).unwrap(),
            vec![
                change(Added, 1, 1, 0),
                change(Modified, 3, 3, 1),
                change(Deleted, 4, 4, 1),
                change(Added, 6, 7, 0),
            ]
        );
    }

    #[test]
    fn deleted_at_top_and_new_file() {
        let before = text(&["a", "b", "c"]);
        let after = text(&["c"]);
        assert_eq!(
            line_changes(Some(&before), &after).unwrap(),
            vec![change(Deleted, 0, 0, 2)]
        );
        assert_eq!(
            line_changes(None, &after).unwrap(),
            vec![change(Added, 1, 1, 0)]
        );
        assert_eq!(line_changes(None, b"").unwrap(), vec![]);
        assert_eq!(line_changes(Some(&before), &before).unwrap(), vec![]);
    }

    #[test]
    fn too_large() {
        let big = vec![b'a'; MAX_BYTES];
        assert!(line_changes(Some(b"x"), &big).is_none());
    }
}

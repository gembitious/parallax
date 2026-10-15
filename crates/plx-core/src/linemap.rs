//! 두 버전 사이의 줄 위치 매핑.
//!
//! `,`(부모 리비전 blame)로 이동할 때 커서 줄을 부모 버전의 줄로 옮긴다. 줄 diff(Myers와 git과 같은 들여쓰기
//! 휴리스틱)를 계산해, 바뀌지 않은 줄은 대응하는 줄로, 그 커밋에서 생기거나 바뀐 줄은 hunk가 시작하는 줄로 보낸다.

use gix::diff::blob::{Algorithm, Diff, InternedInput};

/// 줄 diff 결과. 여러 줄을 옮길 때 다시 계산하지 않도록 hunk만 들고 있다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMap {
    /// (before 범위, after 범위), 0부터 세는 반열린 구간
    hunks: Vec<(std::ops::Range<u32>, std::ops::Range<u32>)>,
    before_lines: u32,
}

impl LineMap {
    pub fn new(before: &[u8], after: &[u8]) -> LineMap {
        let input = InternedInput::new(before, after);
        let mut diff = Diff::compute(Algorithm::Myers, &input);
        diff.postprocess_lines(&input);
        LineMap {
            hunks: diff.hunks().map(|h| (h.before, h.after)).collect(),
            before_lines: input.before.len() as u32,
        }
    }

    /// git diff 출력의 hunk로 만든다(골든 테스트에서 git 결과와 비교할 때 쓴다).
    pub fn from_hunks(
        hunks: Vec<(std::ops::Range<u32>, std::ops::Range<u32>)>,
        before_lines: u32,
    ) -> LineMap {
        LineMap {
            hunks,
            before_lines,
        }
    }

    /// `after`의 `line`(1부터)에 해당하는 `before`의 줄(1부터).
    /// `after`에서 생기거나 바뀐 줄이면 그 hunk가 시작하는 `before`의 줄이다.
    pub fn to_before(&self, line: u32) -> u32 {
        let target = line.saturating_sub(1);
        let mut offset: i64 = 0;
        for (before, after) in &self.hunks {
            if target < after.start {
                break;
            }
            if target < after.end {
                return self.clamp(i64::from(before.start) + 1);
            }
            offset += i64::from(before.end - before.start) - i64::from(after.end - after.start);
        }
        self.clamp(i64::from(target) + offset + 1)
    }

    fn clamp(&self, line: i64) -> u32 {
        line.clamp(1, i64::from(self.before_lines.max(1))) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[&str]) -> Vec<u8> {
        lines
            .iter()
            .flat_map(|l| format!("{l}\n").into_bytes())
            .collect()
    }

    #[test]
    fn unchanged_lines_follow_offsets() {
        let before = text(&["a", "b", "c", "d"]);
        let after = text(&["a", "x", "y", "b", "c", "d"]);
        let map = LineMap::new(&before, &after);
        assert_eq!(map.to_before(1), 1); // a
        assert_eq!(map.to_before(4), 2); // b
        assert_eq!(map.to_before(6), 4); // d
    }

    #[test]
    fn added_lines_go_to_hunk_start() {
        let before = text(&["a", "b", "c"]);
        let after = text(&["a", "x", "y", "b", "c"]);
        let map = LineMap::new(&before, &after);
        // x, y는 a 다음(b 자리)에 생겼다 → before의 2번째 줄(b)
        assert_eq!(map.to_before(2), 2);
        assert_eq!(map.to_before(3), 2);
    }

    #[test]
    fn changed_lines_go_to_hunk_start() {
        let before = text(&["a", "old1", "old2", "b"]);
        let after = text(&["a", "new", "b"]);
        let map = LineMap::new(&before, &after);
        assert_eq!(map.to_before(2), 2);
        assert_eq!(map.to_before(3), 4);
    }

    #[test]
    fn lines_added_at_end_clamp_to_last_line() {
        let before = text(&["a", "b"]);
        let after = text(&["a", "b", "c", "d"]);
        let map = LineMap::new(&before, &after);
        assert_eq!(map.to_before(4), 2);
    }

    #[test]
    fn empty_before() {
        let map = LineMap::new(b"", &text(&["a", "b"]));
        assert_eq!(map.to_before(2), 1);
    }
}

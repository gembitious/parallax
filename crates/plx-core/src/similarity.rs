//! git의 rename 유사도(diffcore-delta.c의 spanhash, diffcore-rename.c의 estimate_similarity)를 그대로 옮겼다.
//! 파일 하나의 rename 원본을 찾을 때([`crate::repo::Repo::rename_source`])와 커밋의 rename을 찾을 때([`crate::renames`]) 쓴다.
//!
//! 내용을 줄 끝 또는 64바이트마다 끊어 조각의 해시별 바이트 수를 세고, 두 파일이 함께 가진 바이트 수를
//! 큰 파일의 크기로 나눈다. 텍스트면 CRLF의 CR은 세지 않는다. `git diff -M`이 보이는 `R095`의 95가
//! `score * 100 / MAX_SCORE`다.

use std::collections::HashMap;

/// git의 MAX_SCORE
pub const MAX_SCORE: u64 = 60000;
/// git 기본 rename 기준(`-M`, 50%)
pub const MINIMUM_SCORE: u64 = MAX_SCORE / 2;

const HASHBASE: u32 = 107_927;

/// 파일 하나의 조각 해시별 바이트 수(해시 순)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spans {
    counts: Vec<(u32, u64)>,
    size: u64,
}

impl Spans {
    /// `text`는 바이너리가 아닌가(git은 앞 8000바이트에 NUL이 있으면 바이너리로 본다).
    pub fn new(data: &[u8], text: bool) -> Spans {
        let mut hashes: Vec<(u32, u64)> = Vec::new();
        let (mut accum1, mut accum2, mut n) = (0u32, 0u32, 0u64);
        let mut i = 0;
        while i < data.len() {
            let c = data[i];
            i += 1;
            if text && c == b'\r' && data.get(i) == Some(&b'\n') {
                continue;
            }
            let old1 = accum1;
            accum1 = (accum1 << 7) ^ (accum2 >> 25);
            accum2 = (accum2 << 7) ^ (old1 >> 25);
            accum1 = accum1.wrapping_add(u32::from(c));
            n += 1;
            if n < 64 && c != b'\n' {
                continue;
            }
            hashes.push((span_hash(accum1, accum2), n));
            (accum1, accum2, n) = (0, 0, 0);
        }
        if n > 0 {
            hashes.push((span_hash(accum1, accum2), n));
        }
        hashes.sort_unstable_by_key(|&(hash, _)| hash);
        let mut counts: Vec<(u32, u64)> = Vec::with_capacity(hashes.len());
        for (hash, bytes) in hashes {
            match counts.last_mut() {
                Some((last, total)) if *last == hash => *total += bytes,
                _ => counts.push((hash, bytes)),
            }
        }
        Spans {
            counts,
            size: data.len() as u64,
        }
    }
}

impl Spans {
    /// 파일 크기(바이트)
    pub fn size(&self) -> u64 {
        self.size
    }
}

fn span_hash(accum1: u32, accum2: u32) -> u32 {
    accum1.wrapping_add(accum2.wrapping_mul(0x61)) % HASHBASE
}

/// 크기만 보고 기준을 넘을 수 없는 짝을 거른다. 크기 차이가 작은 쪽의 크기를 넘으면(50% 기준) 안 된다.
pub fn sizes_may_match(src: u64, dst: u64) -> bool {
    let (max, min) = (src.max(dst), src.min(dst));
    max * (MAX_SCORE - MINIMUM_SCORE) >= (max - min) * MAX_SCORE
}

/// `src`에서 `dst`로 옮겨 온 정도(0..=[`MAX_SCORE`]). git과 같이 크기 차이가 크면 계산하지 않고 0이다.
pub fn score(src: &Spans, dst: &Spans) -> u64 {
    if !sizes_may_match(src.size, dst.size) || dst.size == 0 {
        return 0;
    }
    let max = src.size.max(dst.size);
    // 해시 순으로 정렬한 두 목록을 함께 걷는다(같은 해시는 한 번씩만 있다).
    let (a, b) = (&src.counts[..], &dst.counts[..]);
    let (mut i, mut j, mut copied) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        let ((hash_a, bytes_a), (hash_b, bytes_b)) = (a[i], b[j]);
        if hash_a < hash_b {
            i += 1;
        } else if hash_b < hash_a {
            j += 1;
        } else {
            copied += bytes_a.min(bytes_b);
            i += 1;
            j += 1;
        }
    }
    copied * MAX_SCORE / max
}

/// 여러 원본의 조각을 해시로 모은 색인. 새 파일 하나를 원본 모두와 견줄 때 함께 가진 조각만 걷는다
/// (짝마다 두 목록을 끝까지 걷는 [`score`]와 값이 같다. 서로 다른 파일끼리는 함께 가진 조각이 드물다).
pub struct Index {
    postings: HashMap<u32, Vec<(u32, u64)>>,
    sources: usize,
}

impl Index {
    /// 원본 자리마다 조각(없으면 그 자리는 늘 0이다)
    pub fn new(sources: &[Option<&Spans>]) -> Index {
        let mut postings: HashMap<u32, Vec<(u32, u64)>> = HashMap::new();
        for (i, spans) in sources.iter().enumerate() {
            let Some(spans) = spans else { continue };
            let slot = u32::try_from(i).expect("원본 수는 u32 안이다");
            for &(hash, bytes) in &spans.counts {
                postings.entry(hash).or_default().push((slot, bytes));
            }
        }
        Index {
            postings,
            sources: sources.len(),
        }
    }

    /// 원본 자리마다 `dst`와 함께 가진 바이트 수(git의 src_copied)를 `copied`에 담는다.
    pub fn copied(&self, dst: &Spans, copied: &mut Vec<u64>) {
        copied.clear();
        copied.resize(self.sources, 0);
        for &(hash, dst_bytes) in &dst.counts {
            for &(slot, src_bytes) in self.postings.get(&hash).into_iter().flatten() {
                copied[slot as usize] += src_bytes.min(dst_bytes);
            }
        }
    }
}

/// [`Index::copied`]로 센 바이트 수의 유사도. 같은 두 파일의 [`score`]와 같다.
pub fn score_of(copied: u64, src_size: u64, dst_size: u64) -> u64 {
    if !sizes_may_match(src_size, dst_size) || dst_size == 0 {
        return 0;
    }
    copied * MAX_SCORE / src_size.max(dst_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(prefix: &str, n: usize) -> String {
        (0..n)
            .map(|i| format!("{prefix} line {i}: the quick brown fox jumps over the lazy dog\n"))
            .collect()
    }

    #[test]
    fn identical_and_unrelated() {
        let a = lines("a", 20);
        let spans = Spans::new(a.as_bytes(), true);
        assert_eq!(score(&spans, &spans), MAX_SCORE);
        let b = lines("b", 20);
        assert!(score(&spans, &Spans::new(b.as_bytes(), true)) < MAX_SCORE / 10);
    }

    #[test]
    fn crlf_is_ignored_for_text_only() {
        let lf = lines("a", 10);
        let crlf = lf.replace('\n', "\r\n");
        let (lf, crlf) = (lf.as_bytes(), crlf.as_bytes());
        // 같은 조각이지만 크기는 CR만큼 다르다: copied = LF 파일 크기, max = CRLF 파일 크기
        let expected = lf.len() as u64 * MAX_SCORE / crlf.len() as u64;
        assert_eq!(
            score(&Spans::new(lf, true), &Spans::new(crlf, true)),
            expected
        );
        // 바이너리면 CR도 조각에 들어가 줄이 하나도 같지 않다.
        assert!(score(&Spans::new(lf, false), &Spans::new(crlf, false)) < expected / 2);
    }

    #[test]
    fn long_lines_split_every_64_bytes() {
        let long = "x".repeat(64 * 3);
        let spans = Spans::new(long.as_bytes(), true);
        assert_eq!(spans.counts.len(), 1, "같은 64바이트 조각 셋");
        assert_eq!(spans.counts[0].1, 192);
    }

    #[test]
    fn index_scores_like_pairs() {
        let files = [
            lines("a", 20),
            lines("a", 15) + &lines("b", 5),
            lines("c", 20),
            lines("a", 20).replace('\n', "\r\n"),
            String::new(),
        ];
        let spans: Vec<Spans> = files
            .iter()
            .map(|f| Spans::new(f.as_bytes(), true))
            .collect();
        let slots: Vec<Option<&Spans>> = spans.iter().map(Some).chain([None]).collect();
        let index = Index::new(&slots);
        let mut copied = Vec::new();
        for dst in &spans {
            index.copied(dst, &mut copied);
            assert_eq!(copied.len(), slots.len());
            for (i, src) in spans.iter().enumerate() {
                assert_eq!(
                    score_of(copied[i], src.size(), dst.size()),
                    score(src, dst),
                    "{i}"
                );
            }
            assert_eq!(copied[spans.len()], 0, "조각이 없는 자리");
        }
    }

    #[test]
    fn size_filter() {
        assert!(sizes_may_match(100, 199));
        assert!(sizes_may_match(100, 200));
        assert!(!sizes_may_match(100, 201));
        assert!(!sizes_may_match(0, 10));
        let small = Spans::new(b"a\n", true);
        let big = Spans::new(lines("a", 10).as_bytes(), true);
        assert_eq!(score(&small, &big), 0);
    }
}

//! git의 rename 유사도(diffcore-delta.c의 spanhash, diffcore-rename.c의 estimate_similarity)를 그대로 옮겼다.
//! 파일 하나의 rename 원본을 찾을 때 쓴다([`crate::repo::Repo::rename_source`]).
//!
//! 내용을 줄 끝 또는 64바이트마다 끊어 조각의 해시별 바이트 수를 세고, 두 파일이 함께 가진 바이트 수를
//! 큰 파일의 크기로 나눈다. 텍스트면 CRLF의 CR은 세지 않는다. `git diff -M`이 보이는 `R095`의 95가
//! `score * 100 / MAX_SCORE`다.

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
    let mut copied = 0;
    let mut d = dst.counts.iter().peekable();
    for &(hash, src_bytes) in &src.counts {
        while d.next_if(|&&(h, _)| h < hash).is_some() {}
        if let Some(&(_, dst_bytes)) = d.next_if(|&&(h, _)| h == hash) {
            copied += src_bytes.min(dst_bytes);
        }
    }
    copied * MAX_SCORE / max
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

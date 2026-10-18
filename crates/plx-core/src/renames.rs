//! 커밋 하나의 rename 찾기. git의 diffcore-rename(`-M`, 복사는 찾지 않는다)과 같은 단계·기준이다.
//!
//! gix의 rename 추적은 짝 수(지운 항목 × 새 항목, 디렉터리 항목까지)를 한도 1,000과 견줘(git은 1,000²) 파일을 조금만
//! 옮긴 커밋에서도 유사도 비교를 건너뛰고(새 파일이 수백 개면 rename이 추가 + 삭제로 보였다), 짝마다 줄 diff를 해 느리다.
//! 그래서 git과 같은 기준의 유사도([`crate::similarity`])로 git의 단계를 그대로 밟는다:
//! 1. 내용이 같은 파일(blob id. 일반 파일이 아니면 종류도 같아야 한다). 파일 이름이 같은 원본이 먼저다.
//! 2. 남은 파일 가운데 이름이 지운 쪽과 새 쪽에 하나씩뿐이면 그 둘을 견줘 75% 이상이면 짝짓는다(find_basename_matches).
//! 3. 남은 짝 수가 한도²(기본 1,000²) 이하면 모두 견줘 새 파일마다 가장 비슷한 넷을 두고, 모두를 비슷한 순(같으면 파일
//!    이름이 같은 것 먼저)으로 짝짓는다(50% 이상, 원본은 한 번만). 넘으면 git처럼 이 단계를 건너뛴다.
//!    내용은 크기로 보아 짝이 될 수 있는 blob만 읽고, 원본의 조각 색인으로 새 파일마다 함께 가진 조각만 센다
//!    (짝마다 두 파일을 끝까지 걷지 않는다. 값은 짝마다 견준 것과 같다).
//!
//! blob이 없으면(부분 클론) 그 짝은 견주지 못한 것으로 치고 `incomplete`를 켠다(git은 그때 받아 온다).

use std::collections::HashMap;

use gix::ObjectId;
use gix::object::tree::EntryKind;

use crate::similarity::{self, MAX_SCORE, MINIMUM_SCORE, Spans};

/// git의 `diff.renameLimit` 기본값. 남은 짝이 이 값의 제곱을 넘으면 유사도로 찾지 않는다.
pub const RENAME_LIMIT: usize = 1000;

/// 지운 항목(원본 후보)이나 새 항목(대상 후보) 하나
pub(crate) struct Candidate<'a> {
    pub path: &'a [u8],
    pub id: ObjectId,
    pub kind: EntryKind,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Found {
    /// (원본 번호, 대상 번호, 유사도 0..=[`MAX_SCORE`])
    pub pairs: Vec<(usize, usize, u64)>,
    /// 내용을 읽지 못한 blob이 있어 견주지 못한 짝이 있다(부분 클론).
    pub incomplete: bool,
    /// 남은 짝이 한도²를 넘어 유사도로 찾지 않았다.
    pub limited: bool,
}

fn is_regular(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::Blob | EntryKind::BlobExecutable)
}

fn basename(path: &[u8]) -> &[u8] {
    path.rsplit(|&b| b == b'/').next().unwrap_or(path)
}

/// blob 크기와 조각 해시를 한 번씩만 읽는다.
struct Blobs<'r> {
    repo: &'r gix::Repository,
    sizes: HashMap<ObjectId, Option<u64>>,
    spans: HashMap<ObjectId, Option<Spans>>,
    incomplete: bool,
}

impl Blobs<'_> {
    fn size(&mut self, id: ObjectId) -> Option<u64> {
        let repo = self.repo;
        let size = *self
            .sizes
            .entry(id)
            .or_insert_with(|| repo.find_header(id).ok().map(|header| header.size()));
        if size.is_none() {
            self.incomplete = true;
        }
        size
    }

    fn spans(&mut self, id: ObjectId) -> bool {
        let repo = self.repo;
        let spans = self.spans.entry(id).or_insert_with(|| {
            let data = repo.find_blob(id).ok()?.detach().data;
            Some(Spans::new(&data, !crate::text::is_binary(&data)))
        });
        if spans.is_none() {
            self.incomplete = true;
        }
        spans.is_some()
    }

    /// git의 estimate_similarity: 일반 파일끼리만, 크기 차이가 크면 읽지 않고 0.
    fn similarity(&mut self, src: &Candidate, dst: &Candidate) -> u64 {
        if !is_regular(src.kind) || !is_regular(dst.kind) {
            return 0;
        }
        let (Some(src_size), Some(dst_size)) = (self.size(src.id), self.size(dst.id)) else {
            return 0;
        };
        if dst_size == 0 || !similarity::sizes_may_match(src_size, dst_size) {
            return 0;
        }
        if !self.spans(src.id) || !self.spans(dst.id) {
            return 0;
        }
        match (&self.spans[&src.id], &self.spans[&dst.id]) {
            (Some(src), Some(dst)) => similarity::score(src, dst),
            _ => 0,
        }
    }

    /// 3단계의 표: 남은 새 파일마다(순서대로) 가장 비슷한 원본 넷(git의 diff_score 표, 넷이 안 되면 있는 만큼).
    /// 크기는 맞은편에 일반 파일이 있을 때만 읽고, 내용은 크기로 보아 짝이 될 수 있는 blob만 읽는다(git이 견줄 때
    /// 읽는 것과 같다).
    fn matrix(
        &mut self,
        sources: &[Candidate],
        dests: &[Candidate],
        srcs: &[usize],
        dsts: &[usize],
    ) -> Vec<Scored> {
        let any_src = srcs.iter().any(|&s| is_regular(sources[s].kind));
        let any_dst = dsts.iter().any(|&d| is_regular(dests[d].kind));
        let mut size_of = |c: &Candidate, other_side: bool| {
            if other_side && is_regular(c.kind) {
                self.size(c.id)
            } else {
                None
            }
        };
        let src_size: Vec<Option<u64>> = srcs
            .iter()
            .map(|&s| size_of(&sources[s], any_dst))
            .collect();
        let dst_size: Vec<Option<u64>> =
            dsts.iter().map(|&d| size_of(&dests[d], any_src)).collect();
        let may_pair = |src: Option<u64>, dst: Option<u64>| match (src, dst) {
            (Some(src), Some(dst)) => dst > 0 && similarity::sizes_may_match(src, dst),
            _ => false,
        };
        let mut src_needed = vec![false; srcs.len()];
        let mut dst_needed = vec![false; dsts.len()];
        for (j, &dst) in dst_size.iter().enumerate() {
            for (i, &src) in src_size.iter().enumerate() {
                if may_pair(src, dst) {
                    src_needed[i] = true;
                    dst_needed[j] = true;
                }
            }
        }
        for (i, &s) in srcs.iter().enumerate() {
            if src_needed[i] {
                self.spans(sources[s].id);
            }
        }
        for (j, &d) in dsts.iter().enumerate() {
            if dst_needed[j] {
                self.spans(dests[d].id);
            }
        }

        let spans = &self.spans;
        let loaded = |c: &Candidate| spans.get(&c.id).and_then(Option::as_ref);
        let src_spans: Vec<Option<&Spans>> = srcs
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                if src_needed[i] {
                    loaded(&sources[s])
                } else {
                    None
                }
            })
            .collect();
        let src_names: Vec<&[u8]> = srcs.iter().map(|&s| basename(sources[s].path)).collect();
        let index = similarity::Index::new(&src_spans);
        let mut copied = Vec::new();
        let mut all = Vec::with_capacity(dsts.len() * 4);
        for (j, &d) in dsts.iter().enumerate() {
            let dst_spans = if dst_needed[j] {
                loaded(&dests[d])
            } else {
                None
            };
            match dst_spans {
                Some(dst) => index.copied(dst, &mut copied),
                None => {
                    copied.clear();
                    copied.resize(srcs.len(), 0);
                }
            }
            let name = basename(dests[d].path);
            let mut best: Vec<Scored> = Vec::with_capacity(4);
            for (i, &s) in srcs.iter().enumerate() {
                let score = match (src_spans[i], dst_spans) {
                    (Some(src), Some(dst)) if may_pair(src_size[i], dst_size[j]) => {
                        similarity::score_of(copied[i], src.size(), dst.size())
                    }
                    _ => 0,
                };
                keep_if_better(
                    &mut best,
                    Scored {
                        score,
                        same_name: src_names[i] == name,
                        src: s,
                        dst: d,
                    },
                );
            }
            all.extend(best);
        }
        all
    }
}

/// 짝 후보 하나(git의 diff_score). 정렬: 유사도가 높은 것, 같으면 파일 이름이 같은 것 먼저.
#[derive(Clone, Copy)]
struct Scored {
    score: u64,
    same_name: bool,
    src: usize,
    dst: usize,
}

/// `a`가 `b`보다 앞인가(git의 score_compare < 0)
fn better(a: &Scored, b: &Scored) -> bool {
    (a.score, a.same_name) > (b.score, b.same_name)
}

/// 새 파일 하나의 후보 넷에 넣는다(git의 record_if_better): 넷이 안 되면 넣고, 아니면 가장 뒤의 것보다 앞일 때 바꾼다
/// (가장 뒤가 여럿이면 먼저 넣은 것을 바꾸고, 같으면 먼저 본 것을 둔다).
fn keep_if_better(best: &mut Vec<Scored>, candidate: Scored) {
    if best.len() < 4 {
        best.push(candidate);
        return;
    }
    let worst = (0..best.len())
        .reduce(|w, i| if better(&best[w], &best[i]) { i } else { w })
        .unwrap_or(0);
    if better(&candidate, &best[worst]) {
        best[worst] = candidate;
    }
}

/// `sources`(지운 항목)와 `dests`(새 항목)의 rename 짝. `limit`은 [`RENAME_LIMIT`]처럼 쓴다.
pub(crate) fn find(
    repo: &gix::Repository,
    sources: &[Candidate],
    dests: &[Candidate],
    limit: usize,
) -> Found {
    let mut found = Found::default();
    if sources.is_empty() || dests.is_empty() {
        return found;
    }
    let mut src_used = vec![false; sources.len()];
    let mut dst_done = vec![false; dests.len()];

    // 1. 내용이 같은 파일
    let mut by_id: HashMap<ObjectId, Vec<usize>> = HashMap::new();
    for (i, src) in sources.iter().enumerate() {
        by_id.entry(src.id).or_default().push(i);
    }
    for (d, dst) in dests.iter().enumerate() {
        let Some(candidates) = by_id.get(&dst.id) else {
            continue;
        };
        let mut best: Option<(usize, u8)> = None;
        for &s in candidates {
            let src = &sources[s];
            if src_used[s]
                || ((!is_regular(src.kind) || !is_regular(dst.kind)) && src.kind != dst.kind)
            {
                continue;
            }
            let score = 1 + u8::from(basename(src.path) == basename(dst.path));
            if best.is_none_or(|(_, b)| score > b) {
                best = Some((s, score));
                if score == 2 {
                    break;
                }
            }
        }
        if let Some((s, _)) = best {
            src_used[s] = true;
            dst_done[d] = true;
            found.pairs.push((s, d, MAX_SCORE));
        }
    }

    let mut blobs = Blobs {
        repo,
        sizes: HashMap::new(),
        spans: HashMap::new(),
        incomplete: false,
    };

    // 2. 지운 쪽과 새 쪽에 하나씩뿐인 파일 이름
    let unique = |names: &mut HashMap<Vec<u8>, Option<usize>>, name: &[u8], i: usize| {
        names
            .entry(name.to_vec())
            .and_modify(|slot| *slot = None)
            .or_insert(Some(i));
    };
    let mut src_names: HashMap<Vec<u8>, Option<usize>> = HashMap::new();
    for s in (0..sources.len()).filter(|&s| !src_used[s]) {
        unique(&mut src_names, basename(sources[s].path), s);
    }
    let mut dst_names: HashMap<Vec<u8>, Option<usize>> = HashMap::new();
    for d in (0..dests.len()).filter(|&d| !dst_done[d]) {
        unique(&mut dst_names, basename(dests[d].path), d);
    }
    let basename_score = MINIMUM_SCORE + (MAX_SCORE - MINIMUM_SCORE) / 2;
    for s in 0..sources.len() {
        if src_used[s] {
            continue;
        }
        let name = basename(sources[s].path);
        let (Some(Some(_)), Some(&Some(d))) = (src_names.get(name), dst_names.get(name)) else {
            continue;
        };
        if dst_done[d] {
            continue;
        }
        let score = blobs.similarity(&sources[s], &dests[d]);
        if score >= basename_score {
            src_used[s] = true;
            dst_done[d] = true;
            found.pairs.push((s, d, score));
        }
    }

    // 3. 남은 짝을 모두 견준다.
    let srcs: Vec<usize> = (0..sources.len()).filter(|&s| !src_used[s]).collect();
    let dsts: Vec<usize> = (0..dests.len()).filter(|&d| !dst_done[d]).collect();
    if !srcs.is_empty() && !dsts.is_empty() {
        if srcs.len().saturating_mul(dsts.len()) > limit.saturating_mul(limit) {
            found.limited = true;
        } else {
            let mut all = blobs.matrix(sources, dests, &srcs, &dsts);
            // 안정 정렬: 같은 순위면 새 파일 순서, 그 안에서 넣은 순서
            all.sort_by_key(|pair| std::cmp::Reverse((pair.score, pair.same_name)));
            for pair in all {
                if pair.score < MINIMUM_SCORE {
                    break;
                }
                if dst_done[pair.dst] || src_used[pair.src] {
                    continue;
                }
                src_used[pair.src] = true;
                dst_done[pair.dst] = true;
                found.pairs.push((pair.src, pair.dst, pair.score));
            }
        }
    }
    found.incomplete = blobs.incomplete;
    found
}

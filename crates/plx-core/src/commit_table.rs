//! commit-graph가 없는 저장소를 위한 커밋 표: 커밋 객체를 풀어 읽은 커밋들의 (sha, 부모, 커밋 시각)을 앱 캐시에 둔다.
//!
//! 저장소에 commit-graph가 없으면 그래프의 첫 화면에도 이력 전체를 풀어야 한다(세대 번호가 없으면 어떤 시작점이
//! 다른 시작점의 조상인지 끝까지 걸어 봐야 안다: git/git 0.86초, vscode 1.8초). 커밋은 바뀌지 않으므로 한 번 읽은
//! 결과를 저장소 밖(앱 캐시)에 두면 다음부터는 commit-graph처럼 번호로 따라간다. 저장소는 건드리지 않는다.
//!
//! 표는 sha 순으로 두고 이진 탐색으로 찾는다. 부모는 표 안의 번호다(표는 닫혀 있다: 표에 있는 커밋의 부모도 표에
//! 있다). 그 뒤에 생긴 커밋만 표에 없어 객체에서 읽는다. 얕은 클론은 경계 너머가 나중에 채워질 수 있어 넣지 않는다.

use gix::ObjectId;
use gix::hash::oid;

/// 파일 형식(바뀌면 올린다)
const MAGIC: &[u8; 8] = b"plxcmt1\n";

#[derive(Debug, PartialEq, Eq)]
pub struct CommitTable {
    /// sha 오름차순
    ids: Vec<ObjectId>,
    /// 커밋 `i`의 부모는 `parents[starts[i]..starts[i + 1]]`
    starts: Vec<u32>,
    parents: Vec<u32>,
    /// 커미터 시각(초)
    times: Vec<i64>,
}

impl Default for CommitTable {
    fn default() -> Self {
        CommitTable {
            ids: Vec::new(),
            starts: vec![0],
            parents: Vec::new(),
            times: Vec::new(),
        }
    }
}

impl CommitTable {
    /// (sha, 부모 sha들, 커밋 시각)으로 만든다. 부모가 목록에 없는 커밋은 넣지 않는다(닫힌 표만 쓸모가 있다).
    pub fn from_commits(mut commits: Vec<(ObjectId, Vec<ObjectId>, i64)>) -> CommitTable {
        commits.sort_by_key(|a| a.0);
        commits.dedup_by(|a, b| a.0 == b.0);
        // 부모가 빠진 커밋을 빼면 그 자식도 빠져야 한다. 빠지는 것이 없을 때까지 되풀이한다(보통 한 번).
        loop {
            let ids: Vec<ObjectId> = commits.iter().map(|c| c.0).collect();
            let before = commits.len();
            commits.retain(|(_, parents, _)| parents.iter().all(|p| ids.binary_search(p).is_ok()));
            if commits.len() == before {
                break;
            }
        }
        let ids: Vec<ObjectId> = commits.iter().map(|c| c.0).collect();
        let mut table = CommitTable {
            starts: Vec::with_capacity(ids.len() + 1),
            parents: Vec::new(),
            times: Vec::with_capacity(ids.len()),
            ids: Vec::new(),
        };
        table.starts.push(0);
        for (_, parents, time) in &commits {
            table.parents.extend(
                parents
                    .iter()
                    .map(|p| ids.binary_search(p).expect("닫힌 표") as u32),
            );
            table.starts.push(table.parents.len() as u32);
            table.times.push(*time);
        }
        table.ids = ids;
        table
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn lookup(&self, id: &oid) -> Option<u32> {
        self.ids
            .binary_search_by(|probe| probe.as_ref().cmp(id))
            .ok()
            .map(|i| i as u32)
    }

    pub fn id(&self, index: u32) -> ObjectId {
        self.ids[index as usize]
    }

    pub fn parents(&self, index: u32) -> &[u32] {
        let i = index as usize;
        &self.parents[self.starts[i] as usize..self.starts[i + 1] as usize]
    }

    pub fn time(&self, index: u32) -> i64 {
        self.times[index as usize]
    }

    /// 다른 표와 합친다(겹치는 커밋은 하나로).
    pub fn merged(&self, more: Vec<(ObjectId, Vec<ObjectId>, i64)>) -> CommitTable {
        let mut all: Vec<(ObjectId, Vec<ObjectId>, i64)> = (0..self.len() as u32)
            .map(|i| {
                (
                    self.id(i),
                    self.parents(i).iter().map(|&p| self.id(p)).collect(),
                    self.time(i),
                )
            })
            .collect();
        all.extend(more);
        CommitTable::from_commits(all)
    }

    /// 형식: MAGIC, sha 길이(u32), 커밋 수 n(u32), 부모 수 e(u32), sha n개, starts n+1개(u32), 부모 e개(u32),
    /// 시각 n개(i64). 모두 little-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        let hash_len = self.ids.first().map_or(20, |id| id.as_bytes().len());
        let mut out =
            Vec::with_capacity(20 + self.ids.len() * (hash_len + 12) + self.parents.len() * 4 + 4);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(hash_len as u32).to_le_bytes());
        out.extend_from_slice(&(self.ids.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.parents.len() as u32).to_le_bytes());
        for id in &self.ids {
            out.extend_from_slice(id.as_bytes());
        }
        for start in &self.starts {
            out.extend_from_slice(&start.to_le_bytes());
        }
        for parent in &self.parents {
            out.extend_from_slice(&parent.to_le_bytes());
        }
        for time in &self.times {
            out.extend_from_slice(&time.to_le_bytes());
        }
        out
    }

    /// [`CommitTable::to_bytes`]의 결과를 읽는다. 형식이 맞지 않으면 `None`(캐시는 없어도 된다).
    pub fn from_bytes(bytes: &[u8]) -> Option<CommitTable> {
        let rest = bytes.strip_prefix(MAGIC.as_slice())?;
        let mut reader = Reader(rest);
        let hash_len = reader.u32()? as usize;
        if hash_len != 20 && hash_len != 32 {
            return None;
        }
        let n = reader.u32()? as usize;
        let e = reader.u32()? as usize;
        let expected = n
            .checked_mul(hash_len + 4 + 8)?
            .checked_add(4)?
            .checked_add(e.checked_mul(4)?)?;
        if reader.0.len() != expected {
            return None;
        }
        let ids: Vec<ObjectId> = (0..n)
            .map(|_| reader.take(hash_len).map(ObjectId::from_bytes_or_panic))
            .collect::<Option<_>>()?;
        let starts: Vec<u32> = (0..=n).map(|_| reader.u32()).collect::<Option<_>>()?;
        let parents: Vec<u32> = (0..e).map(|_| reader.u32()).collect::<Option<_>>()?;
        let times: Vec<i64> = (0..n).map(|_| reader.i64()).collect::<Option<_>>()?;
        // 손상된 파일이 범위를 벗어난 번호로 패닉을 내지 않게 확인한다.
        let sorted = ids.windows(2).all(|w| w[0] < w[1]);
        let ranges = starts.first() == Some(&0)
            && starts.last() == Some(&(e as u32))
            && starts.windows(2).all(|w| w[0] <= w[1]);
        let in_table = parents.iter().all(|&p| (p as usize) < n);
        (sorted && ranges && in_table).then_some(CommitTable {
            ids,
            starts,
            parents,
            times,
        })
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        if self.0.len() < len {
            return None;
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Some(head)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ObjectId {
        ObjectId::from_bytes_or_panic(&[n; 20])
    }

    fn sample() -> Vec<(ObjectId, Vec<ObjectId>, i64)> {
        vec![
            (id(9), vec![id(5), id(7)], 90),
            (id(5), vec![id(1)], 50),
            (id(7), vec![id(1)], 70),
            (id(1), vec![], 10),
        ]
    }

    #[test]
    fn lookup_parents_and_times() {
        let table = CommitTable::from_commits(sample());
        assert_eq!(table.len(), 4);
        let merge = table.lookup(&id(9)).unwrap();
        let parents: Vec<ObjectId> = table.parents(merge).iter().map(|&p| table.id(p)).collect();
        assert_eq!(parents, [id(5), id(7)], "부모 순서를 지킨다");
        assert_eq!(table.time(merge), 90);
        assert_eq!(table.lookup(&id(3)), None);
        assert!(table.parents(table.lookup(&id(1)).unwrap()).is_empty());
    }

    #[test]
    fn open_entries_are_dropped() {
        // 부모(2)가 없는 커밋 8과 그 자식 10은 뺀다.
        let mut commits = sample();
        commits.push((id(8), vec![id(2)], 80));
        commits.push((id(10), vec![id(8), id(9)], 100));
        let table = CommitTable::from_commits(commits);
        assert_eq!(table, CommitTable::from_commits(sample()));
    }

    #[test]
    fn bytes_round_trip_and_reject_damage() {
        let table = CommitTable::from_commits(sample());
        let bytes = table.to_bytes();
        assert_eq!(CommitTable::from_bytes(&bytes), Some(table));
        assert!(CommitTable::from_bytes(&bytes[..bytes.len() - 1]).is_none());
        assert!(CommitTable::from_bytes(b"plxcmt0\n").is_none());
        // 범위를 벗어난 부모 번호
        let mut bad = bytes.clone();
        let parents_at = 8 + 12 + 4 * 20 + 5 * 4;
        bad[parents_at..parents_at + 4].copy_from_slice(&99u32.to_le_bytes());
        assert!(CommitTable::from_bytes(&bad).is_none());
        assert_eq!(
            CommitTable::from_bytes(&CommitTable::default().to_bytes()),
            Some(CommitTable::default())
        );
    }

    #[test]
    fn merging_keeps_both() {
        let table = CommitTable::from_commits(sample());
        let merged = table.merged(vec![
            (id(11), vec![id(9)], 110),
            (id(9), vec![id(5), id(7)], 90),
        ]);
        assert_eq!(merged.len(), 5);
        let new = merged.lookup(&id(11)).unwrap();
        assert_eq!(merged.id(merged.parents(new)[0]), id(9));
    }
}

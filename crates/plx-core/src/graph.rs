//! 커밋 그래프: 시작점(HEAD, 브랜치)에서 닿는 커밋 DAG를 `git log --topo-order`와 같은 순서로 놓고 레인을 정한다.
//!
//! DAG는 commit-graph가 있으면 그 안의 위치 번호로 부모를 따라가 커밋 객체를 풀지 않는다.
//! commit-graph에 없는 커밋(마지막으로 쓴 뒤 생긴 커밋, commit-graph가 없는 저장소)만 객체를 푼다.
//! gix의 topo 워크(`gix::traverse::commit::topo`)는 쓰지 않는다. 세대 번호 v1(위상 깊이)만 읽어서 오래된 브랜치가
//! 시작점에 있으면 첫 커밋을 내기 전에 이력 대부분을 걷는다(vscode 원격 브랜치 5천 개: 첫 100개 0.3~0.45초).
//! git은 세대 번호 v2(보정한 커밋 시각)로 이를 피하지만 gix-commitgraph는 아직 v2를 읽지 않는다.
//!
//! 순서는 git의 `sort_in_topological_order`(`--topo-order`)와 같다. 시작점을 커밋 시각 내림차순(같으면 준 순서)으로
//! 스택에 쌓고, 꺼낸 커밋의 부모는 그 부모의 자식을 모두 낸 뒤에 스택에 넣는다(LIFO). 그래서 merge 다음에는
//! 두 번째 부모 쪽 가지가 먼저 나온다(`git log --graph`와 같다).
//!
//! 레이아웃은 행 하나만 보고 그릴 수 있게 반 칸 선으로 나눈다: 줄을 곧게 지나는 레인, 위 가장자리에서 커밋 점으로
//! 들어오는 선, 커밋 점에서 아래 가장자리로 나가는 선. 레인은 옆으로 옮기지 않고(선이 밀리지 않는다) 비면 새 가지가
//! 다시 쓴다. 행마다 레이아웃을 저장하지 않고 [`CHECKPOINT`]행마다 레인 상태만 두었다가, 화면이 요청한 범위를 가장
//! 가까운 지점부터 다시 계산한다.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gix::ObjectId;
use gix::bstr::{BString, ByteSlice};
use gix::commitgraph::{Graph, Position};
use gix::objs::Find;
use serde::{Deserialize, Serialize};

use crate::cache::{CacheKey, DiskCache};
use crate::commit_table::CommitTable;
use crate::git::{self, Canceller, Streaming};
use crate::repo::{Repo, RepoCache, decode_text, person};

/// 레인 색 수(화면 팔레트와 같다)
pub const COLORS: u8 = 8;
/// 첫 화면으로 바로 보내는 행 수
pub const PAGE: usize = 100;
/// 이 행 수마다 레인 상태를 저장한다
pub const CHECKPOINT: usize = 256;
/// 진행을 이 간격보다 자주 알리지 않는다
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);
/// 커밋 객체를 풀 때 쓰는 캐시
const OBJECT_CACHE_BYTES: usize = 64 << 20;
/// 빈 레인, 아직 번호가 없는 위치
const NONE: u32 = u32::MAX;

// ---------------------------------------------------------------- 레이아웃

/// 행 하나의 레이아웃. 레인 번호는 왼쪽부터 0이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowLayout {
    /// 커밋 점이 놓인 레인
    pub lane: u16,
    pub color: u8,
    /// 커밋을 지나쳐 가는 선: (위 가장자리 레인, 아래 가장자리 레인, 색). 대부분 곧게 내려가고, 같은 부모를 기다리던
    /// 레인을 이 커밋의 레인으로 합칠 때만 비스듬히 간다.
    pub through: Vec<[u16; 3]>,
    /// 위 가장자리의 이 레인에서 커밋 점으로 들어온다: (레인, 색)
    pub up: Vec<[u16; 2]>,
    /// 커밋 점에서 아래 가장자리의 이 레인으로 나간다(부모 순서대로): (레인, 색)
    pub down: Vec<[u16; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Lane {
    /// 이 레인이 기다리는 노드. [`NONE`]이면 빈 레인.
    node: u32,
    color: u8,
}

/// 토폴로지 순으로 노드를 받아 레인을 정한다. 노드는 자식보다 먼저 오지 않아야 한다.
#[derive(Debug, Clone, Default)]
pub struct LaneLayout {
    lanes: Vec<Lane>,
    next_color: u8,
}

impl LaneLayout {
    pub fn new() -> Self {
        Self::default()
    }

    /// 지금 열린(끝의 빈 레인을 뺀) 레인 수
    pub fn lanes(&self) -> usize {
        self.lanes.len()
    }

    fn color(&mut self) -> u8 {
        let color = self.next_color;
        self.next_color = (self.next_color + 1) % COLORS;
        color
    }

    /// 빈 레인(`from` 이후를 먼저, 없으면 앞쪽, 그래도 없으면 새로)
    fn free_lane(&mut self, from: usize) -> usize {
        let len = self.lanes.len();
        let found = (from.min(len)..len)
            .chain(0..from.min(len))
            .find(|&i| self.lanes[i].node == NONE);
        found.unwrap_or_else(|| {
            self.lanes.push(Lane {
                node: NONE,
                color: 0,
            });
            len
        })
    }

    /// `node`를 놓는다. 레인마다 기다리는 커밋은 하나뿐이다: 이미 다른 레인이 기다리는 부모로 가는 선은 그 레인에
    /// 합친다(git의 `git log --graph`처럼 왼쪽 레인이 남는다). 그래서 오래된 커밋에서 갈라진 가지가 많아도 레인이
    /// 가지마다 열려 있지 않다.
    pub fn place(&mut self, node: u32, parents: &[u32]) -> RowLayout {
        let mut lane = None;
        let mut through = Vec::new();
        let mut up = Vec::new();
        for (i, l) in self.lanes.iter_mut().enumerate() {
            if l.node == NONE {
                continue;
            }
            if l.node == node {
                lane.get_or_insert((i, l.color));
                up.push([i as u16, u16::from(l.color)]);
                l.node = NONE;
            } else {
                through.push([i as u16, i as u16, u16::from(l.color)]);
            }
        }
        // 아무도 기다리지 않으면 가지 끝이다.
        let (lane, color) = match lane {
            Some(found) => found,
            None => (self.free_lane(0), self.color()),
        };

        let mut down = Vec::with_capacity(parents.len());
        if let Some((&first, rest)) = parents.split_first() {
            match self.lanes.iter().position(|l| l.node == first) {
                // 왼쪽 레인이 이미 첫 부모를 기다린다: 이 가지는 그 레인으로 들어가며 끝난다.
                Some(j) if j < lane => down.push([j as u16, u16::from(color)]),
                // 오른쪽 레인이 기다린다: 그 레인을 이 커밋의 레인으로 합친다(색은 이 커밋의 것을 잇는다).
                Some(j) => {
                    self.lanes[j].node = NONE;
                    if let Some(line) = through.iter_mut().find(|t| usize::from(t[0]) == j) {
                        line[1] = lane as u16;
                    }
                    self.lanes[lane] = Lane { node: first, color };
                    down.push([lane as u16, u16::from(color)]);
                }
                None => {
                    self.lanes[lane] = Lane { node: first, color };
                    down.push([lane as u16, u16::from(color)]);
                }
            }
            for &parent in rest {
                // merge한 가지의 선은 그 가지의 색이다.
                let to = match self.lanes.iter().position(|l| l.node == parent) {
                    Some(j) => j,
                    None => {
                        let j = self.free_lane(lane + 1);
                        let color = self.color();
                        self.lanes[j] = Lane {
                            node: parent,
                            color,
                        };
                        j
                    }
                };
                down.push([to as u16, u16::from(self.lanes[to].color)]);
            }
        }
        while self.lanes.last().is_some_and(|l| l.node == NONE) {
            self.lanes.pop();
        }
        RowLayout {
            lane: lane as u16,
            color,
            through,
            up,
            down,
        }
    }
}

// ---------------------------------------------------------------- DAG와 순서

/// 시작점에서 닿는 커밋들. 노드 번호는 발견한 순서다.
#[derive(Debug, Default)]
pub struct Dag {
    ids: Vec<ObjectId>,
    /// 노드마다 `parents`에서의 (시작, 개수)
    ranges: Vec<(u32, u32)>,
    parents: Vec<u32>,
}

impl Dag {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn id(&self, node: u32) -> ObjectId {
        self.ids[node as usize]
    }

    pub fn parents(&self, node: u32) -> &[u32] {
        let (start, len) = self.ranges[node as usize];
        &self.parents[start as usize..(start + len) as usize]
    }

    /// 노드 `i`의 부모가 `parents[i]`인 DAG(테스트용, sha는 노드 번호로 만든다)
    pub fn from_parents(parents: &[&[u32]]) -> Dag {
        let mut dag = Dag::default();
        for (i, list) in parents.iter().enumerate() {
            let mut bytes = [0u8; 20];
            bytes[..4].copy_from_slice(&(i as u32).to_be_bytes());
            dag.ids.push(ObjectId::from_bytes_or_panic(&bytes));
            dag.ranges
                .push((dag.parents.len() as u32, list.len() as u32));
            dag.parents.extend_from_slice(list);
        }
        dag
    }
}

/// `git log --topo-order <tips>`의 순서. `tips`는 (노드, 커밋 시각)을 git에 준 순서로.
/// DAG의 모든 노드가 시작점에서 닿아야 한다.
pub fn topo_order(dag: &Dag, tips: &[(u32, i64)]) -> Vec<u32> {
    // git과 같이 indegree는 1(목록에 있음) + 목록 안의 자식 수다.
    let mut indegree = vec![1u32; dag.len()];
    for node in 0..dag.len() as u32 {
        for &parent in dag.parents(node) {
            indegree[parent as usize] += 1;
        }
    }
    let mut start: Vec<(u32, i64)> = tips
        .iter()
        .copied()
        .filter(|&(node, _)| indegree[node as usize] == 1)
        .collect();
    // 안정 정렬: 시각이 같으면 준 순서를 지킨다.
    start.sort_by_key(|&(_, time)| std::cmp::Reverse(time));
    let mut stack: Vec<u32> = start.iter().rev().map(|&(node, _)| node).collect();
    let mut order = Vec::with_capacity(dag.len());
    while let Some(node) = stack.pop() {
        for &parent in dag.parents(node) {
            let degree = &mut indegree[parent as usize];
            if *degree == 0 {
                continue;
            }
            *degree -= 1;
            // 부모는 자식을 모두 낸 뒤에 낸다.
            if *degree == 1 {
                stack.push(parent);
            }
        }
        indegree[node as usize] = 0;
        order.push(node);
    }
    order
}

/// DAG를 읽는다. 커밋은 commit-graph(위치 번호), 커밋 표(앱 캐시, 표 안의 번호), 커밋 객체 순으로 찾는다.
struct Loader<'a> {
    repo: &'a gix::Repository,
    graph: Option<&'a Graph>,
    table: Option<&'a CommitTable>,
    shallow: HashSet<ObjectId>,
    rewrites: &'a Rewrites,
    dag: Dag,
    /// 노드마다 commit-graph 위치([`NONE`]이면 그래프에 없다)
    position: Vec<u32>,
    /// 노드마다 커밋 표의 번호([`NONE`]이면 표에 없다)
    table_index: Vec<u32>,
    /// 노드마다 커밋 시각(표·객체에서 읽었을 때, 아니면 [`UNKNOWN_TIME`])
    time: Vec<i64>,
    node_of_position: Vec<u32>,
    node_of_table: Vec<u32>,
    node_of_id: HashMap<ObjectId, u32>,
    /// 객체가 없는 노드(얕은 클론 경계 너머, 손상)
    missing: Vec<u32>,
    from_table: usize,
    from_objects: usize,
    stack: Vec<u32>,
    buf: Vec<u8>,
    parent_ids: Vec<ObjectId>,
}

const UNKNOWN_TIME: i64 = i64::MIN;

impl<'a> Loader<'a> {
    fn new(
        repo: &'a gix::Repository,
        graph: Option<&'a Graph>,
        table: Option<&'a CommitTable>,
        shallow: HashSet<ObjectId>,
        rewrites: &'a Rewrites,
    ) -> Self {
        Loader {
            repo,
            graph,
            table,
            shallow,
            rewrites,
            dag: Dag::default(),
            position: Vec::new(),
            table_index: Vec::new(),
            time: Vec::new(),
            node_of_position: vec![NONE; graph.map_or(0, |g| g.num_commits() as usize)],
            node_of_table: vec![NONE; table.map_or(0, CommitTable::len)],
            node_of_id: HashMap::new(),
            missing: Vec::new(),
            from_table: 0,
            from_objects: 0,
            stack: Vec::new(),
            buf: Vec::new(),
            parent_ids: Vec::new(),
        }
    }

    fn push_node(&mut self, id: ObjectId, position: u32, table_index: u32) -> u32 {
        let node = self.dag.ids.len() as u32;
        self.dag.ids.push(id);
        self.dag.ranges.push((0, 0));
        self.position.push(position);
        self.table_index.push(table_index);
        self.time.push(UNKNOWN_TIME);
        self.stack.push(node);
        node
    }

    fn node_at(&mut self, graph: &Graph, position: u32) -> u32 {
        let node = self.node_of_position[position as usize];
        if node != NONE {
            return node;
        }
        let id = graph.id_at(Position(position)).to_owned();
        let node = self.push_node(id, position, NONE);
        self.node_of_position[position as usize] = node;
        node
    }

    fn node_in_table(&mut self, table: &CommitTable, index: u32) -> u32 {
        let node = self.node_of_table[index as usize];
        if node != NONE {
            return node;
        }
        let node = self.push_node(table.id(index), NONE, index);
        self.node_of_table[index as usize] = node;
        node
    }

    fn node_for(&mut self, id: ObjectId) -> u32 {
        if let Some(graph) = self.graph
            && let Some(position) = graph.lookup(id)
        {
            return self.node_at(graph, position.0);
        }
        if let Some(table) = self.table
            && let Some(index) = table.lookup(&id)
        {
            return self.node_in_table(table, index);
        }
        if let Some(&node) = self.node_of_id.get(&id) {
            return node;
        }
        let node = self.push_node(id, NONE, NONE);
        self.node_of_id.insert(id, node);
        node
    }

    /// 시작점의 노드. commit-graph·커밋 표에 없는 시작점이 태그 객체면(태그를 `refs/remotes/` 아래로 받은 느슨한
    /// 참조 등) 커밋까지 벗기고 그 커밋을 함께 돌려준다. 그래프·표에 있는 시작점은 커밋이라 객체를 읽지 않는다.
    fn tip_node(&mut self, id: ObjectId) -> (u32, Option<ObjectId>) {
        if let Some(graph) = self.graph
            && let Some(position) = graph.lookup(id)
        {
            return (self.node_at(graph, position.0), None);
        }
        if let Some(table) = self.table
            && let Some(index) = table.lookup(&id)
        {
            return (self.node_in_table(table, index), None);
        }
        match peel_tag(self.repo, id) {
            Some(commit) if commit != id => (self.node_for(commit), Some(commit)),
            _ => (self.node_for(id), None),
        }
    }

    fn expand(&mut self, node: u32) -> Result<(), String> {
        let start = self.dag.parents.len() as u32;
        let position = self.position[node as usize];
        let index = self.table_index[node as usize];
        if let (Some(graph), true) = (self.graph, position != NONE) {
            for parent in graph.commit_at(Position(position)).iter_parents() {
                let parent = parent.map_err(|e| format!("commit-graph 읽기 실패: {e}"))?;
                let parent = self.node_at(graph, parent.0);
                self.dag.parents.push(parent);
            }
        } else if let (Some(table), true) = (self.table, index != NONE) {
            self.from_table += 1;
            self.time[node as usize] = table.time(index);
            for &parent in table.parents(index) {
                let parent = self.node_in_table(table, parent);
                self.dag.parents.push(parent);
            }
        } else {
            let id = self.dag.ids[node as usize];
            self.from_objects += 1;
            let mut parent_ids = std::mem::take(&mut self.parent_ids);
            parent_ids.clear();
            // `git replace`로 바꾼 커밋은 바꾼 객체의 부모·시각을 읽는다(git log도 그렇게 보인다).
            match self
                .repo
                .objects
                .try_find(&self.rewrites.object(id), &mut self.buf)
            {
                Ok(Some(data)) => match data.try_into_commit_iter() {
                    Some(commit) => {
                        for token in commit {
                            match token {
                                Ok(gix::objs::commit::ref_iter::Token::Parent { id }) => {
                                    parent_ids.push(id)
                                }
                                Ok(gix::objs::commit::ref_iter::Token::Committer { signature }) => {
                                    self.time[node as usize] =
                                        signature.time().map_or(0, |time| time.seconds);
                                    break;
                                }
                                Ok(_) => {}
                                Err(e) => return Err(format!("커밋 {id} 해석 실패: {e}")),
                            }
                        }
                        if let Some(parents) = self.rewrites.grafts.get(&id) {
                            parent_ids.clear();
                            parent_ids.extend_from_slice(parents);
                        }
                        // 얕은 클론의 경계 커밋은 git처럼 부모가 없는 것으로 본다.
                        if self.shallow.contains(&id) {
                            parent_ids.clear();
                        }
                    }
                    None => self.missing.push(node),
                },
                Ok(None) => self.missing.push(node),
                Err(e) => return Err(format!("커밋 {id} 읽기 실패: {e}")),
            }
            for &parent in &parent_ids {
                let parent = self.node_for(parent);
                self.dag.parents.push(parent);
            }
            self.parent_ids = parent_ids;
        }
        self.dag.ranges[node as usize] = (start, self.dag.parents.len() as u32 - start);
        Ok(())
    }

    /// 객체가 없는 노드를 부모 목록에서 뺀다(없는 부모를 기다리는 레인이 열리지 않게).
    fn drop_missing(&mut self) -> HashSet<u32> {
        let missing: HashSet<u32> = self.missing.iter().copied().collect();
        if missing.is_empty() {
            return missing;
        }
        let mut parents = Vec::with_capacity(self.dag.parents.len());
        for node in 0..self.dag.len() as u32 {
            let start = parents.len() as u32;
            if !missing.contains(&node) {
                parents.extend(
                    self.dag
                        .parents(node)
                        .iter()
                        .filter(|p| !missing.contains(p)),
                );
            }
            self.dag.ranges[node as usize] = (start, parents.len() as u32 - start);
        }
        self.dag.parents = parents;
        missing
    }

    /// 노드의 커밋 시각(시작점 정렬용)
    fn commit_time(&self, node: u32) -> i64 {
        let time = self.time[node as usize];
        if time != UNKNOWN_TIME {
            return time;
        }
        let position = self.position[node as usize];
        match (self.graph, position != NONE) {
            (Some(graph), true) => graph.commit_at(Position(position)).committer_timestamp() as i64,
            _ => 0,
        }
    }

    /// 객체에서 읽은 커밋으로 새 커밋 표를 만든다. commit-graph가 없고 객체에서 많이 읽었을 때만 쓸모가 있다.
    fn new_table(&self, missing: &HashSet<u32>, min_new: usize) -> Option<CommitTable> {
        let new = self.from_objects - missing.len();
        // 객체를 읽지 못한 커밋이 있으면(대체 저장소가 잠시 없는 등) 그 자식의 부모가 빠진 채 적혀, 객체가 돌아와도
        // 표가 이력을 끊는다. 얕은 클론·바꾼 부모(`git replace`, graft)는 객체와 다른 부모라 적지 않는다.
        if self.graph.is_some()
            || !self.shallow.is_empty()
            || !self.rewrites.is_empty()
            || !missing.is_empty()
            || new == 0
            || new < min_new
        {
            return None;
        }
        let commits: Vec<(ObjectId, Vec<ObjectId>, i64)> = (0..self.dag.len() as u32)
            .filter(|&node| self.table_index[node as usize] == NONE && !missing.contains(&node))
            .map(|node| {
                let parents = self
                    .dag
                    .parents(node)
                    .iter()
                    .map(|&p| self.dag.id(p))
                    .collect();
                (self.dag.id(node), parents, self.commit_time(node))
            })
            .collect();
        Some(match self.table {
            Some(table) => table.merged(commits),
            None => CommitTable::from_commits(commits),
        })
    }
}

/// 커밋 표에 없는 커밋을 이만큼 넘게 객체에서 읽으면 표를 새로 쓴다(그보다 적으면 다음에도 객체에서 읽는다:
/// 커밋 1000개 약 10ms).
const TABLE_MIN_NEW: usize = 1000;

/// 읽은 그래프: DAG와 순서. 워크가 끝나면 바뀌지 않는다.
struct Built {
    dag: Dag,
    order: Vec<u32>,
    /// 노드 → 행([`NONE`]이면 순서에 없는 노드)
    row_of: Vec<u32>,
    /// sha → 노드(행 찾기용)
    graph: Option<Graph>,
    node_of_position: Vec<u32>,
    table: Option<Arc<CommitTable>>,
    node_of_table: Vec<u32>,
    node_of_id: HashMap<ObjectId, u32>,
    /// 행의 요약·작성자와 검색도 바꾼 객체에서 읽는다.
    rewrites: Arc<Rewrites>,
}

/// git이 객체와 다른 내용·부모를 보는 커밋: 객체를 바꾸는 참조(`git replace`, `refs/replace/<원래 sha>`)와
/// `info/grafts`. 이것이 있거나 얕은 클론이면 git은 commit-graph를 쓰지 않는다(commit_graph_compatible). 우리도
/// commit-graph와 커밋 표를 쓰지 않고 객체에서 바꾼 부모를 읽는다. gix 0.89는 `core.useReplaceRefs`를 거꾸로 읽어
/// (설정이 없거나 true면 끈다) 객체를 바꾸지 않으므로 직접 한다. 경로 검색(git log)은 git이 바꿔 준다.
#[derive(Debug, Default)]
struct Rewrites {
    /// 원래 커밋 → 그 내용 대신 읽을 객체
    replace: HashMap<ObjectId, ObjectId>,
    /// 커밋 → 그 부모 대신 쓸 부모
    grafts: HashMap<ObjectId, Vec<ObjectId>>,
}

impl Rewrites {
    fn is_empty(&self) -> bool {
        self.replace.is_empty() && self.grafts.is_empty()
    }

    /// 내용을 읽을 객체
    fn object(&self, id: ObjectId) -> ObjectId {
        self.replace.get(&id).copied().unwrap_or(id)
    }

    /// git처럼 `core.useReplaceRefs=false`나 `GIT_NO_REPLACE_OBJECTS`면 바꾸지 않는다.
    fn replace_enabled(repo: &gix::Repository) -> bool {
        std::env::var_os("GIT_NO_REPLACE_OBJECTS").is_none()
            && repo.config_snapshot().boolean("core.useReplaceRefs") != Some(false)
    }

    /// 바꾸는 참조가 놓이는 곳(`GIT_REPLACE_REF_BASE`, 기본 `refs/replace/`)
    fn replace_base() -> Vec<u8> {
        let mut base = std::env::var("GIT_REPLACE_REF_BASE")
            .ok()
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| "refs/replace/".to_owned())
            .into_bytes();
        if !base.ends_with(b"/") {
            base.push(b'/');
        }
        base
    }

    /// `<공통 디렉터리>/info/grafts`: 줄마다 `<커밋> <부모>*`, `#`은 주석
    fn read_grafts(common_dir: &Path) -> HashMap<ObjectId, Vec<ObjectId>> {
        let Ok(text) = std::fs::read_to_string(common_dir.join("info").join("grafts")) else {
            return HashMap::new();
        };
        text.lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .filter_map(|line| {
                let mut ids = line
                    .split_whitespace()
                    .map(|hex| ObjectId::from_hex(hex.as_bytes()).ok());
                let commit = ids.next()??;
                let parents: Option<Vec<ObjectId>> = ids.collect();
                Some((commit, parents?))
            })
            .collect()
    }
}

impl Built {
    fn row_of_id(&self, id: ObjectId) -> Option<usize> {
        let node = if let Some(position) = self.graph.as_ref().and_then(|g| g.lookup(id)) {
            self.node_of_position[position.0 as usize]
        } else if let Some(index) = self.table.as_ref().and_then(|t| t.lookup(&id)) {
            self.node_of_table[index as usize]
        } else {
            *self.node_of_id.get(&id)?
        };
        let row = *self.row_of.get(node as usize)?;
        (row != NONE).then_some(row as usize)
    }
}

struct LoadStats {
    load_ms: f64,
    sort_ms: f64,
    from_graph: usize,
    from_table: usize,
    from_objects: usize,
}

/// `tips`에서 닿는 DAG를 읽고 순서를 정한다. 취소되면 `None`. 새로 쓸 커밋 표가 있으면 같이 돌려준다.
#[allow(clippy::too_many_arguments)]
fn build(
    repo: &gix::Repository,
    graph: Option<Graph>,
    table: Option<Arc<CommitTable>>,
    table_min_new: usize,
    tips: &[ObjectId],
    rewrites: &Arc<Rewrites>,
    cancelled: &AtomicBool,
    on_progress: &mut dyn FnMut(usize) -> bool,
) -> Result<Option<Loaded>, String> {
    let started = Instant::now();
    let shallow = match repo.shallow_commits() {
        Ok(Some(commits)) => commits.iter().copied().collect(),
        _ => HashSet::new(),
    };
    let mut loader = Loader::new(repo, graph.as_ref(), table.as_deref(), shallow, rewrites);
    let mut peeled = Vec::new();
    let mut tip_nodes: Vec<u32> = Vec::with_capacity(tips.len());
    for &id in tips {
        let (node, commit) = loader.tip_node(id);
        if let Some(commit) = commit {
            peeled.push((id, commit));
            // 벗긴 태그가 다른 시작점과 같은 커밋일 수 있다.
            if tip_nodes.contains(&node) {
                continue;
            }
        }
        tip_nodes.push(node);
    }
    let mut last = Instant::now();
    let mut expanded = 0usize;
    while let Some(node) = loader.stack.pop() {
        loader.expand(node)?;
        expanded += 1;
        if expanded.is_multiple_of(4096) {
            if cancelled.load(Ordering::Relaxed) {
                return Ok(None);
            }
            if last.elapsed() >= PROGRESS_INTERVAL {
                last = Instant::now();
                if !on_progress(loader.dag.len()) {
                    return Ok(None);
                }
            }
        }
    }
    let missing = loader.drop_missing();
    let tips: Vec<(u32, i64)> = tip_nodes
        .into_iter()
        .filter(|node| !missing.contains(node))
        .map(|node| (node, loader.commit_time(node)))
        .collect();
    let load_ms = ms(started);

    let sorting = Instant::now();
    let mut order = topo_order(&loader.dag, &tips);
    // 객체가 없는 노드는 자식 없이 남아(부모 목록에서 뺐다) 시작점이 아니므로 순서에 들어가지 않는다.
    order.retain(|node| !missing.contains(node));
    let mut row_of = vec![NONE; loader.dag.len()];
    for (row, &node) in order.iter().enumerate() {
        row_of[node as usize] = row as u32;
    }
    let stats = LoadStats {
        load_ms,
        sort_ms: ms(sorting),
        from_graph: loader.dag.len() - loader.from_table - loader.from_objects,
        from_table: loader.from_table,
        from_objects: loader.from_objects - missing.len(),
    };
    let new_table = loader.new_table(&missing, table_min_new);
    let Loader {
        dag,
        node_of_position,
        node_of_table,
        node_of_id,
        ..
    } = loader;
    Ok(Some((
        Built {
            dag,
            order,
            row_of,
            graph,
            node_of_position,
            table,
            node_of_table,
            node_of_id,
            rewrites: rewrites.clone(),
        },
        stats,
        new_table,
        peeled,
    )))
}

/// [`build`]의 결과: 그래프, 측정, 새로 쓸 커밋 표, 벗긴 시작점(태그 → 커밋)
type Loaded = (
    Built,
    LoadStats,
    Option<CommitTable>,
    Vec<(ObjectId, ObjectId)>,
);

/// 검색을 새 검색이 멈췄을 때의 오류(화면은 무시한다)
pub const CANCELLED: &str = "취소했다";

/// 커밋 메시지(모든 줄, 커밋 인코딩대로)·작성자 이름·메일에 `query`(소문자)가 들어 있는가.
fn commit_matches(repo: &gix::Repository, id: ObjectId, query: &str, buf: &mut Vec<u8>) -> bool {
    let Ok(Some(data)) = repo.objects.try_find(&id, buf) else {
        return false;
    };
    let Ok(gix::objs::ObjectRef::Commit(commit)) = data.decode() else {
        return false;
    };
    let encoding = commit.encoding;
    if let Ok(author) = commit.author()
        && (decode_text(author.name, encoding)
            .to_lowercase()
            .contains(query)
            || decode_text(author.email, encoding)
                .to_lowercase()
                .contains(query))
    {
        return true;
    }
    decode_text(commit.message, encoding)
        .to_lowercase()
        .contains(query)
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

// ---------------------------------------------------------------- 참조

/// 커밋을 가리키는 이름표
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    /// `main`, `origin/main`, `v1.0`, `HEAD`
    pub name: String,
    pub kind: LabelKind,
    /// HEAD가 가리키는 브랜치(detached면 `HEAD` 이름표)
    pub current: bool,
    /// 다른 워크트리가 체크아웃한 브랜치·HEAD면 그 워크트리 이름
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelKind {
    /// 브랜치가 아닌 HEAD(detached)
    Head,
    Branch,
    Remote,
    Tag,
}

/// 어떤 커밋에서 걷기 시작할지
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GraphScope {
    /// HEAD만(`git log HEAD`)
    Head,
    /// HEAD, 로컬 브랜치, 로컬 브랜치의 upstream(`git log HEAD --branches <upstream>…`)
    #[default]
    Branches,
    /// 원격 브랜치까지 모두(`git log HEAD --branches --remotes`)
    All,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphOptions {
    #[serde(default)]
    pub scope: GraphScope,
}

impl GraphOptions {
    pub fn scope(scope: GraphScope) -> GraphOptions {
        GraphOptions { scope }
    }
}

struct Refs {
    labels: HashMap<ObjectId, Vec<Label>>,
    /// 걷기 시작할 커밋: HEAD, 로컬 브랜치, 원격 브랜치(git log에 이 순서로 준 것과 같다)
    tips: Vec<ObjectId>,
    head: Option<ObjectId>,
    branch: Option<String>,
    /// 시작점으로 쓰지 않은 로컬·원격 브랜치 수
    hidden_branches: usize,
    hidden_remotes: usize,
    /// 같은 시작점을 git log에 주는 인자(경로 검색용)
    revs: Vec<String>,
    rewrites: Rewrites,
}

fn read_refs(repo: &Repo, options: GraphOptions) -> Result<Refs, String> {
    let local = repo.local();
    let head = repo.head().map_err(|e| e.to_string())?;
    let head_name = local.head_name().map_err(|e| format!("HEAD: {e}"))?;
    let branch = head_name.as_ref().and_then(|name| {
        name.as_bstr()
            .strip_prefix(b"refs/heads/")
            .map(|b| b.to_str_lossy().into_owned())
    });

    let mut branches: Vec<(BString, ObjectId)> = Vec::new();
    let mut remotes: Vec<(BString, ObjectId)> = Vec::new();
    let mut tags: Vec<(BString, ObjectId)> = Vec::new();
    let mut rewrites = Rewrites {
        replace: HashMap::new(),
        grafts: Rewrites::read_grafts(local.common_dir()),
    };
    let replace_base = Rewrites::replace_enabled(&local).then(Rewrites::replace_base);
    let platform = local.references().map_err(|e| format!("참조: {e}"))?;
    for reference in platform.all().map_err(|e| format!("참조: {e}"))? {
        // 읽지 못한 참조 하나 때문에 그래프 전체를 포기하지 않는다.
        let Ok(reference) = reference else { continue };
        let name = reference.name().as_bstr().to_owned();
        let id = match reference.target() {
            // packed-refs의 annotated 태그는 벗긴 값이 같이 있다. 브랜치는 커밋을 가리킨다고 보고 객체를 읽지 않는다
            // (vscode 참조 6천 개를 모두 벗기면 70ms).
            gix::refs::TargetRef::Object(id) => match reference.inner.peeled {
                Some(peeled) => peeled,
                None if name.starts_with(b"refs/tags/") => match peel_tag(&local, id.to_owned()) {
                    Some(id) => id,
                    None => continue,
                },
                None => id.to_owned(),
            },
            // origin/HEAD 같은 기호 참조
            gix::refs::TargetRef::Symbolic(_) => match reference.into_fully_peeled_id() {
                Ok(id) => id.detach(),
                Err(_) => continue,
            },
        };
        if let Some(short) = name.strip_prefix(b"refs/heads/") {
            branches.push((short.into(), id));
        } else if let Some(short) = name.strip_prefix(b"refs/remotes/") {
            remotes.push((short.into(), id));
        } else if let Some(short) = name.strip_prefix(b"refs/tags/") {
            tags.push((short.into(), id));
        } else if let Some(base) = &replace_base
            && let Some(original) = name.strip_prefix(base.as_slice())
            && let Ok(original) = ObjectId::from_hex(original)
        {
            rewrites.replace.insert(original, id);
        }
    }
    // git의 `--branches`·`--remotes`처럼 이름 순
    branches.sort();
    remotes.sort();

    let scope = options.scope;
    let upstreams: HashSet<BString> = if scope != GraphScope::Branches {
        HashSet::new()
    } else {
        branches
            .iter()
            .filter_map(|(short, _)| {
                let full: BString = [b"refs/heads/".as_slice(), short.as_slice()]
                    .concat()
                    .into();
                let full = gix::refs::FullName::try_from(full).ok()?;
                let upstream = local
                    .branch_remote_tracking_ref_name(full.as_ref(), gix::remote::Direction::Fetch)?
                    .ok()?;
                upstream
                    .as_bstr()
                    .strip_prefix(b"refs/remotes/")
                    .map(BString::from)
            })
            .collect()
    };

    let mut tips: Vec<ObjectId> = Vec::new();
    let mut seen: HashSet<ObjectId> = HashSet::new();
    let mut add_tip = |id: ObjectId| {
        if seen.insert(id) {
            tips.push(id);
        }
    };
    if let Some(head) = head {
        add_tip(head);
    }
    let mut hidden_branches = 0;
    for (short, id) in &branches {
        if scope != GraphScope::Head {
            add_tip(*id);
        } else if branch.as_deref().map(str::as_bytes) != Some(short.as_slice()) {
            hidden_branches += 1;
        }
    }
    let mut hidden_remotes = 0;
    for (short, id) in &remotes {
        if scope == GraphScope::All || upstreams.contains(short) {
            add_tip(*id);
        } else if !short.ends_with(b"/HEAD") {
            hidden_remotes += 1;
        }
    }
    // 다른 워크트리의 detached HEAD도 시작점이다(`git log --all`처럼). 그 워크트리에서 브랜치 없이 한 일이 보인다.
    let elsewhere = crate::worktree::checked_out_elsewhere(repo);
    if scope != GraphScope::Head {
        for (id, _) in &elsewhere.detached {
            add_tip(*id);
        }
    }
    let mut revs: Vec<String> = Vec::new();
    if head.is_some() {
        revs.push("HEAD".to_owned());
    }
    match scope {
        GraphScope::Head => {}
        GraphScope::Branches => {
            revs.push("--branches".to_owned());
            revs.extend(
                remotes
                    .iter()
                    .filter(|(short, _)| upstreams.contains(short))
                    .map(|(short, _)| format!("refs/remotes/{}", short.to_str_lossy())),
            );
        }
        GraphScope::All => revs.extend(["--branches".to_owned(), "--remotes".to_owned()]),
    }
    if scope != GraphScope::Head {
        revs.extend(elsewhere.detached.iter().map(|(id, _)| id.to_string()));
    }

    let mut labels: HashMap<ObjectId, Vec<Label>> = HashMap::new();
    let mut label = |id: ObjectId, name: &BString, kind: LabelKind| {
        let name = name.to_str_lossy().into_owned();
        let current = kind == LabelKind::Branch && branch.as_deref() == Some(name.as_str());
        let worktree = (kind == LabelKind::Branch)
            .then(|| elsewhere.branches.get(&name).map(|names| names.join(", ")))
            .flatten();
        labels.entry(id).or_default().push(Label {
            name,
            kind,
            current,
            worktree,
        });
    };
    for (name, id) in &branches {
        label(*id, name, LabelKind::Branch);
    }
    for (name, id) in &remotes {
        // origin/HEAD는 가리키는 브랜치에 이미 이름이 있다.
        if !name.ends_with(b"/HEAD") {
            label(*id, name, LabelKind::Remote);
        }
    }
    for (name, id) in &tags {
        label(*id, name, LabelKind::Tag);
    }
    if let (Some(head), None) = (head, &branch) {
        labels.entry(head).or_default().push(Label {
            name: "HEAD".to_owned(),
            kind: LabelKind::Head,
            current: true,
            worktree: None,
        });
    }
    // 다른 워크트리의 detached HEAD
    for (id, worktree) in &elsewhere.detached {
        labels.entry(*id).or_default().push(Label {
            name: "HEAD".to_owned(),
            kind: LabelKind::Head,
            current: false,
            worktree: Some(worktree.clone()),
        });
    }
    // 현재 브랜치(또는 HEAD)를 맨 앞에, 그다음 브랜치·원격·태그 순으로
    for list in labels.values_mut() {
        list.sort_by(|a, b| (!a.current, a.kind, &a.name).cmp(&(!b.current, b.kind, &b.name)));
    }
    Ok(Refs {
        labels,
        tips,
        head,
        branch,
        hidden_branches,
        hidden_remotes,
        revs,
        rewrites,
    })
}

/// 느슨한(packed가 아닌) 태그 참조가 annotated 태그면 끝까지 벗긴다.
fn peel_tag(repo: &gix::Repository, id: ObjectId) -> Option<ObjectId> {
    match repo.find_header(id).ok()?.kind() {
        gix::object::Kind::Tag => Some(repo.find_object(id).ok()?.peel_tags_to_end().ok()?.id),
        _ => Some(id),
    }
}

// ---------------------------------------------------------------- 세션

/// 화면에 보이는 행 하나
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRow {
    pub sha: String,
    pub parents: Vec<String>,
    pub summary: String,
    pub author: String,
    pub author_mail: String,
    /// Unix 시각(초)
    pub author_time: i64,
    /// UTC와의 차이(초)
    pub author_tz_offset: i32,
    pub labels: Vec<Label>,
    #[serde(flatten)]
    pub layout: RowLayout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphMeta {
    /// 작업트리 경로
    pub repo: String,
    pub head: Option<String>,
    /// HEAD가 가리키는 브랜치(detached면 없다)
    pub branch: Option<String>,
    /// 걷기 시작한 커밋 수
    pub tips: usize,
    pub scope: GraphScope,
    /// 시작점으로 쓰지 않은 로컬 브랜치 수(`head` 범위)
    pub hidden_branches: usize,
    /// 시작점으로 쓰지 않은 원격 브랜치 수
    pub hidden_remotes: usize,
    /// commit-graph 파일이 있다
    pub commit_graph: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphStats {
    /// DAG 읽기에 든 시간(ms)
    pub load_ms: f64,
    /// 순서 정하기에 든 시간
    pub sort_ms: f64,
    /// 열기를 요청받은 때부터 첫 화면을 보낼 때까지
    pub first_page_ms: f64,
    /// 열기를 요청받은 때부터 레이아웃을 다 정할 때까지
    pub total_ms: f64,
    pub rows: usize,
    /// 가장 넓은 행의 레인 수
    pub width: usize,
    /// commit-graph에서 읽은 커밋 수
    pub from_graph: usize,
    /// 앱 캐시의 커밋 표에서 읽은 커밋 수
    pub from_table: usize,
    /// 커밋 객체를 풀어 읽은 커밋 수
    pub from_objects: usize,
}

/// 화면으로 보내는 이벤트: `meta` → `loading`* → `page` → `progress`* → `done`·`error`·`cancelled` 하나
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GraphEvent {
    Meta(GraphMeta),
    /// DAG를 읽는 중(commit-graph가 없으면 커밋 객체를 하나씩 푼다)
    Loading {
        commits: usize,
    },
    /// 첫 화면: 처음 [`PAGE`]개(적으면 전부) 행과 전체 행 수
    Page {
        rows: Vec<GraphRow>,
        total: usize,
    },
    /// 레이아웃을 정한 행 수(이만큼 [`GraphSessions::rows`]로 읽을 수 있다)
    Progress {
        available: usize,
        width: usize,
    },
    Done {
        stats: GraphStats,
    },
    Error {
        message: String,
    },
    Cancelled,
}

impl GraphEvent {
    pub fn is_end(&self) -> bool {
        matches!(
            self,
            GraphEvent::Done { .. } | GraphEvent::Error { .. } | GraphEvent::Cancelled
        )
    }
}

/// 요청한 범위의 행들
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphWindow {
    pub start: usize,
    pub rows: Vec<GraphRow>,
    /// 전체 행 수(DAG를 다 읽기 전에는 0)
    pub total: usize,
    /// 레이아웃을 정한 행 수
    pub available: usize,
    pub width: usize,
    pub done: bool,
}

#[derive(Default)]
struct State {
    built: Option<Arc<Built>>,
    /// `checkpoints[k]`는 `k * CHECKPOINT`번 행 바로 앞의 레인 상태
    checkpoints: Vec<LaneLayout>,
    available: usize,
    width: usize,
    done: bool,
}

struct Session {
    repo: Arc<Repo>,
    /// 커밋 → 이름표. 태그 객체를 가리키는 시작점을 벗기면 커밋으로 옮긴다.
    labels: Mutex<HashMap<ObjectId, Vec<Label>>>,
    rewrites: Arc<Rewrites>,
    /// 시작점을 git log에 주는 인자
    revs: Vec<String>,
    state: Mutex<State>,
    cancelled: AtomicBool,
    /// 검색할 때마다 올린다. 진행 중인 검색은 값이 바뀌면 멈춘다.
    search_generation: AtomicU64,
    /// 진행 중인 경로 검색의 (세대 번호, git)
    search_git: Mutex<Option<(u64, Canceller)>>,
}

impl Session {
    fn built(&self) -> Result<Arc<Built>, String> {
        self.state
            .lock()
            .unwrap()
            .built
            .clone()
            .ok_or_else(|| "아직 그래프를 읽는 중이다".to_owned())
    }

    /// 새 검색을 시작한다: 진행 중인 검색(경로 검색의 git 포함)을 멈추고 이 검색의 세대 번호를 돌려준다.
    fn start_search(&self) -> u64 {
        // 세대 올리기와 경로 검색 git 등록은 같은 잠금 안에서 한다: 앞 검색이 새 검색의 git을 덮어쓰면
        // 그 git을 더는 멈출 수 없다.
        let mut current = self.search_git.lock().unwrap();
        if let Some((_, git)) = current.take() {
            git.cancel();
        }
        self.search_generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn search_stale(&self, generation: u64) -> bool {
        self.search_generation.load(Ordering::SeqCst) != generation
            || self.cancelled.load(Ordering::Relaxed)
    }

    /// `start`..`end` 행(레이아웃을 정한 데까지)
    fn window(&self, start: usize, end: usize) -> GraphWindow {
        let (built, checkpoint, available, width, done) = {
            let state = self.state.lock().unwrap();
            let end = end.min(state.available);
            let start = start.min(end);
            (
                state.built.clone(),
                state.checkpoints.get(start / CHECKPOINT).cloned(),
                state.available,
                state.width,
                state.done,
            )
        };
        let end = end.min(available);
        let start = start.min(end);
        let total = built.as_ref().map_or(0, |b| b.order.len());
        let rows = match (built, checkpoint) {
            (Some(built), Some(mut layout)) if start < end => {
                let first = start - start % CHECKPOINT;
                let layouts: Vec<RowLayout> = built.order[first..end]
                    .iter()
                    .map(|&node| layout.place(node, built.dag.parents(node)))
                    .skip(start - first)
                    .collect();
                self.rows(&built, &built.order[start..end], layouts)
            }
            _ => Vec::new(),
        };
        GraphWindow {
            start,
            rows,
            total,
            available,
            width,
            done,
        }
    }

    /// 노드들의 행 정보(요약·작성자는 커밋 객체에서 읽는다)
    fn rows(&self, built: &Built, nodes: &[u32], layouts: Vec<RowLayout>) -> Vec<GraphRow> {
        let mut repo = self.repo.local();
        repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
        let labels = self.labels.lock().unwrap();
        nodes
            .iter()
            .zip(layouts)
            .map(|(&node, layout)| {
                let id = built.dag.id(node);
                let mut row = GraphRow {
                    sha: id.to_string(),
                    parents: built
                        .dag
                        .parents(node)
                        .iter()
                        .map(|&p| built.dag.id(p).to_string())
                        .collect(),
                    summary: String::new(),
                    author: String::new(),
                    author_mail: String::new(),
                    author_time: 0,
                    author_tz_offset: 0,
                    labels: labels.get(&id).cloned().unwrap_or_default(),
                    layout,
                };
                if let Ok(commit) = repo.find_commit(built.rewrites.object(id))
                    && let Ok(decoded) = commit.decode()
                {
                    let encoding = decoded.encoding;
                    let message = decode_text(decoded.message, encoding);
                    row.summary = message.lines().next().unwrap_or("").trim().to_owned();
                    if let Ok(author) = decoded.author() {
                        let author = person(author, encoding);
                        row.author = author.name;
                        row.author_mail = author.email;
                        row.author_time = author.time;
                        row.author_tz_offset = author.tz_offset;
                    }
                }
                row
            })
            .collect()
    }
}

/// 저장소(git 공통 디렉터리)별 커밋 표. 처음 쓸 때 디스크 캐시에서 읽고 메모리에 둔다.
struct Tables {
    cache: Option<Arc<DiskCache>>,
    loaded: Mutex<HashMap<PathBuf, Arc<CommitTable>>>,
    /// 표에 없는 커밋을 이만큼 넘게 객체에서 읽으면 표를 새로 쓴다.
    min_new: usize,
}

impl Default for Tables {
    fn default() -> Self {
        Tables {
            cache: None,
            loaded: Mutex::default(),
            min_new: TABLE_MIN_NEW,
        }
    }
}

impl Tables {
    fn get(&self, git_dir: &Path) -> Option<Arc<CommitTable>> {
        if let Some(table) = self.loaded.lock().unwrap().get(git_dir) {
            return Some(table.clone());
        }
        let bytes = self
            .cache
            .as_ref()?
            .get_bytes(&CacheKey::commit_table(git_dir))?;
        let table = Arc::new(CommitTable::from_bytes(&bytes)?);
        self.loaded
            .lock()
            .unwrap()
            .insert(git_dir.to_owned(), table.clone());
        Some(table)
    }

    fn put(&self, git_dir: &Path, table: CommitTable) {
        if let Some(cache) = &self.cache {
            // 실패해도 다음에 다시 객체에서 읽을 뿐이다.
            let _ = cache.put_bytes(&CacheKey::commit_table(git_dir), &table.to_bytes());
        }
        self.loaded
            .lock()
            .unwrap()
            .insert(git_dir.to_owned(), Arc::new(table));
    }
}

/// 열어 둔 그래프들. 앱 상태로 하나 둔다. 화면이 닫으면([`GraphSessions::close`]) 메모리를 돌려준다.
#[derive(Default)]
pub struct GraphSessions {
    next_id: AtomicU64,
    sessions: Mutex<HashMap<u64, Arc<Session>>>,
    tables: Arc<Tables>,
}

impl GraphSessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// commit-graph가 없는 저장소의 커밋 표를 `cache`에 둔다.
    pub fn with_cache(cache: Arc<DiskCache>) -> Self {
        Self::with_cache_min(cache, TABLE_MIN_NEW)
    }

    /// 표에 없는 커밋을 `min_new`개 넘게 읽으면 표를 새로 쓴다(테스트·측정용).
    pub fn with_cache_min(cache: Arc<DiskCache>, min_new: usize) -> Self {
        GraphSessions {
            tables: Arc::new(Tables {
                cache: Some(cache),
                loaded: Mutex::default(),
                min_new,
            }),
            ..Self::default()
        }
    }

    /// `path`가 든 저장소의 그래프를 연다. 워크는 다른 스레드에서 돌고 이벤트는 `on_event`로 온다
    /// (`false`를 돌려주면 받는 쪽이 사라진 것으로 보고 멈춘다).
    pub fn open(
        &self,
        repos: &RepoCache,
        path: &Path,
        options: GraphOptions,
        on_event: impl FnMut(GraphEvent) -> bool + Send + 'static,
    ) -> Result<u64, String> {
        let started = Instant::now();
        let repo = repos.for_path(path).map_err(|e| e.to_string())?;
        let refs = read_refs(&repo, options)?;
        let meta = GraphMeta {
            repo: repo.workdir().to_string_lossy().into_owned(),
            head: refs.head.map(|id| id.to_string()),
            branch: refs.branch,
            tips: refs.tips.len(),
            scope: options.scope,
            hidden_branches: refs.hidden_branches,
            hidden_remotes: refs.hidden_remotes,
            commit_graph: false,
        };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let session = Arc::new(Session {
            repo,
            labels: Mutex::new(refs.labels),
            rewrites: Arc::new(refs.rewrites),
            revs: refs.revs,
            state: Mutex::new(State::default()),
            cancelled: AtomicBool::new(false),
            search_generation: AtomicU64::new(0),
            search_git: Mutex::new(None),
        });
        self.sessions.lock().unwrap().insert(id, session.clone());
        let tips = refs.tips;
        let tables = self.tables.clone();
        std::thread::spawn(move || walk(session, &tables, tips, meta, started, on_event));
        Ok(id)
    }

    fn session(&self, id: u64) -> Result<Arc<Session>, String> {
        self.sessions
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| "닫힌 그래프다".to_owned())
    }

    /// `start`..`end` 행. 레이아웃을 아직 정하지 않은 행은 빠진다(`available`을 본다).
    pub fn rows(&self, id: u64, start: usize, end: usize) -> Result<GraphWindow, String> {
        Ok(self.session(id)?.window(start, end))
    }

    /// 리비전(`HEAD~2`, sha 일부, 브랜치 …)이 놓인 행. 그래프에 없으면 `None`.
    pub fn find(&self, id: u64, rev: &str) -> Result<Option<usize>, String> {
        let session = self.session(id)?;
        let commit = session
            .repo
            .resolve_commit(rev)
            .map_err(|e| e.to_string())?;
        let built = session.state.lock().unwrap().built.clone();
        Ok(built.and_then(|built| built.row_of_id(commit)))
    }

    /// 메시지(모든 줄)·작성자 이름·메일에 `query`가 들어 있거나(대소문자 무시) sha가 `query`로 시작하는 커밋의 행.
    /// `from` 다음 행부터(`forward`가 아니면 앞 행부터) 차례로 커밋을 풀어 보고, 끝에 닿으면 반대쪽 끝에서 이어
    /// 찾는다. 없으면 `None`. 같은 그래프에서 새 검색을 시작하면 이 검색은 멈추고 오류(취소)를 돌려준다.
    pub fn search(
        &self,
        id: u64,
        query: &str,
        from: usize,
        forward: bool,
    ) -> Result<Option<usize>, String> {
        let session = self.session(id)?;
        let generation = session.start_search();
        let built = session.built()?;
        let query = query.trim().to_lowercase();
        let n = built.order.len();
        if query.is_empty() || n == 0 {
            return Ok(None);
        }
        let hex = query.len() >= 4 && query.bytes().all(|b| b.is_ascii_hexdigit());
        let mut repo = session.repo.local();
        repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
        let mut buf = Vec::new();
        let from = from.min(n - 1);
        for step in 1..=n {
            if step % 256 == 0 && session.search_stale(generation) {
                return Err(CANCELLED.to_owned());
            }
            let row = if forward {
                (from + step) % n
            } else {
                (from + n - step % n) % n
            };
            let id = built.dag.id(built.order[row]);
            if (hex && id.to_hex().to_string().starts_with(&query))
                || commit_matches(&repo, built.rewrites.object(id), &query, &mut buf)
            {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    /// `path`(파일·디렉터리, git pathspec)를 바꾼 커밋의 행들(오름차순). 그래프와 같은 시작점으로
    /// `git log --full-history --no-merges -- <path>`를 돌린다(Bloom 필터가 있으면 빠르다). merge는 넣지 않는다:
    /// 양쪽 가지가 바꾼 경로면 merge마다 걸려 정작 바꾼 커밋이 묻힌다. 새 검색을 시작하면 git을 끝내고 취소한다.
    pub fn search_path(&self, id: u64, path: &str) -> Result<Vec<usize>, String> {
        let session = self.session(id)?;
        let generation = session.start_search();
        let built = session.built()?;
        if built.order.is_empty() || session.revs.is_empty() {
            return Ok(Vec::new());
        }
        let mut command = git::command(session.repo.workdir());
        command
            .args([
                "log",
                "--format=%H",
                "--full-history",
                "--no-merges",
                "--no-color",
            ])
            .args(&session.revs)
            .arg("--")
            .arg(path);
        let mut git = Streaming::spawn(command, "git log").map_err(|e| e.to_string())?;
        {
            let mut current = session.search_git.lock().unwrap();
            if session.search_stale(generation) {
                drop(current);
                git.abort();
                return Err(CANCELLED.to_owned());
            }
            *current = Some((generation, git.canceller()));
        }
        let mut rows = Vec::new();
        let mut pending: Vec<u8> = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = git.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            pending.extend_from_slice(&buf[..n]);
            let complete = pending
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(0, |i| i + 1);
            for line in pending[..complete].split(|&b| b == b'\n') {
                if let Ok(id) = ObjectId::from_hex(line.trim_ascii())
                    && let Some(row) = built.row_of_id(id)
                {
                    rows.push(row);
                }
            }
            pending.drain(..complete);
        }
        git.finish().map_err(|e| e.to_string())?;
        let mut current = session.search_git.lock().unwrap();
        if current.as_ref().is_some_and(|(g, _)| *g == generation) {
            current.take();
        }
        drop(current);
        rows.sort_unstable();
        rows.dedup();
        Ok(rows)
    }

    /// 그래프를 닫는다(워크와 검색을 멈추고 행을 버린다). 모르는 id면 `false`.
    pub fn close(&self, id: u64) -> bool {
        match self.sessions.lock().unwrap().remove(&id) {
            Some(session) => {
                session.cancelled.store(true, Ordering::SeqCst);
                session.start_search();
                true
            }
            None => false,
        }
    }

    pub fn open_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

fn walk(
    session: Arc<Session>,
    tables: &Tables,
    tips: Vec<ObjectId>,
    meta: GraphMeta,
    started: Instant,
    mut on_event: impl FnMut(GraphEvent) -> bool,
) {
    let end = walk_inner(&session, tables, &tips, meta, started, &mut on_event);
    session.state.lock().unwrap().done = true;
    on_event(end);
}

fn walk_inner(
    session: &Session,
    tables: &Tables,
    tips: &[ObjectId],
    mut meta: GraphMeta,
    started: Instant,
    on_event: &mut impl FnMut(GraphEvent) -> bool,
) -> GraphEvent {
    let mut repo = session.repo.local();
    repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
    // git처럼(commit_graph_compatible) 얕은 클론이거나 부모를 바꾼 커밋(`git replace`, graft)이 있으면 commit-graph와
    // 커밋 표의 부모는 git이 보는 부모가 아니다. 객체에서 읽는다(얕은 경계도 객체 쪽에서만 지킨다).
    let compatible = session.rewrites.is_empty() && !repo.is_shallow().unwrap_or(true);
    let graph = compatible
        .then(|| repo.commit_graph_if_enabled().ok().flatten())
        .flatten();
    meta.commit_graph = graph.is_some();
    if !on_event(GraphEvent::Meta(meta)) {
        return GraphEvent::Cancelled;
    }

    let git_dir =
        dunce::canonicalize(repo.common_dir()).unwrap_or_else(|_| repo.common_dir().to_owned());
    let table = if graph.is_none() && compatible {
        tables.get(&git_dir)
    } else {
        None
    };
    let (built, load, new_table, peeled) = match build(
        &repo,
        graph,
        table,
        tables.min_new,
        tips,
        &session.rewrites,
        &session.cancelled,
        &mut |commits| on_event(GraphEvent::Loading { commits }),
    ) {
        Ok(Some(built)) => built,
        Ok(None) => return GraphEvent::Cancelled,
        Err(message) => return GraphEvent::Error { message },
    };
    // 태그 객체를 가리키던 시작점의 이름표는 벗긴 커밋에 단다.
    if !peeled.is_empty() {
        let mut labels = session.labels.lock().unwrap();
        for (tag, commit) in peeled {
            if let Some(moved) = labels.remove(&tag) {
                let list = labels.entry(commit).or_default();
                list.extend(moved);
                list.sort_by(|a, b| {
                    (!a.current, a.kind, &a.name).cmp(&(!b.current, b.kind, &b.name))
                });
            }
        }
    }
    let mut stats = GraphStats {
        load_ms: load.load_ms,
        sort_ms: load.sort_ms,
        from_graph: load.from_graph,
        from_table: load.from_table,
        from_objects: load.from_objects,
        rows: built.order.len(),
        ..GraphStats::default()
    };
    let built = Arc::new(built);
    session.state.lock().unwrap().built = Some(built.clone());

    // 레이아웃: 행마다 레인을 정하고 CHECKPOINT행마다 상태를 남긴다.
    let total = built.order.len();
    let page = PAGE.min(total);
    let mut layout = LaneLayout::new();
    let mut checkpoints = Vec::new();
    let mut width = 0usize;
    let mut last_progress = Instant::now();
    let flush = |checkpoints: &mut Vec<LaneLayout>, available: usize, width: usize| {
        let mut state = session.state.lock().unwrap();
        state.checkpoints.append(checkpoints);
        state.available = available;
        state.width = width;
    };
    let mut page_sent = false;
    for (row, &node) in built.order.iter().enumerate() {
        if row % CHECKPOINT == 0 {
            if session.cancelled.load(Ordering::Relaxed) {
                return GraphEvent::Cancelled;
            }
            checkpoints.push(layout.clone());
        }
        let before = layout.lanes();
        let placed = layout.place(node, built.dag.parents(node));
        width = width
            .max(before)
            .max(layout.lanes())
            .max(usize::from(placed.lane) + 1);
        let available = row + 1;
        if available == page {
            flush(&mut checkpoints, available, width);
            page_sent = true;
            let rows = session.window(0, page).rows;
            if !on_event(GraphEvent::Page { rows, total }) {
                return GraphEvent::Cancelled;
            }
            stats.first_page_ms = ms(started);
            last_progress = Instant::now();
        } else if page_sent && last_progress.elapsed() >= PROGRESS_INTERVAL {
            flush(&mut checkpoints, available, width);
            if !on_event(GraphEvent::Progress { available, width }) {
                return GraphEvent::Cancelled;
            }
            last_progress = Instant::now();
        }
    }
    flush(&mut checkpoints, total, width);
    if !page_sent {
        // 커밋이 없는 저장소
        if !on_event(GraphEvent::Page {
            rows: Vec::new(),
            total,
        }) {
            return GraphEvent::Cancelled;
        }
        stats.first_page_ms = ms(started);
    }
    stats.width = width;
    stats.total_ms = ms(started);
    if let Some(table) = new_table {
        tables.put(&git_dir, table);
    }
    GraphEvent::Done { stats }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 그래프(노드 → 부모들)를 `order` 순으로 놓은 레이아웃
    fn lay(order: &[(u32, &[u32])]) -> Vec<RowLayout> {
        let mut layout = LaneLayout::new();
        order
            .iter()
            .map(|(node, parents)| layout.place(*node, parents))
            .collect()
    }

    /// 모든 부모 선이 행을 따라 끊기지 않고 부모 점까지 이어지는지, 행 사이 가장자리의 선이 맞물리는지 본다.
    fn check_edges(order: &[(u32, &[u32])], rows: &[RowLayout]) {
        let row_of: HashMap<u32, usize> = order
            .iter()
            .enumerate()
            .map(|(i, (node, _))| (*node, i))
            .collect();
        for (r, (_, parents)) in order.iter().enumerate() {
            assert_eq!(rows[r].down.len(), parents.len(), "{r}: 부모마다 선 하나");
            for (k, parent) in parents.iter().enumerate() {
                let target = row_of[parent];
                assert!(target > r, "부모는 자식보다 뒤에 온다");
                let mut lane = rows[r].down[k][0];
                for (i, next) in rows.iter().enumerate().take(target).skip(r + 1) {
                    let line = next
                        .through
                        .iter()
                        .find(|t| t[0] == lane)
                        .unwrap_or_else(|| {
                            panic!("{r}→{target}: {i}행에서 레인 {lane}이 끊긴다: {next:?}")
                        });
                    assert_ne!(next.lane, lane, "{r}→{target}: {i}행의 커밋 점을 지난다");
                    lane = line[1];
                }
                assert!(
                    rows[target].up.iter().any(|u| u[0] == lane),
                    "{r}→{target}: 부모 행으로 들어오는 선이 없다: {:?}",
                    rows[target]
                );
            }
        }
        for (r, row) in rows.iter().enumerate() {
            // 위 가장자리에서는 레인마다 선이 하나다.
            let top: Vec<u16> = row
                .through
                .iter()
                .map(|t| t[0])
                .chain(row.up.iter().map(|u| u[0]))
                .collect();
            let unique: HashSet<u16> = top.iter().copied().collect();
            assert_eq!(unique.len(), top.len(), "{r}: {row:?}");
            // 아래 가장자리의 선은 다음 행의 위 가장자리로 이어진다.
            if let Some(next) = rows.get(r + 1) {
                let bottom: HashSet<u16> = row
                    .through
                    .iter()
                    .map(|t| t[1])
                    .chain(row.down.iter().map(|d| d[0]))
                    .collect();
                assert_eq!(
                    bottom,
                    unique_top(next),
                    "{r}→{}: {row:?} / {next:?}",
                    r + 1
                );
            }
        }
    }

    fn unique_top(row: &RowLayout) -> HashSet<u16> {
        row.through
            .iter()
            .map(|t| t[0])
            .chain(row.up.iter().map(|u| u[0]))
            .collect()
    }

    fn order_of<'a>(parents: &'a [&'a [u32]], tips: &[(u32, i64)]) -> Vec<(u32, &'a [u32])> {
        let dag = Dag::from_parents(parents);
        topo_order(&dag, tips)
            .into_iter()
            .map(|n| (n, parents[n as usize]))
            .collect()
    }

    fn width(order: &[(u32, &[u32])]) -> usize {
        let mut layout = LaneLayout::new();
        order
            .iter()
            .map(|(node, parents)| {
                let before = layout.lanes();
                let row = layout.place(*node, parents);
                before.max(layout.lanes()).max(usize::from(row.lane) + 1)
            })
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let order: &[(u32, &[u32])] = &[(3, &[2]), (2, &[1]), (1, &[])];
        let rows = lay(order);
        assert!(rows.iter().all(|r| r.lane == 0));
        assert!(rows[0].up.is_empty(), "가지 끝 위로는 선이 없다");
        assert!(rows[2].down.is_empty(), "루트 아래로는 선이 없다");
        check_edges(order, &rows);
    }

    #[test]
    fn branch_and_merge() {
        // 4 = merge(3, 2), 3 → 1, 2 → 1. git 순서: 두 번째 부모(2) 쪽이 먼저 나온다.
        let order: &[(u32, &[u32])] = &[(4, &[3, 2]), (2, &[1]), (3, &[1]), (1, &[])];
        let rows = lay(order);
        assert_eq!(
            rows.iter().map(|r| r.lane).collect::<Vec<_>>(),
            [0, 1, 0, 0]
        );
        assert_eq!(rows[0].down.len(), 2);
        // 3의 행에서 1을 기다리던 오른쪽 레인을 3의 레인으로 합친다.
        assert_eq!(rows[2].through, [[1, 0, u16::from(rows[1].color)]]);
        assert_eq!(rows[3].up.len(), 1);
        check_edges(order, &rows);
    }

    #[test]
    fn octopus_roots_and_lane_reuse() {
        // 9 = octopus(8, 7, 6), 8·7·6 → 5, 5는 루트, 10은 따로 시작한 루트
        let order: &[(u32, &[u32])] = &[
            (9, &[8, 7, 6]),
            (6, &[5]),
            (7, &[5]),
            (8, &[5]),
            (5, &[]),
            (10, &[]),
        ];
        let rows = lay(order);
        check_edges(order, &rows);
        assert_eq!(rows[4].lane, 0);
        // 세 가지는 5에 닿기 전에 왼쪽 레인으로 차례로 합쳐진다.
        assert_eq!(rows[4].up.len(), 1);
        assert_eq!(width(order), 3);
        assert_eq!(rows[5].lane, 0, "빈 레인을 다시 쓴다");
    }

    #[test]
    fn criss_cross_merges_and_parallel_branches() {
        // 두 브랜치가 서로를 merge한 이력
        let parents: &[&[u32]] = &[
            &[],     // 0
            &[0],    // 1
            &[1],    // 2
            &[1],    // 3
            &[2, 3], // 4
            &[3, 2], // 5
            &[4, 5], // 6
        ];
        let order = order_of(parents, &[(6, 0)]);
        check_edges(&order, &lay(&order));
        assert!(width(&order) <= 3, "너비 {}", width(&order));
    }

    #[test]
    fn several_tips() {
        // 두 가지 끝(5, 4)이 각자 3과 2를 거쳐 1로
        let order: &[(u32, &[u32])] = &[(5, &[3]), (3, &[1]), (4, &[2]), (2, &[1]), (1, &[])];
        let rows = lay(order);
        assert_eq!((rows[0].lane, rows[2].lane), (0, 1));
        check_edges(order, &rows);
    }

    #[test]
    fn topo_order_follows_git_rules() {
        // 0 ← 1 ← 2 ← 4(merge 2, 3), 1 ← 3 ← 5(다른 가지 끝)
        let parents: &[&[u32]] = &[&[], &[0], &[1], &[1], &[2, 3], &[3]];
        let dag = Dag::from_parents(parents);
        // merge 다음에는 두 번째 부모 쪽(3)이 먼저지만, 3은 자식 5가 남아 기다린다.
        assert_eq!(topo_order(&dag, &[(4, 20), (5, 10)]), [4, 2, 5, 3, 1, 0]);
        // 시작점은 커밋 시각이 늦은 것부터
        assert_eq!(topo_order(&dag, &[(4, 10), (5, 20)]), [5, 4, 3, 2, 1, 0]);
        // 시각이 같으면 준 순서
        assert_eq!(topo_order(&dag, &[(5, 10), (4, 10)]), [5, 4, 3, 2, 1, 0]);
        assert_eq!(topo_order(&dag, &[(4, 10), (5, 10)]), [4, 2, 5, 3, 1, 0]);
        // 다른 시작점에서 닿는 시작점은 시각이 늦어도 자식이 다 나온 뒤에
        assert_eq!(
            topo_order(&dag, &[(2, 30), (4, 20), (5, 10)]),
            [4, 2, 5, 3, 1, 0]
        );
    }

    #[test]
    fn random_dags_lay_out_without_breaks() {
        // 작은 의사 난수 DAG 여러 개: 노드 i의 부모는 i보다 작은 번호에서 고른다.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        for _ in 0..200 {
            let n = 2 + next(40) as u32;
            let lists: Vec<Vec<u32>> = (0..n)
                .map(|i| {
                    if i == 0 || next(10) == 0 {
                        return Vec::new();
                    }
                    let count = match next(10) {
                        0 => 3,
                        1 | 2 => 2,
                        _ => 1,
                    };
                    let mut parents: Vec<u32> = Vec::new();
                    for _ in 0..count {
                        let p = next(u64::from(i)) as u32;
                        if !parents.contains(&p) {
                            parents.push(p);
                        }
                    }
                    parents
                })
                .collect();
            let refs: Vec<&[u32]> = lists.iter().map(Vec::as_slice).collect();
            // 자식이 없는 노드가 모두 시작점이다.
            let mut has_child = vec![false; n as usize];
            for list in &lists {
                for &p in list {
                    has_child[p as usize] = true;
                }
            }
            let tips: Vec<(u32, i64)> = (0..n)
                .rev()
                .filter(|&i| !has_child[i as usize])
                .map(|i| (i, i64::from(i)))
                .collect();
            let order = order_of(&refs, &tips);
            assert_eq!(order.len(), n as usize, "모든 노드가 나온다");
            check_edges(&order, &lay(&order));
        }
    }
}

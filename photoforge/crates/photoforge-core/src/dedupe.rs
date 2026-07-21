//! Duplicate detection: exact (BLAKE3) and near (dHash + BK-tree).
//!
//! # Exact duplicates
//! Two files with the same BLAKE3 hash are byte-identical (collision odds are
//! astronomically small), so a simple `GROUP BY blake3 HAVING COUNT(*) > 1`
//! finds them. Groups are sorted by *wasted bytes* — the space you'd reclaim by
//! keeping one copy — so the biggest wins surface first.
//!
//! # Near duplicates
//! Perceptually-similar files (recompressed, resized, lightly edited) share a
//! dHash within a small Hamming distance `k`. Finding all pairs within `k` is
//! naively O(n²); a **BK-tree** prunes that dramatically.
//!
//! A BK-tree is a metric-space tree: each node stores a hash, and each child
//! edge is labeled with the distance between child and parent. When querying
//! for everything within distance `k` of `q`, at a node at distance `d` from
//! `q` we only need to descend into children whose edge label lies in
//! `[d - k, d + k]` — the **triangle inequality** guarantees nothing outside
//! that band can be within `k` of `q`. Whiteboard example: query at distance
//! d = 10 from a node, k = 2. Any match m has dist(m, q) ≤ 2, and the triangle
//! inequality forces dist(node, m) ≥ dist(node, q) − dist(q, m) ≥ 8 and
//! ≤ 10 + 2 = 12, so only edges labeled 8..=12 need visiting — the other ~52
//! possible edge labels are pruned wholesale, subtrees and all.
//!
//! Matching pairs (exact or near) are merged into groups with a **union-find**
//! (disjoint-set) structure, so A~B and B~C land A, B, C in one group even if
//! A and C are farther than `k` apart.
//!
//! ## Measured reality check (criterion, `benches/neardup.rs`, k = 5)
//! On this metric the BK-tree *loses* to the naive scan: 1.0 s naive vs 6.6 s
//! BK-tree at n = 50k synthetic hashes (40 ms vs 330 ms at 10k). Unrelated
//! 64-bit hashes sit at Hamming distance ~32 ± 4, so the pruning band
//! `[d-k, d+k]` removes little of the tree, while the naive loop is a branch-
//! free XOR+POPCNT sweep over a contiguous `Vec<u64>` (~1.3 G pairs/s).
//! [`NearMethod::Naive`] is therefore the default; the tree stays for the
//! benchmark comparison and for workloads where it could win (much larger n
//! with tightly clustered hashes, or k ≤ 2).

use crate::dhash::hamming;
use crate::Result;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashMap;

/// One file inside a duplicate group.
#[derive(Debug, Clone, Serialize)]
pub struct DupeFile {
    pub file_id: i64,
    pub path: String,
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

/// A group of (near-)identical files.
#[derive(Debug, Clone, Serialize)]
pub struct DupeGroup {
    /// Files in the group, largest first (the natural "keep" candidate).
    pub files: Vec<DupeFile>,
    /// Bytes reclaimable by keeping only the largest file.
    pub wasted_bytes: i64,
    /// For near groups: the max pairwise Hamming distance actually observed is
    /// not tracked; this records the threshold used. 0 for exact groups.
    pub threshold: u32,
}

fn finish_groups(mut groups: Vec<Vec<DupeFile>>, threshold: u32) -> Vec<DupeGroup> {
    let mut out: Vec<DupeGroup> = groups
        .drain(..)
        .filter(|g| g.len() > 1)
        .map(|mut files| {
            files.sort_by_key(|f| std::cmp::Reverse(f.size));
            let wasted: i64 = files.iter().skip(1).map(|f| f.size).sum();
            DupeGroup {
                files,
                wasted_bytes: wasted,
                threshold,
            }
        })
        .collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.wasted_bytes));
    out
}

/// Exact duplicates: groups of byte-identical files (same BLAKE3), sorted by
/// wasted bytes descending.
pub fn find_exact_duplicates(conn: &Connection) -> Result<Vec<DupeGroup>> {
    let mut stmt = conn.prepare(
        "SELECT h.blake3, f.id, f.path, f.size, f.width, f.height
         FROM hashes h JOIN files f ON f.id = h.file_id
         WHERE h.blake3 IS NOT NULL
           AND h.blake3 IN (SELECT blake3 FROM hashes WHERE blake3 IS NOT NULL
                            GROUP BY blake3 HAVING COUNT(*) > 1)
         ORDER BY h.blake3",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, Vec<u8>>(0)?,
            DupeFile {
                file_id: r.get(1)?,
                path: r.get(2)?,
                size: r.get(3)?,
                width: r.get(4)?,
                height: r.get(5)?,
            },
        ))
    })?;

    let mut by_hash: HashMap<Vec<u8>, Vec<DupeFile>> = HashMap::new();
    for row in rows {
        let (hash, file) = row?;
        by_hash.entry(hash).or_default().push(file);
    }
    Ok(finish_groups(by_hash.into_values().collect(), 0))
}

// ---------------------------------------------------------------------------
// Union-find (disjoint sets)
// ---------------------------------------------------------------------------

/// Classic union-find with path halving + union by size. Near-linear in
/// practice (inverse-Ackermann amortized).
pub struct UnionFind {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl UnionFind {
    pub fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            size: vec![1; n],
        }
    }

    pub fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            // Path halving: point every other node at its grandparent.
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let (mut ra, mut rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        if self.size[ra] < self.size[rb] {
            std::mem::swap(&mut ra, &mut rb);
        }
        self.parent[rb] = ra;
        self.size[ra] += self.size[rb];
    }
}

// ---------------------------------------------------------------------------
// BK-tree over Hamming distance
// ---------------------------------------------------------------------------

/// BK-tree node. Children are keyed by their Hamming distance to this node
/// (0..=64 for 64-bit hashes), stored as a fixed array of optional indices
/// into the arena to avoid per-node HashMaps.
struct BkNode {
    hash: u64,
    /// Original index of this hash in the input slice (to report matches).
    item: usize,
    /// children[d] = arena index of the child at distance d.
    children: [Option<u32>; 65],
}

/// A BK-tree specialized to 64-bit hashes under Hamming distance.
///
/// Nodes live in a flat `Vec` arena and refer to each other by index — the
/// idiomatic Rust replacement for Java-style parent/child object references,
/// which the ownership rules make painful for trees with sharing.
pub struct BkTree {
    arena: Vec<BkNode>,
}

impl Default for BkTree {
    fn default() -> Self {
        Self::new()
    }
}

impl BkTree {
    pub fn new() -> Self {
        Self { arena: Vec::new() }
    }

    /// Insert `hash` (tagged with caller-side index `item`).
    pub fn insert(&mut self, hash: u64, item: usize) {
        let new_idx = self.arena.len() as u32;
        if self.arena.is_empty() {
            self.arena.push(BkNode {
                hash,
                item,
                children: [None; 65],
            });
            return;
        }
        let mut cur = 0usize;
        loop {
            let d = hamming(hash, self.arena[cur].hash) as usize;
            match self.arena[cur].children[d] {
                Some(child) => cur = child as usize,
                None => {
                    self.arena[cur].children[d] = Some(new_idx);
                    self.arena.push(BkNode {
                        hash,
                        item,
                        children: [None; 65],
                    });
                    return;
                }
            }
        }
    }

    /// Find every stored item within Hamming distance `k` of `query`,
    /// returning `(item, distance)` pairs (including exact self-matches).
    pub fn range(&self, query: u64, k: u32) -> Vec<(usize, u32)> {
        let mut out = Vec::new();
        if self.arena.is_empty() {
            return out;
        }
        let mut stack = vec![0usize];
        while let Some(idx) = stack.pop() {
            let node = &self.arena[idx];
            let d = hamming(query, node.hash);
            if d <= k {
                out.push((node.item, d));
            }
            // Triangle inequality: only children with edge label in
            // [d - k, d + k] can contain matches.
            let lo = d.saturating_sub(k) as usize;
            let hi = (d + k).min(64) as usize;
            for label in lo..=hi {
                if let Some(child) = node.children[label] {
                    stack.push(child as usize);
                }
            }
        }
        out
    }
}

/// Which near-duplicate algorithm to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NearMethod {
    /// All-pairs O(n²) baseline.
    Naive,
    /// BK-tree range queries.
    BkTree,
}

/// Group `hashes` (parallel to caller indices) by Hamming distance ≤ `k`.
/// Returns groups of input indices; singletons are omitted.
pub fn group_near(hashes: &[u64], k: u32, method: NearMethod) -> Vec<Vec<usize>> {
    let n = hashes.len();
    let mut uf = UnionFind::new(n);
    match method {
        NearMethod::Naive => {
            for i in 0..n {
                for j in (i + 1)..n {
                    if hamming(hashes[i], hashes[j]) <= k {
                        uf.union(i, j);
                    }
                }
            }
        }
        NearMethod::BkTree => {
            // Identical hashes are grouped up front and inserted only once:
            // duplicates would otherwise chain off the distance-0 edge forever
            // (real photo libraries have many byte-identical dHashes), and
            // querying once per unique hash instead of once per file cuts the
            // expensive tree walks.
            let mut first_of: HashMap<u64, usize> = HashMap::new();
            let mut tree = BkTree::new();
            for (i, &h) in hashes.iter().enumerate() {
                match first_of.entry(h) {
                    std::collections::hash_map::Entry::Occupied(e) => uf.union(i, *e.get()),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(i);
                        tree.insert(h, i);
                    }
                }
            }
            for (&h, &i) in &first_of {
                for (j, _) in tree.range(h, k) {
                    if j != i {
                        uf.union(i, j);
                    }
                }
            }
        }
    }

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = uf.find(i);
        groups.entry(root).or_default().push(i);
    }
    groups.into_values().filter(|g| g.len() > 1).collect()
}

/// Near-duplicate groups from the catalog: files whose dHashes are within
/// Hamming distance `k`, grouped transitively, sorted by wasted bytes.
pub fn find_near_duplicates(conn: &Connection, k: u32, method: NearMethod) -> Result<Vec<DupeGroup>> {
    let mut stmt = conn.prepare(
        "SELECT h.dhash, f.id, f.path, f.size, f.width, f.height
         FROM hashes h JOIN files f ON f.id = h.file_id
         WHERE h.dhash IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)? as u64,
            DupeFile {
                file_id: r.get(1)?,
                path: r.get(2)?,
                size: r.get(3)?,
                width: r.get(4)?,
                height: r.get(5)?,
            },
        ))
    })?;

    let mut hashes = Vec::new();
    let mut files = Vec::new();
    for row in rows {
        let (h, f) = row?;
        hashes.push(h);
        files.push(f);
    }

    let groups = group_near(&hashes, k, method)
        .into_iter()
        .map(|idxs| idxs.into_iter().map(|i| files[i].clone()).collect())
        .collect();
    Ok(finish_groups(groups, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_find_groups_transitively() {
        let mut uf = UnionFind::new(4);
        uf.union(0, 1);
        uf.union(1, 2);
        assert_eq!(uf.find(0), uf.find(2));
        assert_ne!(uf.find(0), uf.find(3));
    }

    #[test]
    fn bktree_range_matches_naive() {
        // Deterministic xorshift-ish hash set with planted near-pairs.
        let mut hashes = Vec::new();
        let mut x = 0x9e3779b97f4a7c15u64;
        for i in 0..500u64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            hashes.push(x);
            if i % 10 == 0 {
                hashes.push(x ^ (1 << (i % 64))); // distance-1 neighbor
            }
        }
        for k in [0u32, 3, 5] {
            let mut naive = group_near(&hashes, k, NearMethod::Naive);
            let mut bk = group_near(&hashes, k, NearMethod::BkTree);
            for g in naive.iter_mut().chain(bk.iter_mut()) {
                g.sort_unstable();
            }
            naive.sort();
            bk.sort();
            assert_eq!(naive, bk, "mismatch at k={k}");
        }
    }

    #[test]
    fn exact_and_near_queries_over_catalog() {
        let conn = crate::db::open_in_memory().unwrap();
        // Three files: a+b byte-identical, c unrelated content but dhash within 2 of a.
        conn.execute_batch(
            "INSERT INTO files (id, path, size, mtime) VALUES
               (1, 'a.jpg', 100, 0), (2, 'b.jpg', 100, 0), (3, 'c.jpg', 50, 0);
             INSERT INTO hashes (file_id, blake3, dhash) VALUES
               (1, x'aa', 255), (2, x'aa', 255), (3, x'bb', 254);",
        )
        .unwrap();

        let exact = find_exact_duplicates(&conn).unwrap();
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].files.len(), 2);
        assert_eq!(exact[0].wasted_bytes, 100);

        let near = find_near_duplicates(&conn, 2, NearMethod::BkTree).unwrap();
        assert_eq!(near.len(), 1);
        assert_eq!(near[0].files.len(), 3, "transitive grouping pulls all three");
        // Largest first.
        assert_eq!(near[0].files[0].size, 100);
    }
}

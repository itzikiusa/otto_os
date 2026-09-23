//! Render-graph checks over `embeds` / `uses_component` / `uses_tokens` edges:
//! cycle rejection (A embeds B embeds A) and the render-depth cap. Pure over an
//! adjacency snapshot the store loads (bounded BFS); unit-tested.

use std::collections::{HashMap, HashSet, VecDeque};

/// `artifact id → render targets`.
pub type Adjacency = HashMap<String, Vec<String>>;

/// Can `to` be reached from `from` along render edges? Visits at most
/// `max_nodes` nodes (a pathological graph answers "no" rather than hang —
/// the store loads the same bounded subgraph anyway).
pub fn reaches(adj: &Adjacency, from: &str, to: &str, max_nodes: usize) -> bool {
    if from == to {
        return true;
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    queue.push_back(from);
    seen.insert(from);
    while let Some(n) = queue.pop_front() {
        if seen.len() > max_nodes {
            return false;
        }
        for next in adj.get(n).map(Vec::as_slice).unwrap_or(&[]) {
            if next == to {
                return true;
            }
            if seen.insert(next.as_str()) {
                queue.push_back(next.as_str());
            }
        }
    }
    false
}

/// Longest chain of render edges starting at `node`, saturating at `cap + 1`
/// (callers only care whether it exceeds `cap`). Cycles never recurse (a node
/// already on the current path contributes 0).
pub fn depth_below(adj: &Adjacency, node: &str, cap: usize) -> usize {
    let mut memo: HashMap<String, usize> = HashMap::new();
    let mut path: Vec<String> = Vec::new();
    longest(adj, node, cap + 1, &mut memo, &mut path)
}

fn longest(
    adj: &Adjacency,
    node: &str,
    limit: usize,
    memo: &mut HashMap<String, usize>,
    path: &mut Vec<String>,
) -> usize {
    if let Some(d) = memo.get(node) {
        return *d;
    }
    if path.len() >= limit || path.iter().any(|p| p == node) {
        return 0;
    }
    path.push(node.to_string());
    let mut best = 0usize;
    for next in adj.get(node).map(Vec::as_slice).unwrap_or(&[]) {
        let d = 1 + longest(adj, next, limit, memo, path);
        best = best.max(d.min(limit));
        if best >= limit {
            break;
        }
    }
    path.pop();
    memo.insert(node.to_string(), best);
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adj(edges: &[(&str, &str)]) -> Adjacency {
        let mut a = Adjacency::new();
        for (s, d) in edges {
            a.entry(s.to_string()).or_default().push(d.to_string());
        }
        a
    }

    #[test]
    fn reaches_finds_paths_and_detects_would_be_cycles() {
        let g = adj(&[("b", "c"), ("c", "a"), ("x", "y")]);
        // Adding a→b would close a→b→c→a.
        assert!(reaches(&g, "b", "a", 1_000));
        assert!(!reaches(&g, "x", "a", 1_000));
        assert!(reaches(&g, "a", "a", 1_000), "self-link is a cycle");
    }

    #[test]
    fn depth_below_measures_the_longest_chain_and_saturates() {
        let g = adj(&[("a", "b"), ("b", "c"), ("a", "c"), ("c", "d")]);
        assert_eq!(depth_below(&g, "a", 4), 3);
        assert_eq!(depth_below(&g, "d", 4), 0);
        let long = adj(&[
            ("n0", "n1"),
            ("n1", "n2"),
            ("n2", "n3"),
            ("n3", "n4"),
            ("n4", "n5"),
            ("n5", "n6"),
        ]);
        assert_eq!(depth_below(&long, "n0", 4), 5, "saturates at cap + 1");
        // A cycle in stored data never loops forever.
        let cyc = adj(&[("a", "b"), ("b", "a")]);
        assert!(depth_below(&cyc, "a", 4) <= 5);
    }
}

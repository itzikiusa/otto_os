//! Bounds for the local (neighbourhood) graph. A depth-3 walk from a hub
//! note in a 10k-note vault reaches most of the vault; the UI cannot draw
//! that legibly and the payload is built in memory per request. The walk is
//! breadth-first and admits at most [`MAX_NODES`] notes — the nearest win —
//! and at most [`MAX_EDGES`] links among them, then reports `truncated`.

use std::collections::HashMap;
use std::hash::Hash;

/// Notes a local graph may hold (the focus included).
pub(crate) const MAX_NODES: usize = 300;
/// Links a local graph may hold.
pub(crate) const MAX_EDGES: usize = 1_500;

/// Admit up to `room` of one BFS layer's new nodes. `ties` maps each
/// candidate to how many links tie it to the walked set: the best-connected
/// win, then the lowest key (deterministic, so one generation's cache hit
/// and rebuild agree). Returns the admitted nodes and whether any were left out.
pub(crate) fn admit_layer<T: Ord + Hash + Eq>(
    ties: HashMap<T, usize>,
    room: usize,
) -> (Vec<T>, bool) {
    let mut candidates: Vec<(T, usize)> = ties.into_iter().collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let over = candidates.len() > room;
    candidates.truncate(room);
    (candidates.into_iter().map(|(t, _)| t).collect(), over)
}

/// Keep at most [`MAX_EDGES`] of `edges` (both ends in `rank`, the BFS order
/// with the focus at 0). Every admitted node first keeps the link to its
/// nearest already-admitted neighbour, so no node is left floating; the rest
/// fill in nearest-first. Returns the kept edges and whether any were dropped.
pub(crate) fn cap_edges<T: Ord + Hash + Eq + Clone>(
    rank: &HashMap<T, usize>,
    edges: impl IntoIterator<Item = (T, T)>,
) -> (Vec<(T, T)>, bool) {
    let mut ordered: Vec<(usize, usize, T, T)> = edges
        .into_iter()
        .filter_map(|(s, d)| {
            let (a, b) = (*rank.get(&s)?, *rank.get(&d)?);
            Some((a.max(b), a.min(b), s, d))
        })
        .collect();
    if ordered.len() <= MAX_EDGES {
        return (
            ordered.into_iter().map(|(_, _, s, d)| (s, d)).collect(),
            false,
        );
    }
    ordered.sort_unstable();
    // Sorted by (farther end, nearer end): the first edge for each farther
    // end is that node's tie to its nearest neighbour.
    let mut tree = vec![false; ordered.len()];
    let mut last = None;
    for (i, e) in ordered.iter().enumerate() {
        if last != Some(e.0) {
            tree[i] = true;
            last = Some(e.0);
        }
    }
    let mut room = MAX_EDGES.saturating_sub(tree.iter().filter(|t| **t).count());
    let mut kept = Vec::with_capacity(MAX_EDGES);
    for (i, (_, _, s, d)) in ordered.into_iter().enumerate() {
        if tree[i] {
            kept.push((s, d));
        } else if room > 0 {
            room -= 1;
            kept.push((s, d));
        }
    }
    (kept, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layer_over_the_room_admits_the_best_connected_first() {
        let ties = HashMap::from([("c", 1), ("a", 1), ("b", 3)]);
        let (admitted, over) = admit_layer(ties, 2);
        assert_eq!(admitted, vec!["b", "a"]);
        assert!(over);
        let (all, over) = admit_layer(HashMap::from([("x", 1)]), 5);
        assert_eq!((all, over), (vec!["x"], false));
    }

    #[test]
    fn edge_cap_keeps_every_node_attached() {
        // A clique on 100 nodes has 4,950 links: far over the cap.
        let n = 100u32;
        let rank: HashMap<u32, usize> = (0..n).map(|i| (i, i as usize)).collect();
        let edges = (0..n).flat_map(|a| (a + 1..n).map(move |b| (a, b)));
        let (kept, dropped) = cap_edges(&rank, edges);
        assert!(dropped);
        assert_eq!(kept.len(), MAX_EDGES);
        for node in 1..n {
            assert!(
                kept.iter().any(|(a, b)| *a == node || *b == node),
                "node {node} lost every link"
            );
        }
        // Nearest first: the focus keeps its link to every node.
        assert_eq!(kept.iter().filter(|(a, _)| *a == 0).count(), 99);
    }
}

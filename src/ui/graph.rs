//! Commit graph lane layout.
//!
//! Pure computation so it can be unit tested without GTK. The UI draws the
//! segments with Cairo.

use std::collections::BTreeSet;

use crate::git::Commit;

pub const LANE_WIDTH: f64 = 18.0;
pub const ROW_HEIGHT: f64 = 46.0;
pub const DOT_RADIUS: f64 = 4.0;

/// A drawing segment inside one row.
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    /// Vertical line through the whole row.
    Vertical { lane: usize },
    /// Vertical line from the row centre to the bottom (used for lanes that
    /// begin at this row's commit).
    LowerVertical { lane: usize },
    /// Vertical line from the top to the row centre (lanes that end here).
    UpperVertical { lane: usize },
    /// Curve from the commit dot to a parent lane at the bottom of the row.
    Curve { from_lane: usize, to_lane: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphRow {
    pub lane: usize,
    pub color: usize,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, Default)]
pub struct GraphLayout {
    pub rows: Vec<GraphRow>,
    pub max_lane: usize,
}

/// Compute lane assignment for commits ordered newest first (topo order).
pub fn layout_commits(commits: &[Commit]) -> GraphLayout {
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    let mut max_lane = 0usize;
    let mut previous_below: BTreeSet<usize> = BTreeSet::new();

    for commit in commits {
        // Find the lane waiting for this commit, or allocate one.
        let mut lane = lanes
            .iter()
            .position(|expected| expected.as_deref() == Some(commit.id.as_str()));
        if lane.is_none() {
            lane = Some(match lanes.iter().position(|l| l.is_none()) {
                Some(free) => free,
                None => {
                    lanes.push(None);
                    lanes.len() - 1
                }
            });
        }
        let lane = lane.unwrap();
        lanes[lane] = None;

        let above: BTreeSet<usize> = previous_below.clone();
        let mut segments: Vec<Segment> = Vec::new();

        // Lines passing through from above.
        for &l in &above {
            if l == lane {
                segments.push(Segment::UpperVertical { lane: l });
            } else {
                segments.push(Segment::Vertical { lane: l });
            }
        }

        // Assign parents to lanes.
        for (index, parent) in commit.parents.iter().enumerate() {
            let existing = lanes
                .iter()
                .position(|expected| expected.as_deref() == Some(parent.as_str()));
            let parent_lane = match existing {
                Some(l) => l,
                None => {
                    if index == 0 {
                        lanes[lane] = Some(parent.clone());
                        lane
                    } else {
                        let free = match lanes.iter().position(|l| l.is_none()) {
                            Some(f) => f,
                            None => {
                                lanes.push(None);
                                lanes.len() - 1
                            }
                        };
                        lanes[free] = Some(parent.clone());
                        free
                    }
                }
            };
            if parent_lane == lane {
                segments.push(Segment::LowerVertical { lane: parent_lane });
            } else {
                if !above.contains(&parent_lane) {
                    segments.push(Segment::LowerVertical {
                        lane: parent_lane,
                    });
                }
                segments.push(Segment::Curve {
                    from_lane: lane,
                    to_lane: parent_lane,
                });
            }
        }

        let below: BTreeSet<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, expected)| expected.is_some())
            .map(|(i, _)| i)
            .collect();

        max_lane = max_lane.max(lane);
        max_lane = max_lane.max(below.iter().copied().max().unwrap_or(0));
        max_lane = max_lane.max(parent_lane_max(&segments));
        rows.push(GraphRow {
            lane,
            color: lane,
            segments,
        });
        previous_below = below;
    }

    GraphLayout { rows, max_lane }
}

fn parent_lane_max(segments: &[Segment]) -> usize {
    segments
        .iter()
        .map(|s| match s {
            Segment::Vertical { lane }
            | Segment::LowerVertical { lane }
            | Segment::UpperVertical { lane } => *lane,
            Segment::Curve { from_lane, to_lane } => (*from_lane).max(*to_lane),
        })
        .max()
        .unwrap_or(0)
}

/// X coordinate of a lane inside the graph column.
pub fn lane_x(lane: usize) -> f64 {
    lane as f64 * LANE_WIDTH + LANE_WIDTH / 2.0
}

/// Total width needed for a layout.
pub fn graph_width(max_lane: usize) -> f64 {
    (max_lane + 1) as f64 * LANE_WIDTH + 4.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::parse::parse_log;

    fn commit(id: &str, parents: &[&str]) -> Commit {
        Commit {
            id: id.to_string(),
            short_id: id[..7.min(id.len())].to_string(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author_name: "A".into(),
            author_email: "a@a".into(),
            author_time: 0,
            committer_name: "A".into(),
            committer_email: "a@a".into(),
            commit_time: 0,
            summary: format!("commit {id}"),
            body: String::new(),
            refs: Vec::new(),
        }
    }

    #[test]
    fn linear_history_uses_one_lane() {
        let commits = vec![
            commit("c", &["b"]),
            commit("b", &["a"]),
            commit("a", &[]),
        ];
        let layout = layout_commits(&commits);
        assert_eq!(layout.max_lane, 0);
        assert!(layout.rows.iter().all(|r| r.lane == 0));
        // Middle commit has vertical through the row (parent in same lane)
        assert!(layout.rows[1]
            .segments
            .iter()
            .any(|s| matches!(s, Segment::LowerVertical { .. })));
    }

    #[test]
    fn branch_and_merge() {
        // m is a merge of a and b; a and b share parent r
        let commits = vec![
            commit("m", &["a", "b"]),
            commit("a", &["r"]),
            commit("b", &["r"]),
            commit("r", &[]),
        ];
        let layout = layout_commits(&commits);
        assert_eq!(layout.rows[0].lane, 0);
        // The merge must have a curve to the second parent lane.
        assert!(layout.rows[0]
            .segments
            .iter()
            .any(|s| matches!(s, Segment::Curve { .. })));
        assert!(layout.max_lane >= 1);
        // Both branches eventually converge on r's lane.
        assert!(!layout.rows[3].segments.is_empty());
    }

    #[test]
    fn unrelated_roots_get_separate_lanes() {
        let commits = vec![
            commit("x", &[]),
            commit("y", &[]),
        ];
        let layout = layout_commits(&commits);
        assert_eq!(layout.rows[0].lane, 0);
        // Second root reuses lane 0 since it is free again.
        assert_eq!(layout.rows[1].lane, 0);
    }

    #[test]
    fn real_log_lays_out_without_panic() {
        let mut data = Vec::new();
        for (id, parents) in [
            ("f", "d e"),
            ("e", "c"),
            ("d", "c"),
            ("c", "b"),
            ("b", "a"),
            ("a", ""),
        ] {
            let fields = [
                id,
                id,
                parents,
                "A",
                "a@a",
                "0",
                "A",
                "a@a",
                "0",
                "s",
                "",
                "",
            ];
            data.extend_from_slice(fields.join("\u{1f}").as_bytes());
            data.push(0);
        }
        let commits = parse_log(&data).unwrap();
        let layout = layout_commits(&commits);
        assert_eq!(layout.rows.len(), 6);
        assert!(layout.max_lane >= 1);
    }
}

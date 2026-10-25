//! A tab's panes as a binary split tree, after muxy's `Layout`
//! (github.com/muxy-app/muxy, MIT).

use serde::{Deserialize, Serialize};

use crate::PaneId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// Side by side.
    Horizontal,
    /// Stacked.
    Vertical,
}

/// Which child of a split a path step takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Branch {
    First,
    Second,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Leaf(PaneId),
    Split {
        axis: Axis,
        /// Share of the first child, 0.15..=0.85.
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

pub const MIN_RATIO: f32 = 0.15;
pub const MAX_RATIO: f32 = 0.85;

impl Layout {
    /// Panes in reading order.
    pub fn panes(&self) -> Vec<PaneId> {
        let mut panes = Vec::new();
        self.collect(&mut panes);
        panes
    }

    fn collect(&self, panes: &mut Vec<PaneId>) {
        match self {
            Self::Leaf(pane) => panes.push(*pane),
            Self::Split { first, second, .. } => {
                first.collect(panes);
                second.collect(panes);
            }
        }
    }

    pub fn contains(&self, pane: PaneId) -> bool {
        match self {
            Self::Leaf(leaf) => *leaf == pane,
            Self::Split { first, second, .. } => first.contains(pane) || second.contains(pane),
        }
    }

    /// Put `new` next to `target`: right of it for a horizontal split, below
    /// it for a vertical one. Returns `false` when `target` isn't here.
    pub fn split(&mut self, target: PaneId, new: PaneId, axis: Axis) -> bool {
        match self {
            Self::Leaf(leaf) if *leaf == target => {
                *self = Self::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Leaf(target)),
                    second: Box::new(Self::Leaf(new)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split(target, new, axis) || second.split(target, new, axis)
            }
        }
    }

    /// The layout without `pane`; its sibling takes the freed space. `None`
    /// when `pane` was the last one.
    #[must_use]
    pub fn without(&self, pane: PaneId) -> Option<Self> {
        self.retain(&|leaf| leaf != pane)
    }

    /// Set the ratio of the split reached by `path`. Returns `false` when the
    /// path doesn't lead to a split.
    pub fn set_ratio(&mut self, path: &[Branch], ratio: f32) -> bool {
        match (self, path.split_first()) {
            (Self::Split { ratio: current, .. }, None) => {
                *current = ratio.clamp(MIN_RATIO, MAX_RATIO);
                true
            }
            (Self::Split { first, .. }, Some((Branch::First, rest))) => {
                first.set_ratio(rest, ratio)
            }
            (Self::Split { second, .. }, Some((Branch::Second, rest))) => {
                second.set_ratio(rest, ratio)
            }
            (Self::Leaf(_), _) => false,
        }
    }

    /// Clamp every ratio into range; stored files may hold anything.
    pub(crate) fn sanitize(&mut self) {
        if let Self::Split {
            ratio,
            first,
            second,
            ..
        } = self
        {
            *ratio = if ratio.is_finite() {
                ratio.clamp(MIN_RATIO, MAX_RATIO)
            } else {
                0.5
            };
            first.sanitize();
            second.sanitize();
        }
    }

    /// Keep only panes `keep` accepts. `None` when none survive.
    #[must_use]
    pub(crate) fn retain(&self, keep: &impl Fn(PaneId) -> bool) -> Option<Self> {
        match self {
            Self::Leaf(pane) => keep(*pane).then(|| self.clone()),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => match (first.retain(keep), second.retain(keep)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    axis: *axis,
                    ratio: *ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: u64) -> PaneId {
        PaneId(id)
    }

    #[test]
    fn closing_a_pane_gives_its_space_to_the_sibling() {
        let mut layout = Layout::Leaf(pane(1));
        assert!(layout.split(pane(1), pane(2), Axis::Horizontal));
        assert!(layout.split(pane(2), pane(3), Axis::Vertical));
        assert_eq!(layout.panes(), [pane(1), pane(2), pane(3)]);

        let without = layout.without(pane(2)).unwrap();
        assert_eq!(without.panes(), [pane(1), pane(3)]);
        // The vertical split collapsed into its survivor; the outer split stays.
        assert!(matches!(
            without,
            Layout::Split { axis: Axis::Horizontal, ref second, .. } if **second == Layout::Leaf(pane(3))
        ));
        assert_eq!(Layout::Leaf(pane(1)).without(pane(1)), None);
    }

    #[test]
    fn ratios_follow_paths_and_stay_in_range() {
        let mut layout = Layout::Leaf(pane(1));
        layout.split(pane(1), pane(2), Axis::Horizontal);
        layout.split(pane(2), pane(3), Axis::Vertical);
        assert!(layout.set_ratio(&[Branch::Second], 0.99));
        assert!(
            !layout.set_ratio(&[Branch::First], 0.3),
            "first child is a leaf"
        );
        let Layout::Split { second, .. } = &layout else {
            panic!("split expected");
        };
        assert!(matches!(**second, Layout::Split { ratio, .. } if ratio == MAX_RATIO));
    }
}

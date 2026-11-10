//! QNet group placement: split a show session into groups of at most four players.
//! Pure functions; `matchmaking::Members` owns the state.

use std::{collections::BTreeMap, fmt, num::NonZeroU32, time::Duration};

/// Players per group.
pub const DEFAULT_GROUP_SIZE: usize = 4;
/// How long a console that left keeps its place (`GAMESERVER_GROUP_GRACE_SECS`).
pub const DEFAULT_GRACE: Duration = Duration::from_secs(30);
/// The title's QNet session holds four players (`publicSlotsNum`, `xsession.lua`).
pub const MAX_GROUP_SIZE: usize = 4;

/// The QNet group a console is in. A console outside every group has none, which the wire writes
/// as 0 and the codec reads back as `None`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GroupId(NonZeroU32);

impl GroupId {
    /// The first group of a session.
    pub const FIRST: Self = Self(NonZeroU32::MIN);

    /// `id` as a group id; `None` for 0, which names no group.
    pub fn new(id: u32) -> Option<Self> {
        NonZeroU32::new(id).map(Self)
    }

    pub fn get(self) -> u32 {
        self.0.get()
    }

    pub fn to_be_bytes(self) -> [u8; 4] {
        self.0.get().to_be_bytes()
    }
}

impl fmt::Display for GroupId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupConfig {
    /// Players per group, 1 to [`MAX_GROUP_SIZE`].
    pub size: usize,
    pub grace: Duration,
}

impl Default for GroupConfig {
    fn default() -> Self {
        Self {
            size: DEFAULT_GROUP_SIZE,
            grace: DEFAULT_GRACE,
        }
    }
}

/// Why a console went into its group (logged with every placement).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Back into the group its reservation held.
    Reserved,
    /// The group with the fewest free slots that still fit.
    BestFit,
    New,
}

/// Group for a console: reserved first, then best fit, else a new group.
pub fn choose_group(
    occupancy: &BTreeMap<GroupId, usize>,
    reserved: Option<GroupId>,
    players: usize,
    size: usize,
) -> (GroupId, Placement) {
    let fits = |group: GroupId| occupancy.get(&group).copied().unwrap_or(0) + players <= size;
    if let Some(group) = reserved
        && fits(group)
    {
        return (group, Placement::Reserved);
    }
    let best = occupancy
        .iter()
        .filter(|(group, _)| fits(**group))
        .max_by_key(|(group, used)| (**used, std::cmp::Reverse(**group)));
    if let Some((group, _)) = best {
        return (*group, Placement::BestFit);
    }
    let highest = occupancy.keys().next_back().copied().max(reserved);
    // One past the highest group in use; a session never reaches the last 32-bit id.
    let next = highest.map_or(GroupId::FIRST, |group| {
        GroupId::new(group.get() + 1).expect("one past a group id")
    });
    (next, Placement::New)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The group with the wire number `id`.
    fn group(id: u32) -> GroupId {
        GroupId::new(id).expect("a group id")
    }

    /// The wire numbers of `groups`.
    fn numbers(groups: &[GroupId]) -> Vec<u32> {
        groups.iter().map(|group| group.get()).collect()
    }

    /// Place consoles of the given player counts one after another, as `Members` would.
    fn place_all(counts: &[usize], size: usize) -> Vec<GroupId> {
        let mut occupancy = BTreeMap::new();
        counts
            .iter()
            .map(|&players| {
                let (group, _) = choose_group(&occupancy, None, players, size);
                *occupancy.entry(group).or_default() += players;
                group
            })
            .collect()
    }

    #[test]
    fn the_proposals_ten_consoles_fill_eight_groups() {
        // matchmaking-groups.md, "Example: 10 consoles, 29 players".
        let groups = place_all(&[4, 4, 4, 3, 3, 3, 3, 2, 2, 1], 4);
        assert_eq!(numbers(&groups), [1, 2, 3, 4, 5, 6, 7, 8, 8, 4]);
    }

    #[test]
    fn single_player_consoles_fill_a_group_before_opening_the_next() {
        assert_eq!(numbers(&place_all(&[1, 1, 1, 1, 1], 4)), [1, 1, 1, 1, 2]);
    }

    #[test]
    fn best_fit_takes_the_fullest_group_that_still_fits() {
        let occupancy = BTreeMap::from([(group(1), 1), (group(2), 3), (group(3), 2)]);
        assert_eq!(
            choose_group(&occupancy, None, 1, 4),
            (group(2), Placement::BestFit)
        );
        assert_eq!(
            choose_group(&occupancy, None, 2, 4),
            (group(3), Placement::BestFit)
        );
        assert_eq!(
            choose_group(&occupancy, None, 3, 4),
            (group(1), Placement::BestFit)
        );
        assert_eq!(
            choose_group(&occupancy, None, 4, 4),
            (group(4), Placement::New)
        );
    }

    #[test]
    fn ties_go_to_the_lowest_group() {
        let occupancy = BTreeMap::from([(group(1), 2), (group(2), 2)]);
        assert_eq!(
            choose_group(&occupancy, None, 1, 4),
            (group(1), Placement::BestFit)
        );
    }

    #[test]
    fn a_reservation_wins_only_with_room_for_the_players() {
        let occupancy = BTreeMap::from([(group(1), 3), (group(2), 1)]);
        assert_eq!(
            choose_group(&occupancy, Some(group(2)), 2, 4),
            (group(2), Placement::Reserved)
        );
        // Its group took others meanwhile: placed anew.
        assert_eq!(
            choose_group(&occupancy, Some(group(1)), 2, 4),
            (group(2), Placement::BestFit)
        );
        // The reserved group is empty and unknown to the occupancy: it is reused, not renumbered.
        assert_eq!(
            choose_group(&BTreeMap::new(), Some(group(3)), 1, 4),
            (group(3), Placement::Reserved)
        );
    }

    #[test]
    fn reserved_slots_are_not_given_away_and_a_new_group_skips_the_reserved_id() {
        // Group 1: one connected player and a three-player reservation (in the occupancy).
        let occupancy = BTreeMap::from([(group(1), 4)]);
        assert_eq!(
            choose_group(&occupancy, None, 1, 4),
            (group(2), Placement::New)
        );
        assert_eq!(choose_group(&occupancy, Some(group(2)), 4, 1).0, group(3));
    }

    #[test]
    fn group_size_one_isolates_every_single_player() {
        assert_eq!(numbers(&place_all(&[1, 1, 1], 1)), [1, 2, 3]);
    }

    #[test]
    fn randomized_placement_never_puts_more_than_size_players_in_a_group() {
        let mut seed = 0x1234_5678_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        // (group, players) per console present.
        let mut consoles: Vec<(GroupId, usize)> = Vec::new();
        for _ in 0..2000 {
            if next() % 3 == 0 && !consoles.is_empty() {
                consoles.remove((next() as usize) % consoles.len());
            } else {
                let players = (next() % 4) as usize + 1;
                let reserved = (next() % 4 == 0)
                    .then(|| GroupId::new((next() % 6) as u32 + 1).expect("a group id"));
                let mut occupancy = BTreeMap::new();
                for (group, count) in &consoles {
                    *occupancy.entry(*group).or_default() += count;
                }
                let (group, _) = choose_group(&occupancy, reserved, players, 4);
                consoles.push((group, players));
            }
            let mut occupancy: BTreeMap<GroupId, usize> = BTreeMap::new();
            for (group, count) in &consoles {
                *occupancy.entry(*group).or_default() += count;
            }
            assert!(occupancy.values().all(|c| *c <= 4), "{occupancy:?}");
        }
    }
}

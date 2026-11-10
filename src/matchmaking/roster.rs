//! The roster: consoles in their sessions and groups, their players and inboxes, the
//! reservations of those that left. No I/O.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::Instant,
};

use crate::{
    groups::{self, GroupId},
    ids::{ConnId, Xuid},
    sync::lock,
    time::Utc,
};

use super::{
    Push, Writer,
    codec::{ANNOUNCE, CodecError, HEADER_LEN, MAX_MM_FRAME, encode, put_group, put_string},
};

/// Matchmaking roster; no request names a session.
pub struct Members {
    pub(super) consoles: BTreeMap<ConsoleKey, Console>,
    /// Places held for consoles that left, until they return or the grace period ends.
    pub(super) reservations: BTreeMap<ConsoleKey, Reservation>,
    /// The session a joining console is put into.
    pub(super) current_session: Box<dyn Fn() -> String + Send>,
    pub(super) group_config: groups::GroupConfig,
    /// Console address -> XUIDs of its players (user 0 first), as of its last JOIN, so SBG can
    /// tell which console a request came from, a console that left included.
    pub(super) console_players: std::collections::HashMap<std::net::IpAddr, Vec<Xuid>>,
}

/// A console in a session, identified by its first player (user 0).
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ConsoleKey {
    pub(super) session: String,
    pub(super) xuid: Xuid,
}

pub(super) struct Console {
    /// The signed-in local players, user 0 first.
    pub(super) players: Vec<Player>,
    /// Relayed QNet messages for this console (`from`, payload), oldest first.
    pub(super) inbox: std::collections::VecDeque<(Xuid, Vec<u8>)>,
    pub(super) owner: ConnId,
    /// QNet group inside the session: roster, relay and avatars are scoped to it.
    pub(super) group: GroupId,
    /// Where its frames go: its connection's socket, or memory in tests.
    pub(super) writer: Arc<dyn Writer>,
    pub(super) last_heartbeat: Instant,
    pub(super) first_seen: Utc,
}

pub(super) struct Player {
    pub(super) xuid: Xuid,
    pub(super) gamertag: String,
    /// The `XamAvatarGetMetadataLocalUser` blob, served to the other consoles of the group.
    pub(super) avatar: Option<Vec<u8>>,
}

pub(super) struct Reservation {
    pub(super) group: GroupId,
    pub(super) players: usize,
    pub(super) expires: Instant,
    /// Its players when it left, user 0 first: how the console is recognised when it comes back
    /// with another user 0, and what its player change is compared with.
    pub(super) xuids: Vec<Xuid>,
}

/// Writer of a console joined without a socket: its pushed frames go nowhere.
#[cfg(test)]
struct Discarded;

#[cfg(test)]
impl Writer for Discarded {
    fn write_frame(&self, _frame: &[u8]) -> std::io::Result<()> {
        Ok(())
    }
}

impl Default for Members {
    fn default() -> Self {
        Self {
            consoles: BTreeMap::new(),
            reservations: BTreeMap::new(),
            current_session: {
                // Without an `App`: a schedule of its own, which has no slots.
                let schedule = Mutex::new(crate::schedule::Schedule::default());
                Box::new(move || lock(&schedule).current_session_id(crate::time::now().into()))
            },
            group_config: Default::default(),
            console_players: Default::default(),
        }
    }
}

/// Admin-facing copy of one connected console.
pub struct ConsoleSnapshot {
    pub session: String,
    /// The first player's XUID: the console's identity.
    pub xuid: Xuid,
    pub players: Vec<PlayerSnapshot>,
    pub last_heartbeat_secs_ago: u64,
    pub first_seen: Utc,
    pub group: GroupId,
}

pub struct PlayerSnapshot {
    pub xuid: Xuid,
    pub gamertag: String,
    /// Size of the last avatar blob uploaded, absent until one is.
    pub avatar_bytes: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Population {
    pub consoles: usize,
    pub players: usize,
}

/// Admin-facing copy of one reservation.
pub struct ReservationSnapshot {
    pub session: String,
    pub xuid: Xuid,
    pub group: GroupId,
    pub players: usize,
    pub expires_in_secs: u64,
}

/// Why an operator's group move was refused.
#[derive(Debug)]
pub enum MoveError {
    /// No connected console has that first player.
    NoSuchConsole { xuid: Xuid },
    /// The group is 0, which is not a group.
    InvalidGroup { group: u32 },
    /// The group has no room left for the console's players.
    GroupFull { group: GroupId },
}

impl fmt::Display for MoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MoveError::NoSuchConsole { xuid } => {
                write!(f, "no console with first player {xuid:016x}", xuid = xuid.0)
            }
            MoveError::InvalidGroup { group } => write!(f, "group {group} is invalid"),
            MoveError::GroupFull { group } => write!(f, "group {group} is full"),
        }
    }
}

impl std::error::Error for MoveError {}

impl Members {
    /// Remember which players the console at `ip` joined with, user 0 first (also used by SBG
    /// replays).
    pub fn note_console_players(&mut self, ip: std::net::IpAddr, players: &[Xuid]) {
        self.console_players.insert(ip, players.to_vec());
    }

    /// The XUID that identifies the console at `ip`: its first player.
    pub fn xuid_for_console(&self, ip: std::net::IpAddr) -> Option<Xuid> {
        self.console_players
            .get(&ip)
            .and_then(|players| players.first().copied())
    }

    /// The players the console at `ip` joined with (none if it never joined).
    pub fn players_of(&self, ip: std::net::IpAddr) -> &[Xuid] {
        self.console_players.get(&ip).map_or(&[], Vec::as_slice)
    }

    /// Where the session a joining console goes into comes from (`App::new`: the schedule).
    pub fn set_current_session(&mut self, current: impl Fn() -> String + Send + 'static) {
        self.current_session = Box::new(current);
    }

    pub fn set_group_config(&mut self, config: groups::GroupConfig) {
        self.group_config = config;
    }

    /// Players per group in `session`: connected consoles and reservations.
    pub(super) fn occupancy(&self, session: &str) -> BTreeMap<GroupId, usize> {
        let mut occupancy = BTreeMap::new();
        for (key, console) in &self.consoles {
            if key.session == session {
                *occupancy.entry(console.group).or_default() += console.players.len();
            }
        }
        for (key, reservation) in &self.reservations {
            if key.session == session {
                *occupancy.entry(reservation.group).or_default() += reservation.players;
            }
        }
        occupancy
    }

    /// Players connected in `session` (JOIN_ACK and HEARTBEAT_ACK).
    pub(super) fn population(&self, session: &str) -> usize {
        self.consoles
            .iter()
            .filter(|(key, _)| key.session == session)
            .map(|(_, console)| console.players.len())
            .sum()
    }

    /// The group of the console `key` when this connection still owns it.
    pub(super) fn owned(&self, key: &ConsoleKey, owner: ConnId) -> Option<&Console> {
        self.consoles
            .get(key)
            .filter(|console| console.owner == owner)
    }

    /// ROSTER_REPLY / ANNOUNCE payload: the connected consoles of `group` (the receiver included),
    /// then the group id (0: not a member).
    pub(super) fn roster_body(
        &self,
        session: &str,
        group: Option<GroupId>,
    ) -> Result<Vec<u8>, CodecError> {
        let consoles: Vec<_> = self
            .consoles
            .iter()
            .filter(|(key, console)| key.session == session && Some(console.group) == group)
            .map(|(_, console)| console)
            .collect();
        let mut body = (consoles.len() as u16).to_be_bytes().to_vec();
        for console in consoles {
            body.push(console.players.len() as u8);
            for player in &console.players {
                body.extend_from_slice(&player.xuid.to_be_bytes());
                put_string(&mut body, &player.gamertag)?;
            }
        }
        put_group(&mut body, group);
        if HEADER_LEN + body.len() > MAX_MM_FRAME {
            return Err(CodecError::RosterTooLarge {
                len: HEADER_LEN + body.len(),
            });
        }
        Ok(body)
    }

    /// ANNOUNCE the roster of `group` to its connected consoles, except `except` (the console
    /// that caused the change gets its own reply).
    pub(super) fn announce(
        &self,
        session: &str,
        group: GroupId,
        except: Option<&ConsoleKey>,
    ) -> Vec<Push> {
        let body = match self.roster_body(session, Some(group)) {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(%error, session, group = group.get(), "matchmaking ANNOUNCE not sent");
                return Vec::new();
            }
        };
        let frame = encode(ANNOUNCE, &body);
        self.consoles
            .iter()
            .filter(|(key, console)| {
                key.session == session && console.group == group && Some(*key) != except
            })
            .map(|(_, console)| (Arc::clone(&console.writer), frame.clone()))
            .collect()
    }

    /// The console `key` is not connected (`why`): it keeps its place for the grace period.
    pub(super) fn disconnect(
        &mut self,
        key: &ConsoleKey,
        owner: ConnId,
        why: &str,
        now: Instant,
    ) -> Vec<Push> {
        if self.owned(key, owner).is_none() {
            return Vec::new();
        }
        let console = self.consoles.remove(key).expect("owned");
        let grace = self.group_config.grace;
        self.reservations.insert(
            key.clone(),
            Reservation {
                group: console.group,
                players: console.players.len(),
                expires: now + grace,
                xuids: console.players.iter().map(|player| player.xuid).collect(),
            },
        );
        tracing::info!(session = %key.session, xuid = key.xuid.0, group = console.group.get(), players = console.players.len(), grace_secs = grace.as_secs(), why, "matchmaking console left; place reserved");
        crate::timeline::record(
            "matchmaking",
            format!("{:016x} left group {} ({why})", key.xuid.0, console.group),
            Some(&key.session),
        );
        self.announce(&key.session, console.group, None)
    }

    /// Another connected console of `key`'s session with one of `players` (key and owner): the
    /// same console under a former user 0. A profile is signed in on one console at a time, so a
    /// shared player is the same console.
    pub(super) fn same_console(
        &self,
        key: &ConsoleKey,
        players: &[Xuid],
    ) -> Option<(ConsoleKey, ConnId)> {
        self.consoles
            .iter()
            .find(|(other, console)| {
                *other != key
                    && other.session == key.session
                    && console
                        .players
                        .iter()
                        .any(|player| players.contains(&player.xuid))
            })
            .map(|(other, console)| (other.clone(), console.owner))
    }

    /// The reservation a console joining as `key` with `players` comes back to: the one under
    /// its key, or else one in the same session held for any of its players (it left and joined
    /// again with another user 0: its first player signed out, or a new one signed in on
    /// controller 0). Taken out either way; an expired one is dropped and gives nothing.
    pub(super) fn take_reservation(
        &mut self,
        key: &ConsoleKey,
        players: &[Xuid],
        now: Instant,
    ) -> Option<Reservation> {
        let held = self.reservations.remove(key).or_else(|| {
            let same_console = self
                .reservations
                .iter()
                .find(|(held, reservation)| {
                    held.session == key.session
                        && reservation.xuids.iter().any(|xuid| players.contains(xuid))
                })
                .map(|(held, _)| held.clone())?;
            self.reservations.remove(&same_console)
        })?;
        (held.expires > now).then_some(held)
    }

    /// Admin: put the console whose first player is `xuid` into `group`, when its players fit.
    pub(super) fn move_console(
        &mut self,
        xuid: Xuid,
        group: GroupId,
    ) -> Result<Vec<Push>, MoveError> {
        let Some(key) = self.consoles.keys().find(|key| key.xuid == xuid).cloned() else {
            return Err(MoveError::NoSuchConsole { xuid });
        };
        let (from, players) = {
            let console = &self.consoles[&key];
            (console.group, console.players.len())
        };
        let used = self
            .occupancy(&key.session)
            .get(&group)
            .copied()
            .unwrap_or(0);
        let used = if group == from { used - players } else { used };
        if used + players > self.group_config.size {
            return Err(MoveError::GroupFull { group });
        }
        self.consoles.get_mut(&key).unwrap().group = group;
        tracing::info!(session = %key.session, xuid = xuid.0, from = from.get(), group = group.get(), players, "matchmaking console moved by the operator");
        let mut pushes = self.announce(&key.session, from, None);
        if group != from {
            pushes.extend(self.announce(&key.session, group, None));
        }
        Ok(pushes)
    }

    /// A console in `session` with `gamertags` signed in, joined without a socket.
    #[cfg(test)]
    pub fn insert_test_console(&mut self, session: &str, xuid: Xuid, gamertags: &[&str]) {
        let players = gamertags
            .iter()
            .enumerate()
            .map(|(offset, gamertag)| Player {
                xuid: Xuid(xuid.0 + offset as u64),
                gamertag: (*gamertag).to_owned(),
                avatar: None,
            })
            .collect();
        self.consoles.insert(
            ConsoleKey {
                session: session.to_owned(),
                xuid,
            },
            Console {
                players,
                inbox: Default::default(),
                owner: ConnId(0),
                group: GroupId::FIRST,
                writer: Arc::new(Discarded),
                last_heartbeat: Instant::now(),
                first_seen: crate::time::now(),
            },
        );
    }

    /// Admin reset: drop the reservations. Connected consoles keep their groups (nobody is
    /// moved automatically).
    pub fn clear_reservations(&mut self) {
        self.reservations.clear();
    }

    /// Connected consoles and their signed-in players, in every session.
    pub fn total_population(&self) -> Population {
        Population {
            consoles: self.consoles.len(),
            players: self
                .consoles
                .values()
                .map(|console| console.players.len())
                .sum(),
        }
    }

    pub fn snapshot(&self) -> Vec<ConsoleSnapshot> {
        let now = Instant::now();
        self.consoles
            .iter()
            .map(|(key, console)| ConsoleSnapshot {
                session: key.session.clone(),
                xuid: key.xuid,
                players: console
                    .players
                    .iter()
                    .map(|player| PlayerSnapshot {
                        xuid: player.xuid,
                        gamertag: player.gamertag.clone(),
                        avatar_bytes: player.avatar.as_ref().map(Vec::len),
                    })
                    .collect(),
                last_heartbeat_secs_ago: now
                    .saturating_duration_since(console.last_heartbeat)
                    .as_secs(),
                first_seen: console.first_seen,
                group: console.group,
            })
            .collect()
    }

    pub fn reservations_snapshot(&self) -> Vec<ReservationSnapshot> {
        let now = Instant::now();
        self.reservations
            .iter()
            .map(|(key, reservation)| ReservationSnapshot {
                session: key.session.clone(),
                xuid: key.xuid,
                group: reservation.group,
                players: reservation.players,
                expires_in_secs: reservation.expires.saturating_duration_since(now).as_secs(),
            })
            .collect()
    }
}

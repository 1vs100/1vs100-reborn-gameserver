//! One handler per message type, on the state of the connection it came on.

use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use crate::{
    groups::{self, Placement},
    ids::{ConnId, Xuid},
    sync::lock,
};

use super::{
    Writer,
    codec::{
        AVATAR_ACK, AVATAR_GET, AVATAR_PUT, AVATAR_REPLY, CodecError, Cursor, Frame, HEARTBEAT,
        HEARTBEAT_ACK, JOIN, JOIN_ACK, LEAVE, RELAY_ACK, RELAY_POLL, RELAY_PUT, RELAY_REPLY,
        ROSTER, ROSTER_REPLY, RosterPlayer, encode, u32_of,
    },
    deliver,
    roster::{Console, ConsoleKey, Members, Player},
};

/// Largest relayed QNet payload.
const MAX_RELAY_BYTES: usize = 8192;

/// Undelivered relayed messages kept per console.
const MAX_INBOX: usize = 256;

/// Upper bound on one `RELAY_POLL` reply payload.
const MAX_POLL_REPLY: usize = 8400;

/// Largest avatar metadata blob stored per player.
pub(super) const MAX_AVATAR_BYTES: usize = 1024;

/// Largest gamertag in bytes.
pub(super) const MAX_GAMERTAG_BYTES: usize = 31;

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

/// Connection-ending rejection cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Reject {
    /// The frame's protocol version is not this server's.
    Version = 1,
    /// Unparseable JOIN.
    MalformedJoin = 2,
    UnknownType = 3,
}

/// Request that ends the connection.
#[derive(Debug)]
pub(super) enum Refusal {
    /// The connection ends with a REJECT carrying `reason`.
    Reject(Reject),
    /// The frame's payload does not decode or breaks a protocol limit: it is dropped, and the
    /// connection handler logs `error`. `reject` answers the client first when it is set.
    Malformed {
        reject: Option<Reject>,
        error: RequestError,
    },
}

impl From<RequestError> for Refusal {
    fn from(error: RequestError) -> Self {
        Self::Malformed {
            reject: None,
            error,
        }
    }
}

impl From<CodecError> for Refusal {
    fn from(error: CodecError) -> Self {
        Self::from(RequestError::from(error))
    }
}

/// Why a request payload is refused: it does not decode, or it breaks a protocol limit.
#[derive(Debug)]
pub(super) enum RequestError {
    /// The payload does not decode.
    Codec(CodecError),
    /// A JOIN whose player count is outside 1 to `groups::MAX_GROUP_SIZE`.
    PlayerCount { count: usize },
    /// A gamertag longer than [`MAX_GAMERTAG_BYTES`].
    GamertagLength { len: usize },
    /// The same player twice in one JOIN.
    DuplicatePlayer { xuid: Xuid },
    /// A request that needs a JOIN before it.
    BeforeJoin,
    /// An avatar blob that is empty or longer than [`MAX_AVATAR_BYTES`].
    AvatarLength { len: usize },
    /// A relay with more than 8 target players.
    RelayTargets { count: usize },
    /// A relayed payload that is empty or longer than [`MAX_RELAY_BYTES`].
    RelayLength { len: usize },
}

impl From<CodecError> for RequestError {
    fn from(error: CodecError) -> Self {
        Self::Codec(error)
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::Codec(error) => error.fmt(f),
            RequestError::PlayerCount { count } => write!(f, "{count} players in one JOIN"),
            RequestError::GamertagLength { len } => {
                write!(f, "gamertag of {len} bytes, over {MAX_GAMERTAG_BYTES}")
            }
            RequestError::DuplicatePlayer { xuid } => {
                write!(f, "player {xuid:016x} joined twice", xuid = xuid.0)
            }
            RequestError::BeforeJoin => f.write_str("request before JOIN"),
            RequestError::AvatarLength { len } => {
                write!(f, "avatar of {len} bytes, out of 1 to {MAX_AVATAR_BYTES}")
            }
            RequestError::RelayTargets { count } => write!(f, "{count} relay targets"),
            RequestError::RelayLength { len } => {
                write!(
                    f,
                    "relayed payload of {len} bytes, out of 1 to {MAX_RELAY_BYTES}"
                )
            }
        }
    }
}

impl std::error::Error for RequestError {}

/// One matchmaking connection: its id, where its frames go, and the client behind it.
pub(super) struct Client {
    pub(super) owner: ConnId,
    pub(super) writer: Arc<dyn Writer>,
    pub(super) registry: Registry,
    pub(super) state: ClientState,
}

/// What a client has done on its connection so far.
pub(super) enum ClientState {
    /// It has not joined: a request that needs a console is refused.
    Connected,
    /// It joined as `console`, with the players of its last JOIN, user 0 first.
    Joined {
        console: ConsoleKey,
        players: Vec<Xuid>,
    },
}

/// Player registry: gamertag and last avatar per XUID, in storage.
#[derive(Clone, Default)]
pub struct Registry(Option<Arc<dyn crate::storage::Storage>>);

impl Registry {
    pub fn new(storage: Arc<dyn crate::storage::Storage>) -> Self {
        Self(Some(storage))
    }

    /// Record a player seen joining. A JOIN may carry an empty gamertag, which names no one and
    /// keeps the known name.
    fn remember_join(&self, xuid: Xuid, gamertag: &str) {
        let gamertag = Some(gamertag).filter(|gamertag| !gamertag.is_empty());
        self.update(xuid, |storage, now| {
            storage.touch_player(xuid, gamertag, now)
        });
    }

    /// Record a player's latest avatar.
    fn remember_avatar(&self, xuid: Xuid, avatar: &[u8]) {
        self.update(xuid, |storage, now| storage.set_avatar(xuid, avatar, now));
    }

    /// Storage failure never affects matchmaking.
    fn update(
        &self,
        xuid: Xuid,
        write: impl FnOnce(&dyn crate::storage::Storage, crate::time::Utc) -> crate::storage::Result<()>,
    ) {
        let Some(storage) = &self.0 else {
            return;
        };
        if let Err(error) = write(storage.as_ref(), crate::time::now()) {
            tracing::warn!(%error, xuid = xuid.0, "player registry update failed");
        }
    }
}

impl Client {
    /// A connection that has not joined yet; its frames go to `writer`.
    pub(super) fn connected(writer: Arc<dyn Writer>, registry: Registry) -> Self {
        Self {
            owner: ConnId(NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed)),
            writer,
            registry,
            state: ClientState::Connected,
        }
    }
}

impl ClientState {
    /// Who this connection joined as. A request before JOIN ends the connection; the client
    /// joins again.
    fn console(&self) -> Result<&ConsoleKey, RequestError> {
        match self {
            Self::Connected => Err(RequestError::BeforeJoin),
            Self::Joined { console, .. } => Ok(console),
        }
    }

    /// The console identified by `xuid` in the session this connection joined.
    fn console_of(&self, xuid: Xuid) -> Result<ConsoleKey, RequestError> {
        Ok(ConsoleKey {
            session: self.console()?.session.clone(),
            xuid,
        })
    }
}

pub(super) fn handle_frame(
    frame: Frame,
    members: &Mutex<Members>,
    client: &mut Client,
) -> Result<Option<Vec<u8>>, Refusal> {
    Ok(match frame.kind {
        JOIN => Some(join(frame.payload, members, client)?),
        HEARTBEAT => Some(heartbeat(frame.payload, members, client)?),
        LEAVE => {
            leave(frame.payload, members, client)?;
            None
        }
        ROSTER => Some(roster(frame.payload, members, client)?),
        AVATAR_PUT => Some(avatar_put(frame.payload, members, client)?),
        AVATAR_GET => Some(avatar_get(frame.payload, members, client)?),
        RELAY_PUT => Some(relay_put(frame.payload, members, client)?),
        RELAY_POLL => Some(relay_poll(frame.payload, members, client)?),
        kind => {
            tracing::warn!(kind, "unknown matchmaking message type");
            return Err(Refusal::Reject(Reject::UnknownType));
        }
    })
}

/// JOIN: `u8` player count (1-4), then per player `xuid`, gamertag; user 0 first.
fn parse_join(payload: &[u8]) -> Result<Vec<RosterPlayer>, RequestError> {
    let mut cursor = Cursor(payload);
    let count = cursor.take(1, "player count")?[0] as usize;
    if !(1..=groups::MAX_GROUP_SIZE).contains(&count) {
        return Err(RequestError::PlayerCount { count });
    }
    let mut players: Vec<RosterPlayer> = Vec::with_capacity(count);
    for _ in 0..count {
        let player = RosterPlayer {
            xuid: cursor.xuid()?,
            gamertag: cursor.string()?,
        };
        if player.gamertag.len() > MAX_GAMERTAG_BYTES {
            return Err(RequestError::GamertagLength {
                len: player.gamertag.len(),
            });
        }
        if players.iter().any(|known| known.xuid == player.xuid) {
            return Err(RequestError::DuplicatePlayer { xuid: player.xuid });
        }
        players.push(player);
    }
    cursor.finish()?;
    Ok(players)
}

pub(super) fn join(
    payload: Vec<u8>,
    members: &Mutex<Members>,
    client: &mut Client,
) -> Result<Vec<u8>, Refusal> {
    let players = parse_join(&payload).map_err(|error| Refusal::Malformed {
        reject: Some(Reject::MalformedJoin),
        error,
    })?;

    let mut members = lock(members);
    let now = Instant::now();
    let mut pushes = Vec::new();
    let previous = std::mem::replace(&mut client.state, ClientState::Connected);
    if let ClientState::Joined { console, .. } = &previous {
        pushes.extend(members.disconnect(console, client.owner, "joined again", now));
    }
    let key = ConsoleKey {
        session: (members.current_session)(),
        xuid: players[0].xuid,
    };
    if let Some(replaced) = members.consoles.remove(&key) {
        // The same console on a new connection (the old one is dead or about to be).
        pushes.extend(members.announce(&key.session, replaced.group, None));
    }
    let xuids: Vec<Xuid> = players.iter().map(|player| player.xuid).collect();
    if let Some((stale, owner)) = members.same_console(&key, &xuids) {
        // The same console under its former user 0, still connected: it leaves now, and its
        // reservation is what this join comes back to.
        pushes.extend(members.disconnect(&stale, owner, "joined again with another user 0", now));
    }
    let reservation = members.take_reservation(&key, &xuids, now);
    // What the players were: this connection's last JOIN, else the console's before it left.
    let before = match &previous {
        ClientState::Joined { players, .. } => players.clone(),
        ClientState::Connected => reservation
            .as_ref()
            .map(|held| held.xuids.clone())
            .unwrap_or_default(),
    };
    log_player_change(&key, &before, &players);
    let reserved = reservation.map(|reservation| reservation.group);
    let occupancy = members.occupancy(&key.session);
    let size = members.group_config.size;
    let (group, placement) = groups::choose_group(&occupancy, reserved, players.len(), size);
    let free = size.saturating_sub(occupancy.get(&group).copied().unwrap_or(0) + players.len());
    let gamertags: Vec<_> = players
        .iter()
        .map(|player| player.gamertag.as_str())
        .collect();
    tracing::info!(session = %key.session, xuid = key.xuid.0, players = players.len(), gamertags = ?gamertags, group = group.get(), free, ?placement, "matchmaking join");
    crate::timeline::record(
        "matchmaking",
        format!(
            "{} ({:016x}, {} player{}) joined group {group}{}",
            players[0].gamertag,
            key.xuid.0,
            players.len(),
            if players.len() == 1 { "" } else { "s" },
            if placement == Placement::Reserved {
                " (reserved)"
            } else {
                ""
            },
        ),
        Some(&key.session),
    );
    for player in &players {
        client.registry.remember_join(player.xuid, &player.gamertag);
    }
    members.consoles.insert(
        key.clone(),
        Console {
            players: players
                .into_iter()
                .map(|player| Player {
                    xuid: player.xuid,
                    gamertag: player.gamertag,
                    avatar: None,
                })
                .collect(),
            inbox: Default::default(),
            owner: client.owner,
            group,
            writer: Arc::clone(&client.writer),
            last_heartbeat: now,
            first_seen: crate::time::now(),
        },
    );
    pushes.extend(members.announce(&key.session, group, Some(&key)));
    let population = members.population(&key.session);
    client.state = ClientState::Joined {
        console: key,
        players: xuids,
    };
    drop(members);
    deliver(pushes);

    let mut body = u32_of(population)?.to_be_bytes().to_vec();
    body.extend_from_slice(&group.to_be_bytes());
    Ok(encode(JOIN_ACK, &body))
}

/// A console that joins again with other local players (a second controller signed in or out):
/// the players that came and those that went. `None` on a first join (nothing `before`) and on a
/// rejoin with the same players in the same order.
pub(super) fn player_change(before: &[Xuid], now: &[Xuid]) -> Option<(Vec<Xuid>, Vec<Xuid>)> {
    if before.is_empty() || before == now {
        return None;
    }
    let added = now
        .iter()
        .copied()
        .filter(|xuid| !before.contains(xuid))
        .collect();
    let removed = before
        .iter()
        .copied()
        .filter(|xuid| !now.contains(xuid))
        .collect();
    Some((added, removed))
}

fn log_player_change(key: &ConsoleKey, before: &[Xuid], players: &[RosterPlayer]) {
    let now: Vec<Xuid> = players.iter().map(|player| player.xuid).collect();
    let Some((added, removed)) = player_change(before, &now) else {
        return;
    };
    let hex = |xuids: Vec<Xuid>| -> Vec<String> {
        xuids
            .into_iter()
            .map(|xuid| format!("{:016x}", xuid.0))
            .collect()
    };
    let (added, removed) = (hex(added), hex(removed));
    tracing::info!(session = %key.session, xuid = key.xuid.0, ?added, ?removed, players = now.len(), "matchmaking players changed");
}

pub(super) fn heartbeat(
    payload: Vec<u8>,
    members: &Mutex<Members>,
    client: &mut Client,
) -> Result<Vec<u8>, RequestError> {
    let mut cursor = Cursor(&payload);
    let key = client.state.console_of(cursor.xuid()?)?;
    cursor.finish()?;

    let mut members = lock(members);
    match members
        .consoles
        .get_mut(&key)
        .filter(|console| console.owner == client.owner)
    {
        Some(console) => {
            console.last_heartbeat = Instant::now();
            tracing::trace!(session = %key.session, xuid = key.xuid.0, "matchmaking heartbeat");
        }
        None => {
            tracing::trace!(session = %key.session, xuid = key.xuid.0, "heartbeat for unknown or foreign console ignored");
        }
    }
    let population = u32_of(members.population(&key.session))?;
    Ok(encode(HEARTBEAT_ACK, &population.to_be_bytes()))
}

pub(super) fn leave(
    payload: Vec<u8>,
    members: &Mutex<Members>,
    client: &mut Client,
) -> Result<(), RequestError> {
    let mut cursor = Cursor(&payload);
    let key = client.state.console_of(cursor.xuid()?)?;
    cursor.finish()?;

    let pushes = lock(members).disconnect(&key, client.owner, "leave", Instant::now());
    if matches!(&client.state, ClientState::Joined { console, .. } if console == &key) {
        client.state = ClientState::Connected;
    }
    deliver(pushes);
    Ok(())
}

/// `ROSTER` (empty payload): the connected consoles of the asking connection's own group.
pub(super) fn roster(
    payload: Vec<u8>,
    members: &Mutex<Members>,
    client: &Client,
) -> Result<Vec<u8>, RequestError> {
    Cursor(&payload).finish()?;
    let me = client.state.console()?;

    let members = lock(members);
    tracing::trace!(session = %me.session, "matchmaking roster request");
    let group = members.owned(me, client.owner).map(|console| console.group);
    Ok(encode(
        ROSTER_REPLY,
        &members.roster_body(&me.session, group)?,
    ))
}

/// `AVATAR_PUT` (`xuid`, `u16` length, blob): the avatar of one of the sender's players. Ack
/// carries 1 when stored.
pub(super) fn avatar_put(
    payload: Vec<u8>,
    members: &Mutex<Members>,
    client: &Client,
) -> Result<Vec<u8>, RequestError> {
    let mut cursor = Cursor(&payload);
    let xuid = cursor.xuid()?;
    let len = u16::from_be_bytes(cursor.take(2, "avatar length")?.try_into().unwrap()) as usize;
    if len == 0 || len > MAX_AVATAR_BYTES {
        return Err(RequestError::AvatarLength { len });
    }
    let blob = cursor.take(len, "avatar")?.to_vec();
    cursor.finish()?;
    let me = client.state.console()?;

    let mut members = lock(members);
    let player = members
        .consoles
        .get_mut(me)
        .filter(|console| console.owner == client.owner)
        .and_then(|console| console.players.iter_mut().find(|p| p.xuid == xuid));
    let stored = match player {
        Some(player) => {
            tracing::info!(session = %me.session, xuid = xuid.0, bytes = len, "matchmaking avatar stored");
            player.avatar = Some(blob.clone());
            client.registry.remember_avatar(xuid, &blob);
            true
        }
        None => {
            tracing::debug!(session = %me.session, xuid = xuid.0, "avatar for a player of another console ignored");
            false
        }
    };
    Ok(encode(AVATAR_ACK, &[u8::from(stored)]))
}

/// `AVATAR_GET` (a player's xuid): `found u8`, then when found `u16` length and the blob. Only
/// answered for players of the requester's own group.
pub(super) fn avatar_get(
    payload: Vec<u8>,
    members: &Mutex<Members>,
    client: &Client,
) -> Result<Vec<u8>, RequestError> {
    let mut cursor = Cursor(&payload);
    let xuid = cursor.xuid()?;
    cursor.finish()?;
    let me = client.state.console()?;

    let members = lock(members);
    let group = members.owned(me, client.owner).map(|console| console.group);
    let blob = members
        .consoles
        .iter()
        .filter(|(key, console)| key.session == me.session && Some(console.group) == group)
        .flat_map(|(_, console)| &console.players)
        .find(|player| player.xuid == xuid)
        .and_then(|player| player.avatar.as_ref());
    let mut body = Vec::new();
    match blob {
        Some(blob) => {
            body.push(1);
            body.extend_from_slice(&(blob.len() as u16).to_be_bytes());
            body.extend_from_slice(blob);
        }
        None => body.push(0),
    }
    Ok(encode(AVATAR_REPLY, &body))
}

/// `RELAY_PUT` (`xuid`, `u16` target count + `u64` each, `u16` length, bytes): queue a QNet
/// payload for the other consoles of the sender's group, or those holding one of the target
/// players. `from` is `xuid` when that player is the sender's, else the sender's first player.
/// Ack carries how many consoles it was queued for.
pub(super) fn relay_put(
    payload: Vec<u8>,
    members: &Mutex<Members>,
    client: &Client,
) -> Result<Vec<u8>, RequestError> {
    let mut cursor = Cursor(&payload);
    let xuid = cursor.xuid()?;
    let target_count =
        u16::from_be_bytes(cursor.take(2, "relay target count")?.try_into().unwrap()) as usize;
    if target_count > 8 {
        return Err(RequestError::RelayTargets {
            count: target_count,
        });
    }
    let mut targets = Vec::with_capacity(target_count);
    for _ in 0..target_count {
        targets.push(cursor.xuid()?);
    }
    let len = u16::from_be_bytes(cursor.take(2, "relay length")?.try_into().unwrap()) as usize;
    if len == 0 || len > MAX_RELAY_BYTES {
        return Err(RequestError::RelayLength { len });
    }
    let data = cursor.take(len, "relay payload")?.to_vec();
    cursor.finish()?;
    let me = client.state.console()?.clone();

    let mut members = lock(members);
    let Some(sender) = members.owned(&me, client.owner) else {
        tracing::debug!(session = %me.session, xuid = me.xuid.0, "relay from a console that is not joined ignored");
        return Ok(encode(RELAY_ACK, &[0]));
    };
    let group = sender.group;
    let from = if sender.players.iter().any(|player| player.xuid == xuid) {
        xuid
    } else {
        me.xuid
    };
    let mut queued = 0u8;
    for (key, console) in members.consoles.iter_mut() {
        if key.session != me.session
            || console.group != group
            || *key == me
            || (!targets.is_empty() && !console.players.iter().any(|p| targets.contains(&p.xuid)))
        {
            continue;
        }
        if console.inbox.len() >= MAX_INBOX {
            console.inbox.pop_front();
        }
        console.inbox.push_back((from, data.clone()));
        queued = queued.saturating_add(1);
    }
    tracing::trace!(session = %me.session, from = from.0, bytes = len, queued, "relay queued");
    Ok(encode(RELAY_ACK, &[queued]))
}

/// `RELAY_POLL` (the console's first xuid): take its pending messages. Reply: `count:u16`, then
/// per message `from:u64`, `len:u16`, bytes; stops before the reply would exceed
/// [`MAX_POLL_REPLY`].
pub(super) fn relay_poll(
    payload: Vec<u8>,
    members: &Mutex<Members>,
    client: &Client,
) -> Result<Vec<u8>, RequestError> {
    let mut cursor = Cursor(&payload);
    let key = client.state.console_of(cursor.xuid()?)?;
    cursor.finish()?;

    let mut members = lock(members);
    let mut body = vec![0u8, 0u8];
    let mut count: u16 = 0;
    if let Some(console) = members
        .consoles
        .get_mut(&key)
        .filter(|console| console.owner == client.owner)
    {
        while let Some((_, data)) = console.inbox.front() {
            if body.len() + 10 + data.len() > MAX_POLL_REPLY {
                break;
            }
            let (from, data) = console.inbox.pop_front().unwrap();
            body.extend_from_slice(&from.to_be_bytes());
            body.extend_from_slice(&(data.len() as u16).to_be_bytes());
            body.extend_from_slice(&data);
            count += 1;
        }
    }
    body[..2].copy_from_slice(&count.to_be_bytes());
    Ok(encode(RELAY_REPLY, &body))
}

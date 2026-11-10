//! Matchmaking service, protocol version 2.
//! Consoles join with all local players into groups of at most four; leaving consoles hold a reservation.

mod codec;
mod conn;
mod handlers;
mod roster;

use std::{
    io,
    sync::{Arc, Mutex},
};

pub use codec::*;
pub use conn::serve;
pub use handlers::Registry;
pub use roster::*;

use crate::{groups::GroupId, ids::Xuid, sync::lock};

/// Where one console's frames go: its connection's socket in production, memory in tests.
pub(super) trait Writer: Send + Sync {
    /// Write one whole frame.
    fn write_frame(&self, frame: &[u8]) -> io::Result<()>;
}

/// An ANNOUNCE to push once the roster lock is released.
type Push = (Arc<dyn Writer>, Vec<u8>);

/// Move a console into another group and tell both groups.
pub fn move_console(members: &Mutex<Members>, xuid: Xuid, group: u32) -> Result<(), MoveError> {
    let target = GroupId::new(group).ok_or(MoveError::InvalidGroup { group })?;
    let pushes = lock(members).move_console(xuid, target)?;
    deliver(pushes);
    Ok(())
}

/// Deliver pushed ANNOUNCEs; failures belong to the target connection.
fn deliver(pushes: Vec<Push>) {
    for (writer, frame) in pushes {
        if let Err(error) = writer.write_frame(&frame) {
            tracing::debug!(%error, "matchmaking ANNOUNCE push failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::{conn::*, handlers::*, *};
    use crate::{
        groups::{self, GroupId},
        ids::Xuid,
    };
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    /// The group with the wire number `id`.
    fn group(id: u32) -> GroupId {
        GroupId::new(id).expect("a group id")
    }

    /// A console writer whose frames land in memory.
    #[derive(Default)]
    struct MemoryWriter(Mutex<Vec<u8>>);

    impl Writer for MemoryWriter {
        fn write_frame(&self, frame: &[u8]) -> std::io::Result<()> {
            lock(&self.0).extend_from_slice(frame);
            Ok(())
        }
    }

    impl MemoryWriter {
        /// The frames written since the last take.
        fn take(&self) -> Vec<u8> {
            std::mem::take(&mut *lock(&self.0))
        }
    }

    /// A connection that has not joined yet, whose pushed frames land in memory.
    fn connected() -> Client {
        Client::connected(Arc::new(MemoryWriter::default()), Registry::default())
    }

    fn string(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(s.len() as u16).to_be_bytes());
        out.extend_from_slice(s.as_bytes());
        out
    }

    fn request(kind: u8, payload: &[u8]) -> Vec<u8> {
        encode(kind, payload)
    }

    /// Members serving `session`.
    fn serving(session: &'static str) -> Members {
        Members {
            current_session: Box::new(move || session.to_owned()),
            ..Members::default()
        }
    }

    /// The server runs `session` from now on (the schedule moved on).
    fn serve_session(members: &Mutex<Members>, session: &'static str) {
        crate::sync::lock(members).current_session = Box::new(move || session.to_owned());
    }

    fn join_players(players: &[(Xuid, &str)]) -> Vec<u8> {
        let mut payload = vec![players.len() as u8];
        for (xuid, gamertag) in players {
            payload.extend_from_slice(&xuid.to_be_bytes());
            payload.extend_from_slice(&string(gamertag));
        }
        request(JOIN, &payload)
    }

    fn join(xuid: Xuid, gamertag: &str) -> Vec<u8> {
        join_players(&[(xuid, gamertag)])
    }

    fn heartbeat(xuid: Xuid) -> Vec<u8> {
        request(HEARTBEAT, &xuid.to_be_bytes())
    }

    fn leave(xuid: Xuid) -> Vec<u8> {
        request(LEAVE, &xuid.to_be_bytes())
    }

    fn roster() -> Vec<u8> {
        request(ROSTER, &[])
    }

    fn one_response(
        input: &[u8],
        members: &Mutex<Members>,
        client: &mut Client,
    ) -> Option<Vec<u8>> {
        let frame = read_frame(&mut &input[..]).unwrap().unwrap();
        handle_frame(frame, members, client).unwrap()
    }

    /// Connection-ending outcome of a request; `None` closes silently.
    fn refusal(input: &[u8], members: &Mutex<Members>, client: &mut Client) -> Option<Reject> {
        let frame = read_frame(&mut &input[..]).unwrap().unwrap();
        match handle_frame(frame, members, client) {
            Err(Refusal::Reject(reason)) => Some(reason),
            Err(Refusal::Malformed { reject, .. }) => reject,
            Ok(reply) => panic!("answered: {reply:?}"),
        }
    }

    /// A client whose pushed frames land in the returned writer.
    fn listening() -> (Client, Arc<MemoryWriter>) {
        let writer = Arc::new(MemoryWriter::default());
        (
            Client::connected(Arc::clone(&writer) as Arc<dyn Writer>, Registry::default()),
            writer,
        )
    }

    /// Every frame in `bytes`: (type, payload).
    fn frames(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut rest = bytes;
        let mut out = Vec::new();
        while !rest.is_empty() {
            assert_eq!(rest[0], VERSION);
            let len = u32::from_be_bytes(rest[2..6].try_into().unwrap()) as usize;
            out.push((rest[1], rest[6..6 + len].to_vec()));
            rest = &rest[6 + len..];
        }
        out
    }

    fn take_pushes(writer: &MemoryWriter) -> Vec<(u8, Vec<u8>)> {
        frames(&writer.take())
    }

    fn roster_payload(payload: &[u8]) -> Roster {
        decode_roster(payload).expect("a roster")
    }

    fn roster_of(reply: &[u8]) -> Roster {
        let frames = frames(reply);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, ROSTER_REPLY);
        roster_payload(&frames[0].1)
    }

    fn player(xuid: Xuid, gamertag: &str) -> RosterPlayer {
        RosterPlayer {
            xuid,
            gamertag: gamertag.to_owned(),
        }
    }

    /// The first XUID of every console in a roster reply.
    fn roster_consoles(reply: &[u8]) -> Vec<Xuid> {
        roster_of(reply)
            .consoles
            .iter()
            .map(|players| players[0].xuid)
            .collect()
    }

    fn group_ids(members: &Mutex<Members>) -> Vec<(u64, u32)> {
        crate::sync::lock(members)
            .snapshot()
            .iter()
            .map(|c| (c.xuid.0, c.group.get()))
            .collect()
    }

    fn join_all(members: &Mutex<Members>, count: u64) -> Vec<Client> {
        (1..=count)
            .map(|xuid| {
                let mut client = connected();
                one_response(&join(Xuid(xuid), &format!("P{xuid}")), members, &mut client).unwrap();
                client
            })
            .collect()
    }

    #[test]
    fn join_ack_carries_the_player_population_and_the_group() {
        let members = Mutex::new(serving("S"));
        let mut client = connected();
        assert_eq!(
            one_response(&join(Xuid(1), "A"), &members, &mut client).unwrap(),
            [2, JOIN_ACK, 0, 0, 0, 8, 0, 0, 0, 1, 0, 0, 0, 1]
        );
        let mut two = connected();
        let ack = one_response(
            &join_players(&[(Xuid(2), "B"), (Xuid(3), "C")]),
            &members,
            &mut two,
        )
        .unwrap();
        assert_eq!(frames(&ack), [(JOIN_ACK, vec![0, 0, 0, 3, 0, 0, 0, 1])]);
        assert_eq!(
            one_response(&heartbeat(Xuid(1)), &members, &mut client).unwrap(),
            request(HEARTBEAT_ACK, &3u32.to_be_bytes())
        );
    }

    #[test]
    fn a_console_joins_with_every_local_player_and_the_roster_lists_them_by_console() {
        let members = Mutex::new(serving("S"));
        let (mut a, mut b) = (connected(), connected());
        one_response(
            &join_players(&[(Xuid(1), "A0"), (Xuid(2), "A1")]),
            &members,
            &mut a,
        );
        one_response(&join(Xuid(3), "B0"), &members, &mut b);
        let decoded = roster_of(&one_response(&roster(), &members, &mut b).unwrap());
        assert_eq!(decoded.group, Some(group(1)));
        assert_eq!(
            decoded.consoles,
            [
                vec![player(Xuid(1), "A0"), player(Xuid(2), "A1")],
                vec![player(Xuid(3), "B0")]
            ]
        );
    }

    #[test]
    fn malformed_joins_are_rejected() {
        let members = Mutex::new(serving("S"));
        let long = "x".repeat(MAX_GAMERTAG_BYTES + 1);
        for input in [
            request(JOIN, &[0]),
            request(JOIN, &[5]),
            request(JOIN, &[1, 0, 0]),
            join_players(&[(Xuid(1), "A"), (Xuid(1), "B")]),
            join(Xuid(1), &long),
            {
                let mut input = join(Xuid(1), "A");
                input.push(0);
                input[5] += 1;
                input
            },
        ] {
            assert_eq!(
                refusal(&input, &members, &mut connected()),
                Some(Reject::MalformedJoin),
                "{input:?}"
            );
        }
        assert!(crate::sync::lock(&members).snapshot().is_empty());
    }

    #[test]
    fn a_frame_that_does_not_decode_is_dropped_with_its_reason() {
        let members = Mutex::new(serving("S"));
        let mut client = connected();
        let dropped = |input: &[u8], client: &mut Client| {
            let frame = read_frame(&mut &input[..]).unwrap().unwrap();
            match handle_frame(frame, &members, client) {
                Err(Refusal::Malformed { reject, error }) => (reject, error),
                other => panic!("not dropped: {other:?}"),
            }
        };

        let (reject, error) = dropped(&request(JOIN, &[1, 0, 0]), &mut client);
        assert_eq!(reject, Some(Reject::MalformedJoin));
        assert!(matches!(
            error,
            RequestError::Codec(CodecError::Truncated { what: "xuid" })
        ));

        let (reject, error) = dropped(&request(HEARTBEAT, &[0, 0]), &mut client);
        assert_eq!(reject, None);
        assert!(matches!(
            error,
            RequestError::Codec(CodecError::Truncated { what: "xuid" })
        ));
    }

    #[test]
    fn an_unknown_message_type_is_rejected_and_a_request_before_join_closes_silently() {
        let members = Mutex::new(serving("S"));
        assert_eq!(
            refusal(&request(0x42, &[]), &members, &mut connected()),
            Some(Reject::UnknownType)
        );
        for input in [
            heartbeat(Xuid(1)),
            leave(Xuid(1)),
            roster(),
            avatar_put(Xuid(1), &[1; 8]),
            avatar_get(Xuid(1)),
            relay_put(Xuid(1), &[], b"x"),
            relay_poll(Xuid(1)),
        ] {
            assert_eq!(
                refusal(&input, &members, &mut connected()),
                None,
                "type {}",
                input[1]
            );
        }
    }

    #[test]
    fn a_panic_while_holding_the_roster_does_not_break_later_requests() {
        let members = Arc::new(Mutex::new(serving("S")));
        let poisoner = Arc::clone(&members);
        let _ = std::thread::spawn(move || {
            let _guard = crate::sync::lock(&poisoner);
            panic!("connection thread panics while holding the roster");
        })
        .join();
        assert!(members.is_poisoned());

        let mut client = connected();
        one_response(&join(Xuid(1), "A"), &members, &mut client).unwrap();
        sweep(&mut lock(&members), Instant::now(), STALE_THRESHOLD);
        let reply = one_response(&roster(), &members, &mut client).unwrap();
        assert_eq!(roster_consoles(&reply), [Xuid(1)]);
    }

    #[test]
    fn consoles_join_the_session_the_server_runs_and_stay_in_it() {
        let members = Mutex::new(serving("S"));
        let (mut a, mut b, mut c) = (connected(), connected(), connected());
        one_response(&join(Xuid(1), "A"), &members, &mut a);
        one_response(&join(Xuid(2), "B"), &members, &mut b);
        serve_session(&members, "T");
        one_response(&join(Xuid(3), "C"), &members, &mut c);

        let sessions: Vec<_> = lock(&members)
            .snapshot()
            .into_iter()
            .map(|c| (c.session, c.xuid.0))
            .collect();
        assert_eq!(
            sessions,
            [
                ("S".to_owned(), 1),
                ("S".to_owned(), 2),
                ("T".to_owned(), 3)
            ]
        );
        assert_eq!(
            roster_consoles(&one_response(&roster(), &members, &mut a).unwrap()),
            [Xuid(1), Xuid(2)]
        );
        assert_eq!(
            roster_consoles(&one_response(&roster(), &members, &mut c).unwrap()),
            [Xuid(3)]
        );
        assert_eq!(
            one_response(&heartbeat(Xuid(1)), &members, &mut a).unwrap(),
            request(HEARTBEAT_ACK, &2u32.to_be_bytes())
        );
        // Joining again moves the connection to the session the server runs now.
        one_response(&join(Xuid(1), "A"), &members, &mut a);
        assert!(
            lock(&members)
                .snapshot()
                .iter()
                .any(|c| (c.session.as_str(), c.xuid.0) == ("T", 1))
        );
    }

    #[test]
    fn by_default_the_session_is_the_one_the_grid_lists() {
        let members = Mutex::new(Members::default());
        one_response(&join(Xuid(1), "A"), &members, &mut connected());
        assert_eq!(
            lock(&members).snapshot()[0].session,
            crate::schedule::Schedule::default().current_session_id(SystemTime::now())
        );
    }

    #[test]
    fn single_player_consoles_fill_groups_of_four() {
        let members = Mutex::new(serving("S"));
        let mut clients = join_all(&members, 5);
        assert_eq!(
            group_ids(&members),
            [(1, 1), (2, 1), (3, 1), (4, 1), (5, 2)]
        );
        let first = one_response(&roster(), &members, &mut clients[0]).unwrap();
        assert_eq!(roster_of(&first).group, Some(group(1)));
        assert_eq!(
            roster_consoles(&first),
            [Xuid(1), Xuid(2), Xuid(3), Xuid(4)]
        );
        let last = one_response(&roster(), &members, &mut clients[4]).unwrap();
        assert_eq!(
            roster_of(&last),
            Roster {
                consoles: vec![vec![player(Xuid(5), "P5")]],
                group: Some(group(2))
            }
        );
    }

    #[test]
    fn groups_count_players_and_take_the_best_fit() {
        let members = Mutex::new(serving("S"));
        let join_with = |first: Xuid, players: usize| {
            let players: Vec<_> = (0..players as u64)
                .map(|i| (Xuid(first.0 + i), "P"))
                .collect();
            one_response(&join_players(&players), &members, &mut connected()).unwrap();
        };
        join_with(Xuid(10), 3); // group 1: 3 players
        join_with(Xuid(20), 2); // does not fit group 1: group 2
        join_with(Xuid(30), 1); // best fit: group 1 (1 free) before group 2 (2 free)
        join_with(Xuid(40), 2); // group 2 now full
        assert_eq!(group_ids(&members), [(10, 1), (20, 2), (30, 1), (40, 2)]);
    }

    #[test]
    fn a_console_with_no_group_gets_the_group_field_zero_on_the_wire() {
        let members = Mutex::new(serving("S"));
        let mut first = connected();
        one_response(&join(Xuid(1), "A"), &members, &mut first);
        // The same console joined again on a new connection: its old entry is gone.
        one_response(&join(Xuid(1), "A"), &members, &mut connected());
        let reply = one_response(&roster(), &members, &mut first).unwrap();
        assert_eq!(roster_of(&reply).group, None);
        assert_eq!(frames(&reply)[0].1, [0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_console_that_leaves_keeps_its_slots_for_the_grace_period() {
        let members = Mutex::new(serving("S"));
        let mut clients = join_all(&members, 4);
        one_response(&leave(Xuid(2)), &members, &mut clients[1]);
        assert_eq!(
            roster_consoles(&one_response(&roster(), &members, &mut clients[0]).unwrap()),
            [Xuid(1), Xuid(3), Xuid(4)]
        );
        // Its slot is not given away: a newcomer opens group 2.
        one_response(&join(Xuid(5), "P5"), &members, &mut connected());
        assert!(group_ids(&members).contains(&(5, 2)));
        // It comes back into group 1.
        one_response(&join(Xuid(2), "P2"), &members, &mut clients[1]);
        assert!(group_ids(&members).contains(&(2, 1)));
        assert!(lock(&members).reservations_snapshot().is_empty());
    }

    #[test]
    fn a_console_whose_first_player_signs_out_keeps_its_group_through_the_rejoin() {
        let members = Mutex::new(serving("S"));
        let mut phat = connected();
        one_response(
            &join_players(&[(Xuid(1), "P1"), (Xuid(2), "P2")]),
            &members,
            &mut phat,
        );
        one_response(
            &join_players(&[(Xuid(3), "P3"), (Xuid(4), "P4")]),
            &members,
            &mut connected(),
        );
        // P1 signs out: the loader leaves and joins again, now with P2 as user 0.
        one_response(&leave(Xuid(1)), &members, &mut phat);
        one_response(&join(Xuid(5), "P5"), &members, &mut connected());
        assert!(
            group_ids(&members).contains(&(5, 2)),
            "the leaving console's slots stay reserved"
        );
        one_response(&join(Xuid(2), "P2"), &members, &mut phat);
        assert!(group_ids(&members).contains(&(2, 1)), "back in its group");
        assert!(lock(&members).reservations_snapshot().is_empty());
    }

    #[test]
    fn a_rejoin_from_a_new_connection_retires_the_old_entry_of_the_same_console() {
        let members = Mutex::new(serving("S"));
        one_response(
            &join_players(&[(Xuid(1), "P1"), (Xuid(2), "P2")]),
            &members,
            &mut connected(),
        );
        // The same console on a new connection, P1 signed out meanwhile.
        one_response(&join(Xuid(2), "P2"), &members, &mut connected());
        assert_eq!(group_ids(&members), [(2, 1)], "counted once, in its group");
        assert!(lock(&members).reservations_snapshot().is_empty());
    }

    #[test]
    fn a_join_reports_which_local_players_came_and_went() {
        assert_eq!(
            handlers::player_change(&[], &[Xuid(1), Xuid(2)]),
            None,
            "first join"
        );
        assert_eq!(
            handlers::player_change(&[Xuid(1), Xuid(2)], &[Xuid(1), Xuid(2)]),
            None
        );
        assert_eq!(
            handlers::player_change(&[Xuid(1)], &[Xuid(1), Xuid(2)]),
            Some((vec![Xuid(2)], vec![]))
        );
        assert_eq!(
            handlers::player_change(&[Xuid(1), Xuid(2)], &[Xuid(2)]),
            Some((vec![], vec![Xuid(1)]))
        );
    }

    #[test]
    fn an_expired_reservation_frees_its_slots() {
        let members = Mutex::new(serving("S"));
        let mut clients = join_all(&members, 4);
        one_response(&leave(Xuid(4)), &members, &mut clients[3]);
        let later = Instant::now() + groups::DEFAULT_GRACE + Duration::from_secs(1);
        sweep(&mut lock(&members), later, Duration::from_secs(3600));
        assert!(lock(&members).reservations_snapshot().is_empty());
        one_response(&join(Xuid(5), "P5"), &members, &mut connected());
        assert!(group_ids(&members).contains(&(5, 1)));
    }

    #[test]
    fn a_silent_console_starts_its_grace_at_the_sweep() {
        let members = Mutex::new(serving("S"));
        join_all(&members, 2);
        let later = Instant::now() + STALE_THRESHOLD + Duration::from_secs(1);
        sweep(&mut lock(&members), later, STALE_THRESHOLD);
        assert!(lock(&members).snapshot().is_empty());
        let reserved: Vec<_> = lock(&members)
            .reservations_snapshot()
            .iter()
            .map(|r| (r.xuid.0, r.group.get(), r.players))
            .collect();
        assert_eq!(reserved, [(1, 1, 1), (2, 1, 1)]);
    }

    #[test]
    fn a_rejoin_with_more_players_than_its_group_has_room_for_is_placed_anew() {
        let members = Mutex::new(serving("S"));
        let mut clients = join_all(&members, 4);
        one_response(&leave(Xuid(4)), &members, &mut clients[3]);
        one_response(
            &join_players(&[(Xuid(4), "P4"), (Xuid(44), "P4b")]),
            &members,
            &mut clients[3],
        );
        assert!(group_ids(&members).contains(&(4, 2)));
    }

    #[test]
    fn group_changes_are_announced_to_the_rest_of_the_group() {
        let members = Mutex::new(serving("S"));
        let ((mut a, a_pushed), (mut b, b_pushed), (mut c, c_pushed)) =
            (listening(), listening(), listening());
        one_response(&join(Xuid(1), "A"), &members, &mut a);
        one_response(&join(Xuid(2), "B"), &members, &mut b);
        assert_eq!(
            take_pushes(&b_pushed),
            [],
            "the joiner gets its reply, not a push"
        );
        let announced = take_pushes(&a_pushed);
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].0, ANNOUNCE);
        assert_eq!(
            roster_payload(&announced[0].1),
            Roster {
                consoles: vec![vec![player(Xuid(1), "A")], vec![player(Xuid(2), "B")]],
                group: Some(group(1))
            }
        );

        one_response(&join(Xuid(3), "C"), &members, &mut c);
        take_pushes(&a_pushed);
        take_pushes(&b_pushed);
        one_response(&leave(Xuid(2)), &members, &mut b);
        for pushed in [&a_pushed, &c_pushed] {
            let announced = take_pushes(pushed);
            assert_eq!(
                roster_payload(&announced[0].1).consoles,
                [vec![player(Xuid(1), "A")], vec![player(Xuid(3), "C")]]
            );
        }
        assert_eq!(take_pushes(&b_pushed), []);
    }

    #[test]
    fn an_operator_move_checks_player_slots_and_announces_both_groups() {
        let mut inner = serving("S");
        inner.set_group_config(groups::GroupConfig {
            size: 2,
            ..Default::default()
        });
        let members = Mutex::new(inner);
        let ((mut a, a_pushed), (mut b, _), (mut c, c_pushed)) =
            (listening(), listening(), listening());
        one_response(&join(Xuid(1), "A"), &members, &mut a);
        one_response(&join(Xuid(2), "B"), &members, &mut b);
        one_response(&join(Xuid(3), "C"), &members, &mut c);
        assert_eq!(group_ids(&members), [(1, 1), (2, 1), (3, 2)]);
        take_pushes(&a_pushed);

        assert!(move_console(&members, Xuid(1), 2).is_ok());
        assert_eq!(group_ids(&members), [(1, 2), (2, 1), (3, 2)]);
        assert_eq!(
            roster_payload(&take_pushes(&a_pushed)[0].1).group,
            Some(group(2))
        );
        assert_eq!(
            roster_payload(&take_pushes(&c_pushed)[0].1).group,
            Some(group(2))
        );
        assert!(matches!(
            move_console(&members, Xuid(2), 2),
            Err(MoveError::GroupFull { group: full }) if full == group(2)
        ));
        assert!(matches!(
            move_console(&members, Xuid(2), 0),
            Err(MoveError::InvalidGroup { group: 0 })
        ));
        assert!(matches!(
            move_console(&members, Xuid(99), 1),
            Err(MoveError::NoSuchConsole { xuid: Xuid(99) })
        ));
    }

    #[test]
    fn oversized_roster_is_refused_instead_of_exceeding_the_512_byte_frame_cap() {
        // Four players cannot overflow 512 bytes, so lift the group size to prove the frame cap
        // itself still protects the reply.
        let mut inner = serving("S");
        inner.set_group_config(groups::GroupConfig {
            size: 64,
            ..Default::default()
        });
        let members = Mutex::new(inner);
        let mut first = connected();
        for i in 0..32u64 {
            let mut client = connected();
            one_response(&join(Xuid(i), "ABCDEFGHIJKLMNOP"), &members, &mut client);
            if i == 0 {
                first = client;
            }
        }
        assert_eq!(refusal(&roster(), &members, &mut first), None);
    }

    fn avatar_put(xuid: Xuid, blob: &[u8]) -> Vec<u8> {
        let mut payload = xuid.to_be_bytes().to_vec();
        payload.extend_from_slice(&(blob.len() as u16).to_be_bytes());
        payload.extend_from_slice(blob);
        request(AVATAR_PUT, &payload)
    }

    fn avatar_get(xuid: Xuid) -> Vec<u8> {
        request(AVATAR_GET, &xuid.to_be_bytes())
    }

    #[test]
    fn every_local_player_has_its_own_avatar_served_inside_the_group() {
        let members = Mutex::new(serving("S"));
        let (mut a, mut b) = (connected(), connected());
        one_response(
            &join_players(&[(Xuid(1), "A0"), (Xuid(2), "A1")]),
            &members,
            &mut a,
        );
        one_response(&join(Xuid(3), "B0"), &members, &mut b);
        let blob: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();

        assert_eq!(
            one_response(&avatar_put(Xuid(2), &blob), &members, &mut a).unwrap(),
            request(AVATAR_ACK, &[1])
        );
        let reply = frames(&one_response(&avatar_get(Xuid(2)), &members, &mut b).unwrap());
        assert_eq!(reply[0].0, AVATAR_REPLY);
        assert_eq!(reply[0].1[0], 1);
        assert_eq!(u16::from_be_bytes([reply[0].1[1], reply[0].1[2]]), 1000);
        assert_eq!(&reply[0].1[3..], &blob[..]);
        // User 0 has none yet; another console's player cannot be uploaded for.
        assert_eq!(
            one_response(&avatar_get(Xuid(1)), &members, &mut b).unwrap(),
            request(AVATAR_REPLY, &[0])
        );
        assert_eq!(
            one_response(&avatar_put(Xuid(3), &[1; 8]), &members, &mut a).unwrap(),
            request(AVATAR_ACK, &[0])
        );
        let sizes: Vec<_> = lock(&members).snapshot()[0]
            .players
            .iter()
            .map(|p| p.avatar_bytes)
            .collect();
        assert_eq!(sizes, [None, Some(1000)]);
    }

    #[test]
    fn oversized_avatar_is_refused() {
        let members = Mutex::new(serving("S"));
        let mut owner = connected();
        one_response(&join(Xuid(7), "A"), &members, &mut owner).unwrap();
        let input = avatar_put(Xuid(7), &vec![0; MAX_AVATAR_BYTES + 1]);
        assert_eq!(refusal(&input, &members, &mut owner), None);
    }

    fn relay_put(from: Xuid, targets: &[Xuid], data: &[u8]) -> Vec<u8> {
        let mut payload = from.to_be_bytes().to_vec();
        payload.extend_from_slice(&(targets.len() as u16).to_be_bytes());
        for target in targets {
            payload.extend_from_slice(&target.to_be_bytes());
        }
        payload.extend_from_slice(&(data.len() as u16).to_be_bytes());
        payload.extend_from_slice(data);
        request(RELAY_PUT, &payload)
    }

    fn relay_poll(xuid: Xuid) -> Vec<u8> {
        request(RELAY_POLL, &xuid.to_be_bytes())
    }

    /// (from, payload) pairs of a `RELAY_POLL` reply.
    fn poll_messages(reply: &[u8]) -> Vec<(u64, Vec<u8>)> {
        let frames = frames(reply);
        assert_eq!(frames[0].0, RELAY_REPLY);
        let mut tail = &frames[0].1[..];
        let count = u16::from_be_bytes(tail[..2].try_into().unwrap());
        tail = &tail[2..];
        let mut out = Vec::new();
        for _ in 0..count {
            let from = u64::from_be_bytes(tail[..8].try_into().unwrap());
            let len = u16::from_be_bytes(tail[8..10].try_into().unwrap()) as usize;
            out.push((from, tail[10..10 + len].to_vec()));
            tail = &tail[10 + len..];
        }
        assert!(tail.is_empty());
        out
    }

    #[test]
    fn relayed_payload_reaches_the_rest_of_the_group_once_and_in_order() {
        let members = Mutex::new(serving("S"));
        let (mut a, mut b) = (connected(), connected());
        one_response(&join(Xuid(1), "A"), &members, &mut a).unwrap();
        one_response(&join(Xuid(2), "B"), &members, &mut b).unwrap();
        for text in [&b"one"[..], b"two", b"three"] {
            let ack = one_response(&relay_put(Xuid(1), &[], text), &members, &mut a).unwrap();
            assert_eq!(ack, request(RELAY_ACK, &[1]));
        }
        let got = poll_messages(&one_response(&relay_poll(Xuid(2)), &members, &mut b).unwrap());
        assert_eq!(
            got,
            [
                (1, b"one".to_vec()),
                (1, b"two".to_vec()),
                (1, b"three".to_vec())
            ]
        );
        assert!(
            poll_messages(&one_response(&relay_poll(Xuid(2)), &members, &mut b).unwrap())
                .is_empty()
        );
        assert!(
            poll_messages(&one_response(&relay_poll(Xuid(1)), &members, &mut a).unwrap())
                .is_empty()
        );
    }

    #[test]
    fn relay_targets_players_and_from_names_the_sending_player() {
        let members = Mutex::new(serving("S"));
        let (mut a, mut b, mut c) = (connected(), connected(), connected());
        one_response(
            &join_players(&[(Xuid(1), "A0"), (Xuid(2), "A1")]),
            &members,
            &mut a,
        );
        one_response(
            &join_players(&[(Xuid(3), "B0"), (Xuid(4), "B1")]),
            &members,
            &mut b,
        );
        // Group 1 is full (4 players): C opens group 2.
        one_response(&join(Xuid(5), "C0"), &members, &mut c);

        // A target names B's second player: delivered to B. `from` is A's second player.
        let ack =
            one_response(&relay_put(Xuid(2), &[Xuid(4)], b"to-b1"), &members, &mut a).unwrap();
        assert_eq!(ack, request(RELAY_ACK, &[1]));
        assert_eq!(
            poll_messages(&one_response(&relay_poll(Xuid(3)), &members, &mut b).unwrap()),
            [(2, b"to-b1".to_vec())]
        );
        // A sender XUID that is not the sender's own: `from` is its first player.
        one_response(&relay_put(Xuid(4), &[], b"x"), &members, &mut a).unwrap();
        assert_eq!(
            poll_messages(&one_response(&relay_poll(Xuid(3)), &members, &mut b).unwrap()),
            [(1, b"x".to_vec())]
        );
        // Never across groups, not even to an explicit target.
        let ack = one_response(&relay_put(Xuid(1), &[Xuid(5)], b"y"), &members, &mut a).unwrap();
        assert_eq!(ack, request(RELAY_ACK, &[0]));
        assert!(
            poll_messages(&one_response(&relay_poll(Xuid(5)), &members, &mut c).unwrap())
                .is_empty()
        );
    }

    #[test]
    fn relay_to_a_console_in_grace_is_dropped_and_not_replayed_on_return() {
        let members = Mutex::new(serving("S"));
        let (mut a, mut b) = (connected(), connected());
        one_response(&join(Xuid(1), "A"), &members, &mut a);
        one_response(&join(Xuid(2), "B"), &members, &mut b);
        one_response(&relay_put(Xuid(1), &[], b"before"), &members, &mut a);
        one_response(&leave(Xuid(2)), &members, &mut b);
        let ack = one_response(&relay_put(Xuid(1), &[], b"during"), &members, &mut a).unwrap();
        assert_eq!(ack, request(RELAY_ACK, &[0]));
        one_response(&join(Xuid(2), "B"), &members, &mut b);
        assert!(
            poll_messages(&one_response(&relay_poll(Xuid(2)), &members, &mut b).unwrap())
                .is_empty()
        );
    }

    #[test]
    fn relay_poll_reply_is_bounded_and_the_rest_stays_queued() {
        let members = Mutex::new(serving("S"));
        let (mut a, mut b) = (connected(), connected());
        one_response(&join(Xuid(1), "A"), &members, &mut a).unwrap();
        one_response(&join(Xuid(2), "B"), &members, &mut b).unwrap();
        for i in 0..4u8 {
            one_response(&relay_put(Xuid(1), &[], &[i; 3000]), &members, &mut a).unwrap();
        }
        let first = poll_messages(&one_response(&relay_poll(Xuid(2)), &members, &mut b).unwrap());
        assert_eq!(first.len(), 2);
        let rest = poll_messages(&one_response(&relay_poll(Xuid(2)), &members, &mut b).unwrap());
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[0].1[0], 2);
    }

    #[test]
    fn every_player_is_recorded_in_the_player_registry() {
        use crate::storage::{MemoryStorage, Storage};
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::default());
        let members = Mutex::new(Members::default());
        let mut client = Client {
            registry: Registry::new(Arc::clone(&storage)),
            ..connected()
        };
        one_response(
            &join_players(&[
                (Xuid(0xE000_0690_52E8_AD19), "Player1"),
                (Xuid(7), "Player2"),
            ]),
            &members,
            &mut client,
        );
        one_response(&avatar_put(Xuid(7), &[7; 1000]), &members, &mut client);

        let players = storage.players().unwrap();
        assert_eq!(players.len(), 2);
        let second = players.iter().find(|p| p.xuid == Xuid(7)).unwrap();
        assert_eq!(second.gamertag.as_deref(), Some("Player2"));
        assert_eq!(second.avatar.as_deref(), Some(&[7u8; 1000][..]));
        // The registry outlives the roster.
        one_response(&leave(Xuid(0xE000_0690_52E8_AD19)), &members, &mut client);
        assert_eq!(storage.players().unwrap().len(), 2);
    }

    #[test]
    fn a_join_without_a_gamertag_keeps_the_known_one() {
        use crate::storage::{MemoryStorage, SqliteStorage, Storage};
        let backends: [(&str, Arc<dyn Storage>); 2] = [
            ("memory", Arc::new(MemoryStorage::default())),
            ("sqlite", Arc::new(SqliteStorage::open_in_memory().unwrap())),
        ];
        for (name, storage) in backends {
            let members = Mutex::new(Members::default());
            let mut client = Client {
                registry: Registry::new(Arc::clone(&storage)),
                ..connected()
            };
            one_response(
                &join_players(&[(Xuid(7), "Player2")]),
                &members,
                &mut client,
            );
            one_response(&leave(Xuid(7)), &members, &mut client);
            one_response(&join_players(&[(Xuid(7), "")]), &members, &mut client);
            let players = storage.players().unwrap();
            assert_eq!(players[0].gamertag.as_deref(), Some("Player2"), "{name}");
        }
    }

    fn served(
        members: Members,
        read_timeout: Duration,
    ) -> (std::net::SocketAddr, Arc<Mutex<Members>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let members = Arc::new(Mutex::new(members));
        let serving = Arc::clone(&members);
        std::thread::spawn(move || {
            serve_with_read_timeout(listener, serving, Registry::default(), read_timeout).unwrap()
        });
        (addr, members)
    }

    fn connect(addr: std::net::SocketAddr) -> TcpStream {
        let stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
    }

    /// A frame as it came off a client socket.
    #[derive(Debug, PartialEq, Eq)]
    struct Frame {
        version: u8,
        kind: u8,
        payload: Vec<u8>,
    }

    fn read_one(stream: &mut TcpStream) -> Frame {
        let mut header = [0; HEADER_LEN];
        stream.read_exact(&mut header).unwrap();
        let mut payload = vec![0; u32::from_be_bytes(header[2..6].try_into().unwrap()) as usize];
        stream.read_exact(&mut payload).unwrap();
        Frame {
            version: header[0],
            kind: header[1],
            payload,
        }
    }

    #[test]
    fn a_frame_of_another_version_gets_a_reject_and_the_connection_closes() {
        let (addr, members) = served(serving("S"), READ_IDLE_TIMEOUT);
        let mut stream = connect(addr);
        // A version 1 JOIN: `type | length | xuid | gamertag`.
        stream
            .write_all(&[1, 0, 0, 0, 11, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, b'A'])
            .unwrap();
        assert_eq!(
            read_one(&mut stream),
            Frame {
                version: VERSION,
                kind: REJECT,
                payload: vec![1],
            }
        );
        assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0, "closed");
        assert!(lock(&members).snapshot().is_empty());
    }

    #[test]
    fn a_closed_connection_starts_the_grace_period_and_its_group_is_told() {
        let (addr, members) = served(serving("S"), READ_IDLE_TIMEOUT);
        let mut a = connect(addr);
        a.write_all(&join(Xuid(1), "A")).unwrap();
        assert_eq!(read_one(&mut a).kind, JOIN_ACK);
        {
            let mut b = connect(addr);
            b.write_all(&join(Xuid(2), "B")).unwrap();
            assert_eq!(read_one(&mut b).kind, JOIN_ACK);
            // B's join is pushed to A.
            let Frame { kind, payload, .. } = read_one(&mut a);
            assert_eq!(kind, ANNOUNCE);
            assert_eq!(roster_payload(&payload).consoles.len(), 2);
        }
        // B's connection closed: A is told, and B's place is held.
        let Frame { kind, payload, .. } = read_one(&mut a);
        assert_eq!(kind, ANNOUNCE);
        assert_eq!(
            roster_payload(&payload).consoles,
            [vec![player(Xuid(1), "A")]]
        );
        let reserved: Vec<_> = lock(&members)
            .reservations_snapshot()
            .iter()
            .map(|r| (r.xuid.0, r.group.get()))
            .collect();
        assert_eq!(reserved, [(2, 1)]);
        // A's own requests are still answered in order.
        a.write_all(&roster()).unwrap();
        assert_eq!(read_one(&mut a).kind, ROSTER_REPLY);
    }

    #[test]
    fn partial_frame_times_out_and_the_listener_still_serves_clients() {
        let (addr, _) = served(serving("S"), Duration::from_millis(50));
        let mut partial = connect(addr);
        partial.write_all(&[VERSION, JOIN, 0, 0, 0, 14, 1]).unwrap();
        assert_eq!(partial.read(&mut [0; 1]).unwrap(), 0);

        let mut stream = connect(addr);
        stream.write_all(&join(Xuid(1), "A")).unwrap();
        assert_eq!(read_one(&mut stream).kind, JOIN_ACK);
    }

    #[test]
    fn listener_survives_malformed_truncated_and_oversized_clients() {
        let (addr, _) = served(serving("S"), READ_IDLE_TIMEOUT);
        connect(addr).write_all(&request(JOIN, &[1, 0])).unwrap();
        connect(addr)
            .write_all(&[VERSION, JOIN, 0, 0, 0, 14, 1])
            .unwrap();
        {
            let mut header = vec![VERSION, JOIN];
            header.extend_from_slice(&((MAX_INBOUND_PAYLOAD as u32) + 1).to_be_bytes());
            connect(addr).write_all(&header).unwrap();
        }
        let mut stream = connect(addr);
        stream.write_all(&join(Xuid(1), "A")).unwrap();
        assert_eq!(read_one(&mut stream).kind, JOIN_ACK);
    }

    #[test]
    fn sbg_learns_every_player_of_a_console() {
        let ip: std::net::IpAddr = "192.0.2.77".parse().unwrap();
        let mut members = Members::default();
        members.note_console_players(ip, &[Xuid(10), Xuid(11), Xuid(12)]);
        assert_eq!(members.xuid_for_console(ip), Some(Xuid(10)));
        assert_eq!(members.players_of(ip), [Xuid(10), Xuid(11), Xuid(12)]);
        assert!(members.players_of("192.0.2.78".parse().unwrap()).is_empty());
    }
}

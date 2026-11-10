//! The all-time leaderboard: players ranked by lifetime score, listed only when their privacy
//! setting is public.

use std::{sync::Arc, time::Duration};

use super::{
    cache::Cached,
    ranking::{Gamertag, Standing, rank},
    user_storage,
};
use crate::storage::{BlobKind, Storage, StorageError};

/// Every player document is read to build the board.
const LIFETIME: Duration = Duration::from_secs(60);
/// A failed read is not retried for every request.
const FAILED_LIFETIME: Duration = Duration::from_secs(10);

/// The player documents could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unavailable;

#[derive(Default)]
pub struct Leaderboard {
    board: Cached<Result<Arc<Vec<Standing>>, Unavailable>>,
}

impl Leaderboard {
    pub fn board(&self, storage: &dyn Storage) -> Result<Arc<Vec<Standing>>, Unavailable> {
        self.board.get(|| match build(storage) {
            Ok(board) => (Ok(Arc::new(board)), LIFETIME),
            Err(error) => {
                tracing::warn!(%error, "leaderboard not read");
                (Err(Unavailable), FAILED_LIFETIME)
            }
        })
    }
}

/// Players without a public setting, a score, a valid gamertag or a readable document are left
/// out.
fn build(storage: &dyn Storage) -> Result<Vec<Standing>, StorageError> {
    let listed = storage
        .players_with_blob(BlobKind::UserStorage)?
        .into_iter()
        .filter_map(|(gamertag, document)| {
            let parsed = user_storage::parse(&document)?;
            parsed.privacy?.allows_callout().then_some(())?;
            Some((Gamertag::new(&gamertag)?, parsed.lifetime_score?))
        })
        .collect();
    Ok(rank(listed))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::{ids::Xuid, storage::MemoryStorage};

    pub fn document(score: &str, privacy: &str) -> Vec<u8> {
        format!("<XsorObject><LS type=\"float\">{score}</LS><PS>{privacy}</PS></XsorObject>")
            .into_bytes()
    }

    pub fn player(storage: &dyn Storage, xuid: Xuid, gamertag: &str, document: Option<Vec<u8>>) {
        storage
            .touch_player(xuid, Some(gamertag), crate::time::Utc::from_unix_secs(1))
            .unwrap();
        if let Some(document) = document {
            storage
                .put_blob(
                    BlobKind::UserStorage,
                    xuid,
                    &document,
                    crate::time::Utc::from_unix_secs(1),
                )
                .unwrap();
        }
    }

    #[test]
    fn only_public_players_with_a_score_are_ranked() {
        let storage = MemoryStorage::default();
        player(&storage, Xuid(1), "Third", Some(document("10.9", "Public")));
        player(&storage, Xuid(2), "First", Some(document("500", "Public")));
        player(
            &storage,
            Xuid(3),
            "Hidden",
            Some(document("900", "Private")),
        );
        player(&storage, Xuid(4), "Child", Some(document("900", "Child")));
        player(&storage, Xuid(5), "NoDocument", None);
        player(&storage, Xuid(6), "Broken", Some(b"<XsorObject>".to_vec()));
        player(&storage, Xuid(7), "SecondB", Some(document("20", "Public")));
        player(
            &storage,
            Xuid(8),
            "SecondA",
            Some(document("20.5", "Public")),
        );
        player(
            &storage,
            Xuid(10),
            "<script>",
            Some(document("800", "Public")),
        );
        player(
            &storage,
            Xuid(9),
            "Recorded",
            Some(include_bytes!("../../fixtures/user-storage-1149.xml").to_vec()),
        );
        let board: Vec<_> = build(&storage)
            .unwrap()
            .into_iter()
            .map(|standing| {
                (
                    standing.rank.0,
                    standing.gamertag.as_str().to_owned(),
                    standing.points.0,
                )
            })
            .collect();
        insta::assert_debug_snapshot!(board);
    }

    #[test]
    fn the_board_is_read_again_once_it_is_old() {
        let storage = MemoryStorage::default();
        let leaderboard = Leaderboard::default();
        assert!(leaderboard.board(&storage).unwrap().is_empty());
        player(&storage, Xuid(1), "Late", Some(document("1", "Public")));
        assert!(
            leaderboard.board(&storage).unwrap().is_empty(),
            "still cached"
        );
        leaderboard.board.expire();
        assert_eq!(leaderboard.board(&storage).unwrap().len(), 1);
    }
}

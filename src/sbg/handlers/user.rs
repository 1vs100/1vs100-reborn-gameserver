//! The player documents (UserStorage, UserStats).

use std::sync::Mutex;

use crate::{ids::Xuid, store::Store, sync::lock, xsor};

use crate::sbg::{
    key::{Key, ParsedKey, UserDoc},
    message::Reply,
};

/// Which player document; anything but UserStorage is UserStats, as the decoder routes only the
/// two user keys here.
fn user_kind(key: &ParsedKey) -> crate::storage::BlobKind {
    if key.kind == Key::User(UserDoc::Storage) {
        crate::storage::BlobKind::UserStorage
    } else {
        crate::storage::BlobKind::UserStats
    }
}

pub(in crate::sbg) fn user_get(key: &ParsedKey, xuid: Xuid, store: &Mutex<Store>) -> Reply {
    use crate::storage::BlobKind;
    let kind = user_kind(key);
    // The documents live in persistent storage (outliving restarts); do the I/O outside the
    // store lock.
    let storage = lock(store).storage();
    match storage.blob(kind, xuid) {
        Ok(Some(value)) => Reply::raw(value),
        Ok(None) if kind == BlobKind::UserStorage => {
            Reply::raw(xsor::user_storage_default_fixture())
        }
        Ok(None) => Reply::not_found(),
        Err(error) => {
            tracing::warn!(%error, xuid = xuid.0, ?kind, "user document read failed");
            if kind == BlobKind::UserStorage {
                Reply::raw(xsor::user_storage_default_fixture())
            } else {
                Reply::not_found()
            }
        }
    }
}

pub(in crate::sbg) fn user_set(
    key: &ParsedKey,
    xuid: Xuid,
    value: &[u8],
    store: &Mutex<Store>,
) -> Reply {
    let kind = user_kind(key);
    let storage = lock(store).storage();
    if let Err(error) = storage.put_blob(kind, xuid, value, crate::time::now()) {
        tracing::warn!(%error, xuid = xuid.0, ?kind, "user document write failed");
    }
    Reply::ok_empty()
}

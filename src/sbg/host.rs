//! Session host: only its GameState posts move the show on; first to talk holds it.

use std::{collections::HashMap, net::IpAddr, sync::Mutex};

use crate::sync::lock;

#[derive(Debug, Default)]
pub struct HostAuthority {
    host: Option<IpAddr>,
    /// Open SBG connections per console address.
    live: HashMap<IpAddr, usize>,
}

impl HostAuthority {
    /// Whether `peer` holds the host role.
    pub fn is_host(&self, peer: IpAddr) -> bool {
        self.host == Some(peer)
    }

    /// Grant the role when vacant: nobody holds it, or the holder is offline.
    pub fn claim_if_vacant(&mut self, peer: IpAddr) {
        match self.host {
            Some(host) if host == peer => {}
            Some(host) if self.live.get(&host).copied().unwrap_or(0) > 0 => {}
            previous => {
                tracing::info!(%peer, ?previous, "GameState host assigned");
                self.host = Some(peer);
            }
        }
    }

    /// Drop the host role; open connections still count.
    pub fn forget_roles(&mut self) {
        self.host = None;
    }

    fn opened(&mut self, peer: IpAddr) {
        *self.live.entry(peer).or_insert(0) += 1;
    }

    fn closed(&mut self, peer: IpAddr) {
        if let Some(n) = self.live.get_mut(&peer) {
            *n = n.saturating_sub(1);
            if *n == 0 && self.host == Some(peer) {
                tracing::info!(%peer, "GameState host's last SBG connection closed: the next console to ask takes over");
            }
        }
    }
}

/// Counts a console's open SBG connection for as long as it lives.
pub(crate) struct LiveConnection<'a> {
    authority: &'a Mutex<HostAuthority>,
    peer: IpAddr,
}

impl<'a> LiveConnection<'a> {
    pub(crate) fn open(authority: &'a Mutex<HostAuthority>, peer: IpAddr) -> Self {
        lock(authority).opened(peer);
        Self { authority, peer }
    }
}

impl Drop for LiveConnection<'_> {
    fn drop(&mut self) {
        lock(self.authority).closed(self.peer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 7));
    const B: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 8));

    #[test]
    fn reading_the_role_claims_nothing() {
        let auth = HostAuthority::default();
        assert!(!auth.is_host(A));
        assert_eq!(auth.host, None);
    }

    #[test]
    fn the_first_to_ask_is_host_while_connected_then_the_next_takes_over() {
        let authority = Mutex::new(HostAuthority::default());
        let a = LiveConnection::open(&authority, A);
        lock(&authority).claim_if_vacant(A);
        lock(&authority).claim_if_vacant(B);
        assert!(lock(&authority).is_host(A));
        assert!(!lock(&authority).is_host(B));
        drop(a);
        lock(&authority).claim_if_vacant(B);
        assert!(lock(&authority).is_host(B));
        assert!(!lock(&authority).is_host(A));
    }

    #[test]
    fn forgetting_roles_frees_the_host_and_keeps_live_connection_counts() {
        let mut auth = HostAuthority::default();
        auth.opened(A);
        auth.claim_if_vacant(A);

        auth.forget_roles();

        assert_eq!(auth.host, None);
        assert_eq!(auth.live.get(&A), Some(&1), "open connections still count");
    }
}

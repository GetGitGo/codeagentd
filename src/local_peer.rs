use std::net::SocketAddr;

/// True only for loopback peers (127.0.0.1 / ::1). LAN clients are rejected.
pub fn is_local_peer(peer: &SocketAddr) -> bool {
    peer.ip().is_loopback()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn loopback_is_local() {
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12345);
        assert!(is_local_peer(&peer));
    }

    #[test]
    fn lan_is_not_local() {
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5)), 12345);
        assert!(!is_local_peer(&peer));
    }
}

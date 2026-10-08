//! Verify the authenticated socket peer against local interface ownership before capture.
use std::net::{IpAddr, SocketAddr};
use uuid::Uuid;

#[derive(Clone, Copy)]
struct LocalAddress {
    ip: IpAddr,
    index: Option<u32>,
}

fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4),
        ip => ip,
    }
}
fn local_peer(remote: SocketAddr, addresses: &[LocalAddress]) -> bool {
    let ip = canonical_ip(remote.ip());
    if ip.is_loopback() {
        return true;
    }
    addresses.iter().any(|local| {
        if canonical_ip(local.ip) != ip {
            return false;
        }
        match remote {
            SocketAddr::V6(address) if address.ip().is_unicast_link_local() => {
                address.scope_id() != 0 && local.index == Some(address.scope_id())
            }
            _ => true,
        }
    })
}
fn endpoint_id(id: &str) -> String {
    // These are backend IDs, never display names. WASAPI endpoint GUID strings
    // are case insensitive; Core Audio UIDs/PipeWire object identities are exact.
    if id.starts_with("wasapi:") {
        id.to_ascii_lowercase()
    } else {
        id.to_owned()
    }
}
fn check(
    remote: SocketAddr,
    authenticated_hub: Uuid,
    capture: &str,
    output: &str,
    addresses: std::io::Result<Vec<LocalAddress>>,
) -> crate::Result<()> {
    if authenticated_hub.is_nil() || capture.is_empty() || output.is_empty() {
        return Err("local_feedback_check_unavailable".into());
    }
    let addresses = addresses.map_err(|_| "local_feedback_check_unavailable")?;
    if addresses.is_empty() {
        return Err("local_feedback_check_unavailable".into());
    }
    if let SocketAddr::V6(address) = remote
        && address.ip().is_unicast_link_local()
        && address.scope_id() == 0
        && addresses.iter().any(|local| local.ip == remote.ip())
    {
        return Err("local_feedback_check_unavailable".into());
    }
    if local_peer(remote, &addresses) && endpoint_id(capture) == endpoint_id(output) {
        return Err("local_feedback_loop".into());
    }
    Ok(())
}

pub fn before_capture(
    remote: SocketAddr,
    authenticated_hub: Uuid,
    capture: &str,
    output: &str,
) -> crate::Result<()> {
    let addresses = if_addrs::get_if_addrs().map(|addresses| {
        addresses
            .into_iter()
            .map(|address| LocalAddress {
                ip: address.ip(),
                index: address.index,
            })
            .collect()
    });
    check(remote, authenticated_hub, capture, output, addresses)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_loopback_lan_and_scoped_ipv6_share_the_endpoint_check() {
        let hub = Uuid::new_v4();
        let addresses = [
            LocalAddress {
                ip: "192.168.20.5".parse().unwrap(),
                index: Some(4),
            },
            LocalAddress {
                ip: "fe80::5".parse().unwrap(),
                index: Some(4),
            },
        ];
        for peer in [
            "127.0.0.1:7000",
            "192.168.20.5:7000",
            "[::1]:7000",
            "[::ffff:192.168.20.5]:7000",
            "[fe80::5%4]:7000",
        ] {
            assert_eq!(
                check(
                    peer.parse().unwrap(),
                    hub,
                    "wasapi:ENDPOINT",
                    "wasapi:endpoint",
                    Ok(addresses.to_vec())
                )
                .unwrap_err()
                .to_string(),
                "local_feedback_loop"
            );
        }
        for peer in ["192.168.20.6:7000", "[fe80::5%6]:7000"] {
            assert!(
                check(
                    peer.parse().unwrap(),
                    hub,
                    "wasapi:ENDPOINT",
                    "wasapi:ENDPOINT",
                    Ok(addresses.to_vec())
                )
                .is_ok()
            );
        }
        assert!(
            check(
                "192.168.20.5:7000".parse().unwrap(),
                hub,
                "coreaudio:one",
                "coreaudio:two",
                Ok(addresses.to_vec())
            )
            .is_ok()
        );
        assert_eq!(
            check(
                "127.0.0.1:7000".parse().unwrap(),
                hub,
                "one",
                "two",
                Err(std::io::Error::other("probe failed"))
            )
            .unwrap_err()
            .to_string(),
            "local_feedback_check_unavailable"
        );
    }
}

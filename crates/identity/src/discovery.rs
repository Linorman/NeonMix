//! DNS-SD candidates retain every endpoint; names and TXT never confer trust.
use crate::Result;
use mdns_sd::{ResolvedService, ScopedIp, ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap},
    net::{SocketAddr, SocketAddrV6},
    time::{Duration, Instant},
};
use uuid::Uuid;
pub const SERVICE: &str = "_neonmix._tcp.local.";
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Candidate {
    pub instance: String,
    pub hub_id: Uuid,
    pub room_name: String,
    pub version: u16,
    pub capabilities: String,
    pub endpoints: Vec<Endpoint>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Endpoint {
    pub url: String,
    pub address: SocketAddr,
}
pub struct Publisher {
    daemon: ServiceDaemon,
    fullname: String,
}
impl Publisher {
    pub fn new(
        hub_id: Uuid,
        room: &str,
        listen: SocketAddr,
        certificate_sha256: &str,
    ) -> Result<Self> {
        if room.trim().is_empty()
            || room.len() > 128
            || room.chars().any(char::is_control)
            || listen.port() == 0
            || certificate_sha256.len() != 64
            || !certificate_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid discovery registration".into());
        }
        let daemon = ServiceDaemon::new()?;
        daemon.set_ip_check_interval(1)?;
        let props = HashMap::from([
            ("hub_id".to_string(), hub_id.to_string()),
            ("name".into(), room.to_string()),
            ("version".into(), "1".into()),
            ("caps".into(), "opus48-stereo,pair-v1".into()),
        ]);
        // A changed key claiming the same UUID must remain a separate candidate.
        // This suffix only separates DNS records; clients still verify the full pin.
        let label = format!("{}-{}", hub_id.simple(), &certificate_sha256[..12]);
        let host = format!("neonmix-{label}.local.");
        let addresses = if listen.ip().is_unspecified() {
            String::new()
        } else {
            listen.ip().to_string()
        };
        let mut service = ServiceInfo::new(
            SERVICE,
            &format!("NeonMix-{label}"),
            &host,
            addresses.as_str(),
            listen.port(),
            props,
        )?;
        if listen.ip().is_unspecified() {
            service = service.enable_addr_auto();
        }
        if let SocketAddr::V6(address) = listen
            && address.scope_id() != 0
        {
            service.set_interfaces(vec![mdns_sd::IfKind::IndexV6(address.scope_id())]);
        } else if !listen.ip().is_unspecified() {
            service.set_interfaces(vec![mdns_sd::IfKind::Addr(listen.ip())]);
        } else {
            // Windows IPv6 wildcard listeners are IPv6-only by default.
            service.set_interfaces(vec![if listen.is_ipv4() {
                mdns_sd::IfKind::IPv4
            } else {
                mdns_sd::IfKind::IPv6
            }]);
        }
        let fullname = service.get_fullname().to_string();
        if let Err(error) = daemon.register(service) {
            let _ = daemon.shutdown();
            return Err(error.into());
        }
        Ok(Self { daemon, fullname })
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        if let Ok(done) = self.daemon.unregister(&self.fullname) {
            let _ = done.recv_timeout(Duration::from_secs(1));
        }
        if let Ok(done) = self.daemon.shutdown() {
            let _ = done.recv_timeout(Duration::from_secs(1));
        }
    }
}
fn candidate(info: &ResolvedService) -> Option<Candidate> {
    let version = info.get_property_val_str("version")?.parse().ok()?;
    if version != 1 || info.port == 0 {
        return None;
    }
    let hub_id = info.get_property_val_str("hub_id")?.parse().ok()?;
    let room_name = info.get_property_val_str("name")?;
    if room_name.trim().is_empty()
        || room_name.len() > 128
        || room_name.chars().any(char::is_control)
    {
        return None;
    }
    let mut endpoints: Vec<_> = info
        .addresses
        .iter()
        .filter_map(|address| match address {
            ScopedIp::V4(ip) => Some(Endpoint {
                url: format!("https://{}:{}", ip.addr(), info.port),
                address: SocketAddr::new((*ip.addr()).into(), info.port),
            }),
            ScopedIp::V6(ip) => {
                let scoped = ip.addr().is_unicast_link_local();
                if scoped && ip.scope_id().index == 0 {
                    return None;
                }
                let url = if scoped {
                    format!("https://neonmix-{}.local:{}", hub_id, info.port)
                } else {
                    format!("https://[{}]:{}", ip.addr(), info.port)
                };
                Some(Endpoint {
                    url,
                    address: SocketAddrV6::new(
                        *ip.addr(),
                        info.port,
                        0,
                        if scoped { ip.scope_id().index } else { 0 },
                    )
                    .into(),
                })
            }
            _ => None,
        })
        .collect();
    endpoints.sort();
    endpoints.dedup();
    Some(Candidate {
        instance: info.fullname.clone(),
        hub_id,
        room_name: room_name.into(),
        version,
        capabilities: info
            .get_property_val_str("caps")
            .unwrap_or("")
            .chars()
            .take(128)
            .collect(),
        endpoints,
    })
}
pub fn browse(
    seconds: u32,
    mut changed: impl FnMut(&str, &Candidate) -> Result<()>,
) -> Result<Vec<Candidate>> {
    if !(1..=60).contains(&seconds) {
        return Err("discovery seconds must be 1..60".into());
    }
    let daemon = ServiceDaemon::new()?;
    let result = (|| {
        let receiver = daemon.browse(SERVICE)?;
        let until = Instant::now() + Duration::from_secs(u64::from(seconds));
        let mut candidates: BTreeMap<String, Candidate> = BTreeMap::new();
        let mut verify_at = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until {
            if Instant::now() >= verify_at {
                for name in candidates.keys() {
                    daemon.verify(name.clone(), Duration::from_secs(2))?;
                }
                verify_at = Instant::now() + Duration::from_secs(3);
            }
            match receiver.recv_timeout(
                until
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(200)),
            ) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    if let Some(c) = candidate(&info)
                        && candidates.get(&c.instance) != Some(&c)
                        && (candidates.len() < 128 || candidates.contains_key(&c.instance))
                    {
                        changed("resolved", &c)?;
                        candidates.insert(c.instance.clone(), c);
                    }
                }
                Ok(ServiceEvent::ServiceRemoved(_, name)) => {
                    if let Some(c) = candidates.remove(&name) {
                        changed("removed", &c)?;
                    }
                }
                _ => {}
            }
        }
        Ok(candidates.into_values().collect())
    })();
    let _ = daemon.stop_browse(SERVICE);
    if let Ok(done) = daemon.shutdown() {
        let _ = done.recv_timeout(Duration::from_secs(1));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_distinct_candidates_and_all_usable_addresses_survive() {
        let id = Uuid::new_v4();
        let info = ServiceInfo::new(
            SERVICE,
            "first",
            "first.local.",
            "127.0.0.1,::1",
            7443,
            HashMap::from([
                ("hub_id".into(), id.to_string()),
                ("name".into(), "同名房间".into()),
                ("version".into(), "1".into()),
            ]),
        )
        .unwrap()
        .as_resolved_service();
        let a = candidate(&info).unwrap();
        assert_eq!(a.endpoints.len(), 2);
        let mut other = info.clone();
        other.fullname = "second._neonmix._tcp.local.".into();
        let b = candidate(&other).unwrap();
        assert_eq!(a.room_name, b.room_name);
        assert_ne!(a.instance, b.instance);
        let mut changed = info;
        changed.port = 8443;
        assert_ne!(candidate(&changed).unwrap(), a);
    }
    #[test]
    fn incompatible_or_incomplete_records_do_not_become_candidates() {
        let mut info = ServiceInfo::new(
            SERVICE,
            "old",
            "old.local.",
            "127.0.0.1",
            7443,
            HashMap::from([
                ("hub_id".into(), Uuid::new_v4().to_string()),
                ("name".into(), "old".into()),
                ("version".into(), "2".into()),
            ]),
        )
        .unwrap()
        .as_resolved_service();
        assert!(candidate(&info).is_none());
        info.port = 0;
        assert!(candidate(&info).is_none());
    }
}

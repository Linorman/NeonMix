//! AirPlay audio discovery has one owner: the Hub, after its worker is ready.
use crate::Result;
use mdns_sd_scoped::{ServiceDaemon, ServiceInfo, UnregisterStatus};
use std::{collections::HashMap, net::SocketAddr, time::Duration};
use uuid::Uuid;

pub struct SpeakerPublisher {
    daemon: ServiceDaemon,
    services: Vec<ServiceInfo>,
    published: bool,
}
impl SpeakerPublisher {
    pub fn new(
        hub: Uuid,
        name: &str,
        listen: SocketAddr,
        key: &str,
        features: &str,
    ) -> Result<Self> {
        let mut owner = Self::prepare(hub, name, listen, key, features)?;
        owner.publish()?;
        Ok(owner)
    }

    /// Prepare a hidden receiver. Pass the durable receiver UUID, not a shared
    /// room UUID, so withdrawing this entry cannot remove another entry's host.
    pub fn prepare(
        hub: Uuid,
        name: &str,
        listen: SocketAddr,
        key: &str,
        features: &str,
    ) -> Result<Self> {
        if name.trim().is_empty()
            || name.len() > 50
            || name.chars().any(char::is_control)
            || key.len() != 64
            || !key.bytes().all(|b| b.is_ascii_hexdigit())
            || listen.port() == 0
            || (listen.is_ipv6() && !listen.ip().is_unspecified())
            || features.len() > 32
        {
            return Err("invalid Speaker registration".into());
        }
        let daemon = ServiceDaemon::new()?;
        // The isolated mdns-sd fork binds link-local AAAA records to the
        // actual publishing interface, including when IPv4 carries mDNS.
        daemon.set_ip_check_interval(1)?;
        let host = format!("neonmix-speaker-{}.local.", hub.simple());
        let id = receiver_id(hub);
        let colon_id = id
            .as_bytes()
            .chunks(2)
            .map(|b| std::str::from_utf8(b).unwrap_or(""))
            .collect::<Vec<_>>()
            .join(":");
        let common = HashMap::from([
            ("pk".to_string(), key.to_string()),
            ("model".into(), "AppleTV3,2".into()),
            ("srcvers".into(), "220.68".into()),
            ("pi".into(), hub.to_string()),
        ]);
        let mut raop = common.clone();
        for (k, v) in [
            ("ch", "2"),
            ("cn", "0,1,2,3"),
            ("et", "0,3,5"),
            ("sr", "44100"),
            ("ss", "16"),
            ("tp", "UDP"),
            ("da", "true"),
            ("sv", "false"),
            ("vv", "2"),
            ("md", "0"),
            ("txtvers", "1"),
            ("vn", "65537"),
            ("vs", "220.68"),
            ("sf", "0x4"),
            ("pw", "true"),
            ("am", "AppleTV3,2"),
        ] {
            raop.insert(k.into(), v.into());
        }
        raop.insert("ft".into(), features.into());
        let mut airplay = common;
        for (k, v) in [
            ("deviceid", colon_id.as_str()),
            ("features", features),
            ("flags", "0x4"),
            ("pw", "true"),
            ("vv", "2"),
        ] {
            airplay.insert(k.into(), v.into());
        }
        let mut owner = Self {
            daemon,
            services: Vec::new(),
            published: false,
        };
        for (service, instance, props) in [
            ("_raop._tcp.local.", format!("{id}@{name}"), raop),
            ("_airplay._tcp.local.", name.to_string(), airplay),
        ] {
            let address = if listen.ip().is_unspecified() {
                String::new()
            } else {
                listen.ip().to_string()
            };
            let mut info = ServiceInfo::new(
                service,
                &instance,
                &host,
                address.as_str(),
                listen.port(),
                props,
            )?;
            if listen.ip().is_unspecified() {
                info = info.enable_addr_auto();
            } else {
                info.set_interfaces(vec![mdns_sd_scoped::IfKind::Addr(listen.ip())]);
            }
            owner.services.push(info);
        }
        Ok(owner)
    }

    /// Submit both records using the original identity. This acknowledges the
    /// daemon queue; multicast probing/announcement is asynchronous.
    pub fn publish(&mut self) -> Result<()> {
        if self.published {
            return Ok(());
        }
        for info in &self.services {
            if let Err(error) = self.daemon.register(info.clone()) {
                // A partial service pair must not remain visible.
                let _ = self.withdraw();
                return Err(error.into());
            }
        }
        self.published = true;
        Ok(())
    }

    /// Withdraw both records and wait for daemon confirmation, without stopping
    /// the worker or discovery daemon. Call only from a non-realtime owner task.
    /// On failure the caller must keep the entry in discovery error state.
    pub fn withdraw(&mut self) -> Result<()> {
        let mut first_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;
        // Queue both withdrawals before waiting, so a timeout on one service
        // never leaves the other service deliberately registered.
        let pending = self
            .services
            .iter()
            .map(|info| self.daemon.unregister(info.get_fullname()))
            .collect::<Vec<_>>();
        for result in pending {
            let result: Result<()> = match result {
                Ok(done) => match done.recv_timeout(Duration::from_secs(1)) {
                    Ok(UnregisterStatus::OK | UnregisterStatus::NotFound) => Ok(()),
                    Err(error) => Err(error.into()),
                },
                Err(error) => Err(error.into()),
            };
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        self.published = false;
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// True when both registrations have been submitted. Peer visibility is
    /// asynchronous and must be observed separately for Apple UI acceptance.
    pub fn is_published(&self) -> bool {
        self.published
    }
}
/// Locally administered receiver identity derives from the durable Hub UUID, never a NIC.
pub fn receiver_id(hub: Uuid) -> String {
    let mut bytes = [0u8; 6];
    bytes.copy_from_slice(&hub.as_bytes()[..6]);
    bytes[0] = (bytes[0] | 2) & !1;
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}
impl Drop for SpeakerPublisher {
    fn drop(&mut self) {
        let _ = self.withdraw();
        if let Ok(done) = self.daemon.shutdown() {
            let _ = done.recv_timeout(Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_pair_can_withdraw_and_republish_without_changing_identity() {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let daemon = ServiceDaemon::new_with_port(port).unwrap();
        daemon
            .disable_interface(mdns_sd_scoped::IfKind::All)
            .unwrap();
        let services = ["_raop._tcp.local.", "_airplay._tcp.local."]
            .into_iter()
            .map(|kind| {
                ServiceInfo::new(
                    kind,
                    "test-entry",
                    "test-entry.local.",
                    "127.0.0.1",
                    7000,
                    None,
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        let names = services
            .iter()
            .map(|info| info.get_fullname().to_string())
            .collect::<Vec<_>>();
        let mut publisher = SpeakerPublisher {
            daemon,
            services,
            published: false,
        };
        assert!(!publisher.is_published());
        publisher.publish().unwrap();
        publisher.publish().unwrap();
        assert!(publisher.is_published());
        publisher.withdraw().unwrap();
        publisher.withdraw().unwrap();
        assert!(!publisher.is_published());
        publisher.publish().unwrap();
        assert!(publisher.is_published());
        assert_eq!(
            names,
            publisher
                .services
                .iter()
                .map(|info| info.get_fullname().to_string())
                .collect::<Vec<_>>()
        );
        // Confirm both re-registrations actually reached the daemon.
        for info in &publisher.services {
            assert!(matches!(
                publisher
                    .daemon
                    .unregister(info.get_fullname())
                    .unwrap()
                    .recv_timeout(Duration::from_secs(1))
                    .unwrap(),
                UnregisterStatus::OK
            ));
        }
        publisher.withdraw().unwrap();
    }
}

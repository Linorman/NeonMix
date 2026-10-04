# mdns-sd 0.21.4 / NeonMix Speaker scope / multi-receiver revision 2

This isolated fork is used by `crates/identity/src/speaker.rs` through the
`mdns_sd_scoped` Cargo alias. Native NeonMix discovery keeps the unmodified
registry `mdns-sd` dependency. Both copies retain upstream public APIs.

IPv6 address publication changes so that link-local AAAA records
are emitted only when the address belongs to the actual outgoing interface.
Upstream subnet matching is retained for global/ULA addresses and for remote
querier network matching (`valid_ip_on_intf`). This prevents loopback, VPN or
another Ethernet link's `fe80::/64` address acquiring the Wi-Fi scope. Every
upstream AAAA publication path uses the patched address selector, including
AAAA answers transported over IPv4 mDNS. Interface address refreshes use the
current interface address set. No platform-specific networking API is added.

A second narrow change prevents a reflected registered local address from
renaming the publisher while another NIC on the same LAN is still probing.
The address must belong to a local interface and be explicitly registered
under the probed hostname. Unrelated peer address and SRV/TXT conflicts keep
upstream handling. Responses still enter the normal discovery/cache path.

Original archive checksum, licensing and patch checksum are recorded in
`../mdns-sd-scoped-provenance.json`. The exact diff is
`../patches/mdns-sd-0.21.4-ipv6-scope.patch`. The vendor source retains only the
library and required manifests/licenses/readme; manifest entries for omitted
upstream examples and integration tests are removed. Four extra unit tests
cover multiple links, loopback, global/ULA behavior, remote link-local queriers
and refreshed interface addresses. A reflected local address is accepted
without renaming while an unrelated peer still triggers the upstream conflict.
A third change tags delayed goodbye retransmissions with their service and hostname. Re-registering either service of the same receiver cancels its stale goodbye packets; other receivers retain their retries. The deterministic rapid-republish unit test exercises reversal before the 120ms retry without sleeping. Formatting-only changes may also appear in the exact patch.

macOS validation:

```sh
tools/dev cargo check -p neonmix-identity
tools/dev cargo test --manifest-path vendor/mdns-sd-scoped/Cargo.toml --lib
tools/dev python3 tools/airplay_ipv6_scope_probe.py
```

The independent fixture verifies actual native DNS-SD address records plus
AAAA replies sent over IPv4 multicast, attributed by source-interface IP.
Same-LAN NICs can receive each other multicast; native callback scope is the
receiving scope and does not identify the outgoing interface. The fixture
rejects loopback/VPN LLA callbacks and checks each wire reply against its
source interface, then removes its temporary source/build directory. It never starts a worker or
modifies the live manual receiver. Passing it proves discovery scope only,
not Apple-device audio playback. The test requires `en0` and at least one
other LAN interface with IPv6 link-local addresses. Windows/Linux keep the
same Rust path but have not been runtime tested here.

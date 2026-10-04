# Vendor source and local revisions

## CPAL 0.18.2 / NeonMix native I/O revision 2

Source: crates.io CPAL 0.18.2, Apache-2.0; original LICENSE and upstream documentation are retained in `cpal/`. `cpal-provenance.json` records the original archive SHA256 and patch SHA256.

The reviewable delta is `patches/cpal-0.18.2-native-position.patch`:

- expose an optional raw device-frame coordinate: WASAPI IAudioClock, valid Core Audio sampleTime, PipeWire graph ticks scaled by its rational clock rate;
- monitor explicitly selected PipeWire node removal, even if the stream only changes to Paused; retain a healthy empty device list while preserving timeout/connection errors;
- expose an endpoint predicate so NeonMix excludes transient application stream nodes;
- honor SPA chunk offsets, wrapping, bounds, frame alignment and EMPTY/CORRUPTED flags. Input scratch is allocated once at stream creation for at most 8192 frames; processing does not allocate or resize it.

Device-clock origins differ. HAL sampleTime is an I/O-cycle coordinate and PipeWire ticks are graph-clock coordinates, not measurements of analog playback latency. Missing/invalid device positions remain None. The original estimated presentation timestamp remains separate.

Cargo builds this source directly; no dependency cache is patched. Upgrades require reviewing/removing the delta, rerunning three-target checks and native probes. Exact range-copy code is exercised by `crates/audio-io/tests/pipewire_buffer.rs` on every host. Source hashes and the Apache license are included in build evidence. Changes do not introduce another native backend or C/C++ product code.

Linux scheduling revision 1 additionally enables CPAL realtime promotion, tries an already-authorized native `RLIMIT_RTPRIO` grant before RTKit, and selects PipeWire `client-rt.conf` for the data loop (an explicit `PIPEWIRE_CONFIG_NAME` wins). See `patches/cpal-0.18.2-linux-scheduling.patch`, applied after the native-position patch. The helper remains outside callbacks. `audio_thread_priority` 0.35.1 is MPL-2.0 and is locked in Cargo.lock; distribution must retain its license/notice. No audio buffer limits are enlarged.

## tympan-aspl 0.1.0 / NeonMix HAL

`tympan-aspl/` is the MIT OR Apache-2.0 Rust CFPlugIn implementation used by the macOS virtual output. The original archive SHA256 and the ABI, output controls, multi-client lifecycle and realtime clock deltas are recorded in `patches/tympan-aspl-0.1.0-realtime-clock.md`. Both upstream license texts are retained. The project plug-in adds sample-time tags so a ring wrap cannot replay stale PCM.

## mdns-sd 0.21.4 / NeonMix Speaker scope / multi-receiver revision 2

`mdns-sd-scoped/` is an isolated Apache-2.0 OR MIT fork used only by AirPlay
Speaker discovery. Native NeonMix discovery retains registry `mdns-sd`. The
IPv6 publication selector limits link-local AAAA records to addresses owned by
the outgoing interface; global/ULA and remote-query subnet matching retain
upstream behavior. Registered local address reflections on the same LAN do
not rename the publisher on another NIC; remote and service conflicts retain
upstream behavior. Original checksum, retained files and patch checksum are in
`mdns-sd-scoped-provenance.json`; the minimal delta is
`patches/mdns-sd-0.21.4-ipv6-scope.patch`. Both license texts are retained.

See `mdns-sd-scoped/README-NEONMIX.md` for macOS unit-test and actual DNS-SD
fixture commands and their scope. Windows/Linux runtime tests have not been
run for this revision. No dependency cache is patched.

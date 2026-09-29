# CPAL 0.18.2 / NeonMix native I/O revision 2

Source: crates.io CPAL 0.18.2, Apache-2.0; original LICENSE and upstream documentation are retained in `cpal/`. `cpal-provenance.json` records the original archive SHA256 and patch SHA256.

The reviewable delta is `patches/cpal-0.18.2-native-position.patch`:

- expose an optional raw device-frame coordinate: WASAPI IAudioClock, valid Core Audio sampleTime, PipeWire graph ticks scaled by its rational clock rate;
- monitor explicitly selected PipeWire node removal, even if the stream only changes to Paused; retain a healthy empty device list while preserving timeout/connection errors;
- expose an endpoint predicate so NeonMix excludes transient application stream nodes;
- honor SPA chunk offsets, wrapping, bounds, frame alignment and EMPTY/CORRUPTED flags. Input scratch is allocated once at stream creation for at most 8192 frames; processing does not allocate or resize it.

Device-clock origins differ. HAL sampleTime is an I/O-cycle coordinate and PipeWire ticks are graph-clock coordinates, not measurements of analog playback latency. Missing/invalid device positions remain None. The original estimated presentation timestamp remains separate.

Cargo builds this source directly; no dependency cache is patched. Upgrades require reviewing/removing the delta, rerunning three-target checks and native probes. Exact range-copy code is exercised by `crates/audio-io/tests/pipewire_buffer.rs` on every host. Source hashes and the Apache license are included in build evidence. Changes do not introduce another native backend or C/C++ product code.

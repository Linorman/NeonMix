# NeonMix AirPlay audio worker

Audio worker based on checksum-locked UxPlay v1.73.7. macOS is the validated developer platform; Windows/MinGW and Linux have source/build adapters but remain unvalidated as complete worker builds. The worker receives authenticated classic/NTP UDP audio, decodes PCM, ALAC or AAC to 48 kHz stereo float32, and sends at most 480 frames per private media IPC packet. It has no sound device, renderer, video/HLS implementation or discovery registration. The Hub owns the Mixer/output, discovery, file credentials and authorization context.

## Build and verification

From the repository root:

```sh
tools/dev python3 tools/prepare_airplay.py
tools/dev cmake --build .local/airplay/build --target neonmix-airplay-probe-crypto neonmix-airplay-audio-probe --parallel 8
tools/dev python3 apps/airplay-worker/probe.py
tools/dev python3 apps/airplay-worker/pairing_probe.py
tools/dev .local/airplay/build/neonmix-airplay-audio-probe
tools/dev cmake --build .local/airplay/build --target neonmix-airplay-identity-probe
tools/dev python3 tools/airplay_identity_probe.py --worker .local/airplay/build/neonmix-airplay-worker
```

All archives, extracted headers, protocol patches, libraries, plugin registry and artifacts are inside `.local/airplay`. The existing `.local/gstreamer/prefix` runtime is reused. Only system SDK/compiler tools are read from outside the project. The prepare script verifies each archive SHA-256 before extraction; it uses a maintained patch rather than the upstream top-level CMake renderer build. No Rust `Cargo.toml` exists for this worker.

To prepare a separate build while an existing Hub uses `target/debug` or `target/release`, pass `--no-copy`. This retains the new executable in its CMake build directory and leaves both Hub-side copies untouched. `--build-dir` chooses another project-local output directory.

Source preparation is independent of the host SDK:

```sh
tools/dev python3 tools/prepare_airplay.py --sources-only
```

The script locks the same UxPlay/libplist archives and patch on every platform. Only macOS downloads/extracts the pinned macOS development package. Windows/Linux require an already extracted project-local GStreamer development/runtime prefix; the script never installs packages. Configure a native build using `--gst-prefix .local/PREFIX`, optional `--sdk-prefix .local/SDK`, and `--build-dir .local/airplay/build-PLATFORM`. Windows selects Ninja and requires a MinGW compiler with pthread support; MSVC is explicitly rejected for this worker, independently of the Rust Hub compiler. The supplied SDK/import libraries must match the toolchain and architecture; reuse of the project's MSVC GStreamer package with MinGW has not been validated. Put its runtime `bin` directory on PATH for DLL loading/scanning. Linux runtime/scanner paths must likewise point to the extracted prefix.

For Linux pkg-config layouts, CMake can instead use the environment established by `tools/dev` (including the project's extracted multiarch development libraries):

```sh
tools/dev cmake -S apps/airplay-worker -B .local/airplay/build-linux \
  -DCMAKE_BUILD_TYPE=Release
tools/dev cmake --build .local/airplay/build-linux --parallel 8
```

Set `NEONMIX_AUDIO_PLUGIN_DIR` to a private directory containing only `coreelements`, `app`, `audioconvert`, `audioresample` and `libav`; set `NEONMIX_AUDIO_REGISTRY` to a project-local cache. The prepare script creates this plugin selection automatically, using symlinks on POSIX and copies on Windows. Windows uses the project's `tools/dev.ps1` entry, plus already available Python, patch, CMake, Ninja and MinGW tools; this entry does not provision them. An illustrative native Windows configuration, with MinGW `gcc`/`g++` on PATH and an already extracted matching SDK, is:

```powershell
tools/dev.ps1 python tools/prepare_airplay.py --sources-only
tools/dev.ps1 python tools/prepare_airplay.py --gst-prefix .local/airplay-windows/prefix --build-dir .local/airplay/build-windows --no-copy --cmake-arg=-DCMAKE_C_COMPILER=gcc --cmake-arg=-DCMAKE_CXX_COMPILER=g++
```

Cross builds should use `--sources-only` then a CMake toolchain file and independent target SDK; the native prepare command does not assemble a cross SDK.

CMake exposes `NEONMIX_GSTREAMER_PREFIX` and `NEONMIX_GSTREAMER_SDK`, detects libplist date/string/`struct tm` features for the target, and defines `LIBPLIST_STATIC` for all static-library consumers. A macOS `config.h` is never reused on Windows. Use a separate CMake directory for each host/architecture.

On 2026-10-01, macOS worker/crypto/audio targets compiled and the digital protocol/PCM/ALAC/AAC probes passed. The platform adapter also passed macOS-hosted syntax checks against project-local Windows x64 MinGW headers and Linux x64 sysroot headers, with read-only system libc++ headers and a project-local target configuration overlay. These checks cover `platform.h` only; they do not establish a linked Windows/Linux worker, compatible dependency ABI, private-pipe/DACL behavior, native audio, Apple-client compatibility, or distribution readiness. No Windows/Linux executable was run.

`probe.py` verifies a synthetic sender: wrong/correct SRP PIN, Ed25519 key binding, signed pair-verify, Hub admission, NTP timing, encrypted UDP RTP, source RTP wrap, PCM IPC, paired reconnect, revocation and refusal of a second connection. Malformed SRP and video/HLS/mirror/TEARDOWN requests are also exercised. `audio_probe.cpp` encodes and decodes real GStreamer PCM/ALAC/AAC-LC samples and checks normalization/metadata without opening a sound device. Neither probe proves iPhone/iPad/Mac interoperability, protected-content behavior or source-video/speaker synchronization.

`pairing_probe.py` additionally checks receiver authentication: the SRP-GCM response binds the published long-term key, and the receiver's encrypted pair-verify signature must match the cached key. It persists a synthetic client record, restarts the worker with the same receiver key, and verifies PIN-free reconnect. Missing records require a fresh correct PIN; blocked keys remain rejected; copying a client list cannot make a replacement receiver key authenticate as the old receiver. Temporary files are removed, and its JSON report excludes keys and PINs. This checks protocol and persistence seams, not the iPhone's native prompt UI.

## Process contract

Launch `.local/airplay/build/neonmix-airplay-worker` without secret command arguments. Send one JSON object on stdin, followed by LF:

```json
{"media_address":"127.0.0.1:PORT","ipc_token":"64 lowercase hex characters","device_id":"12 hex characters","receiver_uuid":"stable Hub UUID","keyfile":"project-private 0600 PEM Ed25519 file","name":"NeonMix","pin":"1234","rtsp_port":0,"session_id":1,"stream_id":1,"stream_epoch":1,"format_epoch":1,"mapping_id":1,"known_client_keys":[],"blocked_client_keys":[],"pairing_allowed":true}
```

The Hub generates/stores the PEM identity in receiver.json’s adjacent .credentials file store and supplies a temporary current-user regular file with mode 0600 on POSIX. Windows requires a filesystem with persistent ACLs, the current user's file ownership, and a protected DACL permitting only that user or OWNER_RIGHTS; reparse points and alternate data streams are refused. The JSON path is UTF-8; Win32 checks use wide paths, while pinned UxPlay uses the process ANSI file API. A path that cannot be converted without loss is rejected explicitly. Full Windows Unicode relocation remains unvalidated. Missing/corrupt/wrong-algorithm keys fail closed; the worker never generates, overwrites or stores keys. The Hub removes the temporary file after exit. A PIN expires after ten minutes and supports no more than five fresh SRP attempts per worker lifetime.

The identity loader accepts the unencrypted Ed25519 PKCS#8 v1 envelope from OpenSSL and v2 envelopes from ring (including its legacy explicit public-key tag). A v2 embedded public key must match the seed. Local CRT file reads feed an OpenSSL memory BIO, avoiding a MinGW `FILE*` crossing into MSVC OpenSSL. Invalid keys propagate a normal initialization failure. The identity probe checks unchanged public keys/signatures, malformed input rejection and full worker startup; macOS OpenSSL 3.0.9 compatibility testing does not replace Linux/Windows native verification.

Production media uses private Unix sockets on macOS/Linux and a protected current-user NamedPipe on Windows; TCP loopback remains available for standalone protocol fixtures. The Windows client verifies its parent PID, creation time, user SID and the pipe DACL before sending the token. Windows media writes use FILE_FLAG_OVERLAPPED with one completion event and one outstanding write, retaining a 250ms whole-packet budget. Stop/timeout cancels and drains the operation before releasing its buffer. The control pipe remains PIPE_NOWAIT. Native Windows throughput, stalled-reader, stop and peer-close tests are provided by `neonmix-airplay-pipe-probe` and `tools/windows_airplay_pipe_probe.py`; see `docs/WINDOWS-AIRPLAY-INVESTIGATION-20261007.md` for real-device status.

Control uses stdin/stdout JSONL, limited to 16 KiB per inbound line. Startup requires a complete LF-terminated configuration within five seconds. Outbound events are at most 4096 bytes and nonblocking; a blocked/closed consumer ends this worker. Windows requires private pipe handles, polls stdin with `PeekNamedPipe`, and sets stdout to immediate `PIPE_NOWAIT` writes; unsupported pipe configuration fails closed. This intentionally uses immediate failure rather than queued asynchronous I/O. [Microsoft documents that SetNamedPipeHandleState accepts CreatePipe handles](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-setnamedpipehandlestate). The separate media socket authenticates with the token followed by LF, then uses the 120-byte little-endian header in `crates/airplay-ipc` / `docs/AIRPLAY-IPC-CONTRACT.md`. Both IPC channels are private to the parent process. Media send timeout remains 250 ms on all platform adapters, and shutdown interrupts an outstanding media write before joining its writer thread.

Optional startup `protocol_trace: true` enables bounded `protocol {method,route,status}` events. Method and route are collapsed into fixed safe enums; headers, bodies, peer addresses and secret values are excluded. Additional `protocol_detail {stage,a_bytes,proof_bytes,reason}` events report only SRP proof lengths and fixed failure categories. Trace is disabled by default. SHA-1 SRP proof verification accepts native 20-byte M and legacy 64-byte padding; both validate the full 20-byte cryptographic proof, and incompatible public-key sizes are rejected.

Events use `type`: `ready {port,public_key,features}`; `pairing_pin {pin}`; `registered {client_public_key,device_id,name}`; `admit_request {request_id,client_public_key,device_id,name}`; `session_started {client_public_key,device_id,name}`; `session_ended`; `volume {volume_db}`; `flush`; `fatal {message}`. Public keys in ready are hex; client keys are base64. Features are numeric `0x481C5A00` (audio, redundancy, FairPlay, audio formats 1–3, legacy pairing and RAOP); video/photos/mirror/HLS/PTP/buffered-audio/MFi are absent. The current upstream PIN comparison profile advertises `flags/sf=0x4` and full `/info.statusFlags=0x44`, with `pw=true`; actual PIN/SRP/public-key authentication stays mandatory. `/info.pi` is the provided stable Hub receiver UUID. The `_raop` / `_airplay` profile requires actual Apple-client verification before it becomes a supported profile. The current protocol compatibility selector uses upstream `model/am=AppleTV3,2`; it does not add Apple TV video capabilities or claim to be Apple hardware. A previous `NeonMixSpeaker1,1` profile reached OPTIONS/FairPlay setup from macOS system audio but selected textual SDP ANNOUNCE, which this worker does not currently implement. The model-only change is an A/B compatibility experiment with the same strict audio/PIN admission gates.

Commands use `type`: `admit {request_id,allowed}`; `grant {session_id,stream_id,stream_epoch,format_epoch,mapping_id}`; `disconnect`; `revoke`; `allow`; `stop`. Each `admit` must match the current request ID and arrive within 500 ms. Revoke cancels pending admission and blocks the last verified key. Disconnect/revoke require a new grant and allow before reuse. The five control context IDs must be in `1..2^53-1`; the worker rejects larger/negative values to prevent libplist JSON integer saturation from silently changing an identity. The binary IPC fields remain `u64`. Session start and flush stop media until the Hub grants a fresh complete context. Old epochs are always rejected again by the Hub.

Media contains normalized PCM while keeping the original negotiated source rate (this prototype admits only 44.1 kHz source formats), extended RTP sample position and upstream `ntp_time_local` Unix nanoseconds. Decoder/resampler PTS are retained. Mapping uncertainty uses the selected NTP measurement's delay/2 + dispersion and 15 ppm growth with age; no timestamped media is emitted before an NTP anchor exists. This is a protocol estimate, not a measured guarantee of acoustic synchronization. Compressed appsrc queue <=256 KiB, appsink <=8 buffers, metadata <=1024 entries, pending IPC <=200 chunks; a stalled 250 ms IPC write or over-capacity queue ends the worker. There is no fixed wait until presentation in this process; Hub schedules PCM against output feedback.

The Hub should set `GST_PLUGIN_SYSTEM_PATH_1_0` to `.local/airplay/plugins`, `GST_REGISTRY` to its private project state registry, and reuse the project runtime library/scanner paths from `tools/dev`. Explicit parent environment values are respected; project-local development defaults are compiled into the worker. Production relocation/packaging and Windows/Linux builds are not validated here.

See `vendor/airplay/README.md` for the GPL/distribution boundary.

## Control IPC v2

Startup requires `control_version: 2` and a positive `worker_generation` (at most `2^53-1`); `ready` echoes both. Startup and ready also require `pcm_version: 2`; PCM v2 adds a normalized 48 kHz coordinate in its 120-byte header. There is no silent v1 control fallback.

Admission and grants address `worker_generation`, real RTSP `connection_id`, and `request_id`. A newly accepted `session_started` reports zero session/epoch until the Hub grants identity. `grant_applied` echoes all five media identifiers plus the process/connection/request tuple before this worker enables PCM. The Hub must still allow for independent control/media transport ordering. Format, volume, flush and end observations carry the complete active provenance; flush reports the old epoch. New grants must advance that epoch within the same session.

`disconnect` and `revoke` require the current generation, connection, session and epoch. Old commands and close callbacks cannot terminate a successor. Targeted closes run on the RTSP owner thread. `trust_update` replaces this worker's known/blocked key snapshot and pairing policy, scoped by generation. `allow` is generation-scoped; `stop` is reserved for the process owner. Duplicate initial key/timing SETUP returns 455; stream SETUP retains the established owner and triggers a media epoch reset when needed.

`neonmix-airplay-audio-probe` covers stale generations, connections, requests and epochs, directed revocation, and delayed old-owner close in addition to PCM/ALAC/AAC decoding. `probe.py` covers v2 startup/ready, real socket provenance, grant acknowledgment, repeated initial SETUP, encrypted RTP and trust updates on macOS.

The Hub supplies `trust_generation`, `pairing_remaining_ms` (0..600000) and cumulative `pairing_attempts` (0..6) on startup. Worker restart cannot create a fresh pairing window or attempt budget. `pairing_attempt` returns the cumulative count with `worker_generation`; the sixth challenge is rejected. `pairing_pin` also carries `worker_generation`.

Trust snapshots require a strictly newer `trust_generation`, cancel pending admission, and fence previously started SRP exchanges. `registered`, `admit_request` and `session_started` carry the captured trust generation for Hub-side validation. Repairing a source cannot turn a delayed old registration or a socket's old PIN verification into new trust.

First-pairing challenges are synchronously authorized by the Hub: `pairing_request` identifies generation, trust generation, connection and a separate `pairing_request_id`; `pairing_admit` echoes that tuple with `allowed` and the Hub-counted `attempts`. No SRP challenge begins without an answer within 500 ms. `pairing_ended` releases the exact slot on registration, disconnect, trust cancellation, retry or timeout. `registered` includes `pairing_request_id`; already trusted SETUP no longer repeats registration. `pairing_attempt` is only an observation of Hub-authorized cumulative attempts.

An administrator can renew an idle worker using `trust_update` followed by `pairing_window` in the new trust generation. The window supplies the PIN, remaining milliseconds and attempt count; `pairing_window_applied` acknowledges its generations. A window cannot change an admitted or pending connection and cannot be replayed in the same trust generation to reset its attempt budget.

`tools/airplay_multi_source_probe.py` runs isolated macOS Hub acceptance with `--sources 0..4`, `--native-sources 0..4` (combined maximum four), and required `--output`. Native sources synthesize independent tones rather than capture the output. For example: `tools/dev python3 tools/airplay_multi_source_probe.py --profile release --sources 2 --native-sources 2 --output coreaudio:BlackHole2ch_UID`. It covers targeted failures/recovery, cross-receiver source competition, full-room admission, control idempotency, old-session fencing, live idle PIN renewal, and occupied discovery state beyond ten seconds. Temporary profiles live under `.local/airplay-multi/<run-id>` and are removed; redacted results remain in `artifacts/airplay/multi-source/<run-id>`. These are synthetic digital checks, not Apple selector/route compatibility acceptance.

Use `--fault-cycles 5` for repeated recovery stress. Each run copies Hub and worker into its private project-local `bin` directory and records those copies' hashes, so concurrent rebuilds cannot change a running fixture's executables. Reports separate a 300 ms pre-fault baseline, the fault interval, and recovery; each includes survivor lane underruns, source late-packet deltas, and the explicitly global Mixer late-frame counter. Stable session identities and nonzero meters are not by themselves a gap-free audio claim. Failure reports include only protocol stage, method/route, status and response length, never payloads or credentials.

Focused follow-ups use `--scenario management` or `--scenario mix-control`, both with `--sources 2 --native-sources 0`. Management checks live idle-entry configuration, cross-entry revocation, explicit PIN repair, and independent playback modes. Mix control checks per-lane gain, mute, single/multiple Solo, and reconnect preferences against three stable 50 ms meter windows; missing lanes never count as silence. Neither mode repeats the full mixing/fault matrix.


2026-10-07：平台层 `read_private_key` 使用同一已验证句柄读出有界 PEM，Windows 全程 UTF-16 文件 API，Unix 使用 no-follow fd + fstat。4096 字节及以上、宽权限、错误算法、多个对象和尾部垃圾均拒绝，秘密缓冲在所有分支清零。worker 调用新增 `raop_init2_from_pem`，不将 MinGW 的 FILE* 交给 OpenSSL，不生成替代身份。受维护的 libplist 补丁正确合并 JSON 的 UTF-16 surrogate pair，覆盖非 BMP 路径与嵌入 NUL 拒绝。

`ready` 增加 `identity_loader_version=1` 和 `stop_version=1`。stop 或 stdin EOF 正常析构，终止故障只发一次 fatal 并返回非零。Hub 的 writer 可取消、停止不依赖普通控制队列，正常路径等待 worker 后才回收线程及运行 PEM。

身份探针现在为 C++，实际使用平台读取器：`tools/dev cmake --build .local/airplay/build --target neonmix-airplay-identity-probe`，随后运行 `tools/dev python3 tools/airplay_identity_probe.py --worker .local/airplay/build/neonmix-airplay-worker --report artifacts/airplay-identity.json`。Windows 可传 `--plugins` 和按顺序排列的 `--dll-dir` 指定匹配 runtime；不会改变系统 ACP。probe 使用公开 RFC 8032 夹具，并覆盖校验后路径替换、Unicode、权限、大小边界、stop 与 EOF。

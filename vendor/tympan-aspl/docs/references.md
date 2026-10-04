English | [日本語](ja/references.md)

# References

Reference material consulted during design.

## Apple documentation

- **AudioServerPlugIn.h** (CoreAudio framework headers)
  - `<CoreAudio/AudioServerPlugIn.h>` in the macOS SDK
  - The canonical source for the AudioServerPlugin C interface
- **Creating an Audio Server Driver Plug-in**
  - <https://developer.apple.com/documentation/coreaudio/creating-an-audio-server-driver-plug-in>
- **Building an Audio Server Plug-in and Driver Extension**
  - <https://developer.apple.com/documentation/coreaudio/building-an-audio-server-plug-in-and-driver-extension>
- **Create audio drivers with DriverKit** (WWDC21 · 10190)
  - <https://developer.apple.com/videos/play/wwdc2021/10190/>
  - Overview of AudioDriverKit (a separate but adjacent technology)
- **TN3169: Choosing an audio driver type**
  - Technical note on AudioServerPlugin vs. AudioDriverKit

## Existing implementations

### libASPL

- <https://github.com/gavv/libASPL>
- C++17 library for writing AudioServerPlugins
- License: MIT
- The closest analogue to `tympan-aspl`, in a different language
- Notable: code-generated bridge layer, structured object model, request
  handler pattern

### BlackHole

- <https://github.com/ExistentialAudio/BlackHole>
- Production AudioServerPlugin: virtual loopback device for system audio
  routing
- License: GPL-3.0
- Implementation language: C / Objective-C
- Useful as a reference for: bundle packaging, Info.plist structure,
  `kAudioObjectPropertyName` handling, format negotiation

### BackgroundMusic

- <https://github.com/kyleneideck/BackgroundMusic>
- macOS audio utility built on a custom AudioServerPlugin (BGMDriver)
- License: GPL-2.0
- Implementation language: C++ / Objective-C
- Useful as a reference for: longer-lived state, app-aware behaviour,
  custom property dispatch
- See `BGMDriver/DEVELOPING.md` for an unusually thorough walkthrough of
  AudioServerPlugin internals

### Apple sample code

- **SimpleAudioDriver** — minimal AudioServerPlugin example bundled with
  the macOS SDK (`Examples/AudioServerDriverExamples/`)
- The starting point most implementations use

## Related Rust crates

### Foundation

- **coreaudio-sys**: <https://crates.io/crates/coreaudio-sys>
  - Raw bindgen-generated bindings to CoreAudio. The lowest layer
    `tympan-aspl` builds on.
- **core-foundation**: <https://crates.io/crates/core-foundation>
  - Safe wrappers for CFTypeRef, CFString, CFArray, etc.
- **core-foundation-sys**: <https://crates.io/crates/core-foundation-sys>
  - Raw CoreFoundation bindings (used transitively)

### Client-side (out of scope for tympan-aspl, but relevant)

- **coreaudio-rs**: <https://crates.io/crates/coreaudio-rs>
  - Friendly Rust interface to Apple's Core Audio API. Use this to
    *consume* an audio device, not to *implement* one.
- **cpal**: <https://crates.io/crates/cpal>
  - Cross-platform audio I/O. Same role as `coreaudio-rs`: client side.

### Realtime / lock-free

- **crossbeam**: <https://crates.io/crates/crossbeam>
  - Lock-free data structures suitable for the audio realtime thread
- **atomic-waker**: <https://crates.io/crates/atomic-waker>
  - Cross-thread wake notification (non-blocking)

## Audio Hardware Abstraction Layer (HAL) background

- **Core Audio Essentials**
  - <https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/CoreAudioOverview/>
  - Older but still relevant overview of the HAL architecture
- **HALLab**
  - Apple's HAL debugging tool. Inspect plug-in properties, send
    notifications, observe object state. Useful during plug-in
    development.

## Conventions and packaging

- **CFBundle Programming Guide**
  - <https://developer.apple.com/library/archive/documentation/CoreFoundation/Conceptual/CFBundles/>
  - Required reading for `.driver` bundle structure
- **Notarization**
  - <https://developer.apple.com/documentation/security/notarizing_macos_software_before_distribution>
  - Required for distributing plug-ins to end users on modern macOS
- **Code signing requirements for AudioServerPlugins**
  - Plug-ins must be signed with a Developer ID certificate or self-signed
    for local-only use. SIP enforces this.

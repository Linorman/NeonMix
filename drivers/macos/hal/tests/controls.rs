//! Exercise the actual C vtable with HAL property buffers and independent SDK operation codes.
#![allow(unsafe_code)]
use neonmix_hal::NeonMixHalFactory;
use tympan_aspl::raw::abi::{
    AudioObjectPropertyAddress, AudioServerPlugInDriverInterface, AudioServerPlugInDriverRef,
    AudioServerPlugInIOCycleInfo, AudioTimeStamp,
};

fn code(value: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*value)
}
struct Fixture {
    object: *mut std::ffi::c_void,
    clients: Vec<u32>,
}
impl Fixture {
    fn new() -> Self {
        // SAFETY: factory and Initialize permit null CF/host pointers in this process-local harness.
        let object = unsafe { NeonMixHalFactory(std::ptr::null(), std::ptr::null()) };
        assert!(!object.is_null());
        let value = Self {
            object,
            clients: vec![],
        };
        // SAFETY: the factory object is live until Fixture::drop.
        assert_eq!(
            // SAFETY: the factory object is live until Fixture::drop.
            unsafe { value.table().Initialize.unwrap()(value.driver(), std::ptr::null_mut()) },
            0
        );
        value
    }
    fn driver(&self) -> AudioServerPlugInDriverRef {
        self.object.cast()
    }
    fn table(&self) -> &AudioServerPlugInDriverInterface {
        // SAFETY: the live factory object's first word is its static C vtable.
        unsafe { &**self.driver() }
    }
    fn start(&mut self, id: u32) {
        // SAFETY: live factory ref and known device ID; HAL client IDs are opaque integers.
        assert_eq!(
            // SAFETY: live factory ref and known device ID.
            unsafe { self.table().StartIO.unwrap()(self.driver(), 2, id) },
            0
        );
        self.clients.push(id);
    }
    fn stop(&mut self, id: u32) {
        // SAFETY: each stop corresponds to a successful start on this live object.
        assert_eq!(
            // SAFETY: paired with a successful StartIO on the live object.
            unsafe { self.table().StopIO.unwrap()(self.driver(), 2, id) },
            0
        );
        self.clients.retain(|client| *client != id);
    }
    fn set(&self, id: u32, selector: &[u8; 4], scope: &[u8; 4], bytes: &[u8]) -> i32 {
        let address = AudioObjectPropertyAddress {
            mSelector: code(selector),
            mScope: code(scope),
            mElement: 0,
        };
        // SAFETY: address and readable payload remain live for the C call; size matches the slice.
        unsafe {
            self.table().SetPropertyData.unwrap()(
                self.driver(),
                id,
                0,
                &address,
                0,
                std::ptr::null(),
                bytes.len() as u32,
                bytes.as_ptr().cast(),
            )
        }
    }
    fn get(&self, id: u32, selector: &[u8; 4], scope: &[u8; 4]) -> Vec<u8> {
        let address = AudioObjectPropertyAddress {
            mSelector: code(selector),
            mScope: code(scope),
            mElement: 0,
        };
        let mut data = [0u8; 32];
        let mut used = 0;
        // SAFETY: all pointers address valid buffers and remain live for the C call.
        assert_eq!(
            // SAFETY: address and output buffers are live and correctly sized.
            unsafe {
                self.table().GetPropertyData.unwrap()(
                    self.driver(),
                    id,
                    0,
                    &address,
                    0,
                    std::ptr::null(),
                    data.len() as u32,
                    &mut used,
                    data.as_mut_ptr().cast(),
                )
            },
            0
        );
        data[..used as usize].to_vec()
    }
    fn roundtrip(&self, time: f64, source: &[f32; 8]) -> [f32; 8] {
        let client = self.clients[0];
        let timestamp = AudioTimeStamp {
            mSampleTime: time,
            ..Default::default()
        };
        let cycle = AudioServerPlugInIOCycleInfo {
            mInputTime: timestamp,
            mOutputTime: timestamp,
            ..Default::default()
        };
        let mut read = [9.0; 8];
        // SAFETY: the cycle info and stereo buffers are valid for four frames; both stream IDs
        // are declared by NeonMix. These literal operation codes match the Apple SDK, independently
        // of the framework's IoOperation constants.
        unsafe {
            assert_eq!(
                self.table().DoIOOperation.unwrap()(
                    self.driver(),
                    2,
                    4,
                    client,
                    code(b"rite"),
                    4,
                    &cycle,
                    source.as_ptr() as *mut _,
                    std::ptr::null_mut()
                ),
                0
            );
            assert_eq!(
                self.table().DoIOOperation.unwrap()(
                    self.driver(),
                    2,
                    3,
                    client,
                    code(b"read"),
                    4,
                    &cycle,
                    read.as_mut_ptr().cast(),
                    std::ptr::null_mut()
                ),
                0
            );
        }
        read
    }
    fn seed(&self) -> u64 {
        let (mut time, mut host, mut seed) = (0.0, 0, 0);
        // SAFETY: each output pointer is live and correctly typed.
        assert_eq!(
            // SAFETY: the output pointers are live and correctly typed.
            unsafe {
                self.table().GetZeroTimeStamp.unwrap()(
                    self.driver(),
                    2,
                    1,
                    &mut time,
                    &mut host,
                    &mut seed,
                )
            },
            0
        );
        seed
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for client in self.clients.clone() {
            self.stop(client);
        }
        // SAFETY: the final Release owns this factory object; nothing accesses it afterward.
        unsafe {
            self.table().Release.unwrap()(self.object);
        }
    }
}

#[test]
fn device_volume_and_mute_apply_once_on_the_mixed_render_side() {
    let mut f = Fixture::new();
    f.start(1);
    let controls = f.get(2, b"ctrl", b"outp");
    assert_eq!(controls, [5u32.to_ne_bytes(), 6u32.to_ne_bytes()].concat());
    assert_eq!(f.set(5, b"lcsv", b"glob", &0.5f32.to_ne_bytes()), 0);
    assert_eq!(f.get(2, b"volm", b"outp"), 0.5f32.to_ne_bytes());
    let db = f32::from_ne_bytes(f.get(5, b"lcdv", b"glob").try_into().unwrap());
    assert!((db + 6.0206).abs() < 0.001);
    let source = [0.25, -0.5, 0.75, -1.0, 0.125, -0.125, 0.0, 0.5];
    assert_eq!(
        f.roundtrip(1024.0, &source),
        source.map(|value| value * 0.5)
    );
    assert_eq!(f.set(6, b"bcvl", b"glob", &1u32.to_ne_bytes()), 0);
    assert_eq!(f.roundtrip(1028.0, &source), [0.0; 8]);
    assert_eq!(f.set(2, b"mute", b"outp", &0u32.to_ne_bytes()), 0);
    assert_eq!(
        f.roundtrip(1032.0, &source),
        source.map(|value| value * 0.5)
    );
    assert_eq!(f.set(6, b"bcvl", b"glob", &1u32.to_ne_bytes()), 0);
    assert_eq!(
        f.roundtrip(
            1036.0,
            &[
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                0.5,
                0.0,
                -0.2,
                0.1,
                0.7
            ]
        ),
        [0.0; 8]
    );
}

#[test]
fn stopping_one_client_preserves_other_clients_clock_and_audio() {
    let mut f = Fixture::new();
    f.start(1);
    let seed = f.seed();
    f.start(2);
    assert_eq!(f.seed(), seed);
    f.stop(1);
    let source = [0.2; 8];
    assert_eq!(f.roundtrip(256.0, &source), source);
    assert_eq!(f.seed(), seed);
    f.stop(2);
    f.start(3);
    assert!(f.seed() > seed);
}

#[test]
fn removing_a_running_client_retires_only_its_io_and_last_removal_stops_clock() {
    use tympan_aspl::raw::abi::AudioServerPlugInClientInfo;
    let mut f = Fixture::new();
    f.start(1);
    f.start(2);
    let seed = f.seed();
    let mut client = AudioServerPlugInClientInfo {
        mClientID: 1,
        mProcessID: 1,
        mIsNativeEndian: 1,
        mBundleID: std::ptr::null(),
    };
    // SAFETY: live factory reference and readable client information throughout both calls.
    unsafe {
        assert_eq!(
            f.table().RemoveDeviceClient.unwrap()(f.driver(), 2, &client),
            0
        );
    }
    f.clients.retain(|id| *id != 1);
    assert_eq!(f.seed(), seed);
    assert_eq!(f.roundtrip(128.0, &[0.25; 8]), [0.25; 8]);
    client.mClientID = 2;
    // SAFETY: same live reference and client record, now removing the final started client.
    unsafe {
        assert_eq!(
            f.table().RemoveDeviceClient.unwrap()(f.driver(), 2, &client),
            0
        );
    }
    f.clients.clear();
    assert_eq!(f.get(2, b"goin", b"glob"), 0u32.to_ne_bytes());
    f.start(3);
    assert!(f.seed() > seed);
}

#[test]
fn invalid_properties_and_io_lengths_do_not_change_volume() {
    let mut f = Fixture::new();
    f.start(1);
    for value in [f32::NAN, f32::INFINITY, -0.5, 1.5] {
        assert_ne!(f.set(5, b"lcsv", b"glob", &value.to_ne_bytes()), 0);
    }
    assert_ne!(f.set(5, b"lcsv", b"glob", &[0; 8]), 0);
    assert_ne!(f.set(2, b"volm", b"inpt", &0.5f32.to_ne_bytes()), 0);
    assert_eq!(f.get(5, b"lcsv", b"glob"), 1.0f32.to_ne_bytes());
    // SAFETY: malformed null buffers are deliberately rejected before any dereference.
    unsafe {
        assert_ne!(
            f.table().DoIOOperation.unwrap()(
                f.driver(),
                2,
                4,
                1,
                code(b"rite"),
                u32::MAX,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ),
            0
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn name_changes_keep_uid_clock_and_audio_and_reject_control_characters() {
    use tympan_aspl::raw::cf;
    let mut f = Fixture::new();
    f.start(1);
    let seed = f.seed();
    let before = f.get(2, b"uid ", b"glob");
    let name = cf::create_string("NeonMix — 客厅");
    let pointer = name as usize;
    assert_eq!(f.set(2, b"lnam", b"glob", &pointer.to_ne_bytes()), 0);
    // SAFETY: created CFString is owned by this test and setter has copied the text.
    unsafe { cf::release(name) };
    let got = f.get(2, b"lnam", b"glob");
    let reference = usize::from_ne_bytes(got.try_into().unwrap()) as *const std::ffi::c_void;
    // SAFETY: HAL getter returns a retained CFString; this test reads and releases it.
    unsafe {
        assert_eq!(cf::read_name(reference).as_deref(), Some("NeonMix — 客厅"));
        cf::release(reference);
    }
    let after = f.get(2, b"uid ", b"glob");
    let before = usize::from_ne_bytes(before.try_into().unwrap()) as *const std::ffi::c_void;
    let after = usize::from_ne_bytes(after.try_into().unwrap()) as *const std::ffi::c_void;
    // SAFETY: both getter-returned CFStrings are retained and released once here.
    unsafe {
        assert_eq!(cf::read_name(before), cf::read_name(after));
        cf::release(before);
        cf::release(after);
    }
    assert_eq!(f.seed(), seed);
    assert_eq!(f.roundtrip(256.0, &[0.125; 8]), [0.125; 8]);
    for text in ["", "  ", "bad\0name", "bad\nname"] {
        let invalid = cf::create_string(text);
        let pointer = invalid as usize;
        assert_ne!(f.set(2, b"lnam", b"glob", &pointer.to_ne_bytes()), 0);
        // SAFETY: the rejected input remains owned by this test.
        unsafe { cf::release(invalid) };
    }
}

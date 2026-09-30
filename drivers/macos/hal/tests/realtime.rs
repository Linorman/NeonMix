//! Allocation audit of actual HAL data and clock callbacks after startup.
#![allow(unsafe_code)]
use neonmix_hal::NeonMixHalFactory;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;
use tympan_aspl::raw::abi::{
    AudioServerPlugInDriverRef, AudioServerPlugInIOCycleInfo, AudioTimeStamp,
};
#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

#[test]
fn hal_data_and_clock_callbacks_allocate_and_free_nothing() {
    // SAFETY: factory returns a live interface; subsequent buffers are typed and sized correctly.
    unsafe {
        let object = NeonMixHalFactory(std::ptr::null(), std::ptr::null());
        assert!(!object.is_null());
        let driver: AudioServerPlugInDriverRef = object.cast();
        let table = &**driver;
        assert_eq!(table.Initialize.unwrap()(driver, std::ptr::null_mut()), 0);
        assert_eq!(table.StartIO.unwrap()(driver, 2, 1), 0);
        let source = [0.125f32; 512];
        let mut read = [0.0f32; 512];
        let (mut time, mut host, mut seed) = (0.0, 0, 0);
        let region = Region::new(GLOBAL);
        for index in 0..1000 {
            let timestamp = AudioTimeStamp {
                mSampleTime: f64::from(index * 256),
                ..Default::default()
            };
            let cycle = AudioServerPlugInIOCycleInfo {
                mInputTime: timestamp,
                mOutputTime: timestamp,
                ..Default::default()
            };
            assert_eq!(
                table.DoIOOperation.unwrap()(
                    driver,
                    2,
                    4,
                    1,
                    u32::from_be_bytes(*b"rite"),
                    256,
                    &cycle,
                    source.as_ptr() as *mut _,
                    std::ptr::null_mut()
                ),
                0
            );
            assert_eq!(
                table.DoIOOperation.unwrap()(
                    driver,
                    2,
                    3,
                    1,
                    u32::from_be_bytes(*b"read"),
                    256,
                    &cycle,
                    read.as_mut_ptr().cast(),
                    std::ptr::null_mut()
                ),
                0
            );
            assert_eq!(
                table.GetZeroTimeStamp.unwrap()(driver, 2, 1, &mut time, &mut host, &mut seed),
                0
            );
        }
        let stats = region.change();
        assert_eq!(
            (stats.allocations, stats.reallocations, stats.deallocations),
            (0, 0, 0)
        );
        assert_eq!(read, source);
        assert_eq!(table.StopIO.unwrap()(driver, 2, 1), 0);
        assert_eq!(table.Release.unwrap()(object), 0);
    }
}

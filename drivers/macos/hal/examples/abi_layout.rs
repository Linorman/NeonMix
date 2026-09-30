//! Values independently compared to the installed Apple SDK by tools/macos_hal_abi.py.
use std::mem::{offset_of, size_of};
use tympan_aspl::IoOperation;
use tympan_aspl::raw::abi::*;

fn main() {
    let mut values = vec![
        ("AudioTimeStamp.size", size_of::<AudioTimeStamp>()),
        ("SMPTETime.size", size_of::<SMPTETime>()),
        ("SMPTETime.mCounter", offset_of!(SMPTETime, mCounter)),
        (
            "AudioStreamBasicDescription.size",
            size_of::<AudioStreamBasicDescription>(),
        ),
        (
            "AudioServerPlugInClientInfo.size",
            size_of::<AudioServerPlugInClientInfo>(),
        ),
        (
            "AudioServerPlugInClientInfo.mBundleID",
            offset_of!(AudioServerPlugInClientInfo, mBundleID),
        ),
        (
            "AudioServerPlugInIOCycleInfo.size",
            size_of::<AudioServerPlugInIOCycleInfo>(),
        ),
        (
            "AudioServerPlugInIOCycleInfo.mInputTime",
            offset_of!(AudioServerPlugInIOCycleInfo, mInputTime),
        ),
        (
            "AudioServerPlugInIOCycleInfo.mOutputTime",
            offset_of!(AudioServerPlugInIOCycleInfo, mOutputTime),
        ),
        (
            "AudioServerPlugInDriverInterface.size",
            size_of::<AudioServerPlugInDriverInterface>(),
        ),
        (
            "AudioServerPlugInDriverInterface.StartIO",
            offset_of!(AudioServerPlugInDriverInterface, StartIO),
        ),
        (
            "AudioServerPlugInDriverInterface.DoIOOperation",
            offset_of!(AudioServerPlugInDriverInterface, DoIOOperation),
        ),
    ];
    for (name, op) in [
        ("Thread", IoOperation::THREAD),
        ("Cycle", IoOperation::CYCLE),
        ("ReadInput", IoOperation::READ_INPUT),
        ("ConvertInput", IoOperation::CONVERT_INPUT),
        ("ProcessInput", IoOperation::PROCESS_INPUT),
        ("ProcessOutput", IoOperation::PROCESS_OUTPUT),
        ("MixOutput", IoOperation::MIX_OUTPUT),
        ("ProcessMix", IoOperation::PROCESS_MIX),
        ("ConvertMix", IoOperation::CONVERT_MIX),
        ("WriteMix", IoOperation::WRITE_MIX),
    ] {
        values.push((name, op.code().as_u32() as usize));
    }
    println!(
        "{{{}}}",
        values
            .into_iter()
            .map(|(key, value)| format!("\"{key}\":{value}"))
            .collect::<Vec<_>>()
            .join(",")
    );
}

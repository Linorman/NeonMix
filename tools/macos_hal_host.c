// Load the actual project bundle using Apple's CFPlugIn loader and SDK vtable.
// This is a process-local host, not evidence of coreaudiod installation/loading.
#include <CoreAudio/AudioServerPlugIn.h>
#include <CoreFoundation/CoreFoundation.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static unsigned notifications = 0;
static OSStatus changed(AudioServerPlugInHostRef host, AudioObjectID object,
                        UInt32 count, const AudioObjectPropertyAddress *addresses) {
    (void)host; (void)object;
    if (count && addresses) notifications += count;
    return 0;
}
static void require(int success, const char *label) {
    if (!success) { fprintf(stderr, "HAL bundle probe failed: %s\n", label); exit(1); }
}
static OSStatus get(AudioServerPlugInDriverRef driver, AudioObjectID object,
                    AudioObjectPropertySelector selector, AudioObjectPropertyScope scope,
                    UInt32 capacity, void *out) {
    AudioObjectPropertyAddress address = {selector, scope, 0}; UInt32 used = 0;
    OSStatus status = (*driver)->GetPropertyData(driver, object, 0, &address, 0, NULL, capacity, &used, out);
    return status ? status : (used == capacity ? 0 : -1);
}
static OSStatus set(AudioServerPlugInDriverRef driver, AudioObjectID object,
                    AudioObjectPropertySelector selector, AudioObjectPropertyScope scope,
                    UInt32 size, const void *value) {
    AudioObjectPropertyAddress address = {selector, scope, 0};
    return (*driver)->SetPropertyData(driver, object, 0, &address, 0, NULL, size, value);
}
int main(int argc, char **argv) {
    require(argc == 2, "bundle argument");
    CFURLRef url = CFURLCreateFromFileSystemRepresentation(NULL, (const UInt8*)argv[1], strlen(argv[1]), true);
    CFPlugInRef plugin = CFPlugInCreate(NULL, url); CFRelease(url);
    require(plugin != NULL, "CFPlugInCreate");
    CFArrayRef factories = CFPlugInFindFactoriesForPlugInTypeInPlugIn(kAudioServerPlugInTypeUUID, plugin);
    require(factories && CFArrayGetCount(factories) == 1, "one declared factory");
    AudioServerPlugInDriverRef driver = CFPlugInInstanceCreate(NULL, CFArrayGetValueAtIndex(factories, 0), kAudioServerPlugInTypeUUID);
    require(driver && *driver, "CFPlugInInstanceCreate");
    void *queried = NULL;
    require((*driver)->QueryInterface(driver, CFUUIDGetUUIDBytes(kAudioServerPlugInDriverInterfaceUUID), &queried) == 0 && queried == driver, "SDK QueryInterface");
    require((*driver)->Release(queried) == 1, "queried interface release");
    AudioServerPlugInHostInterface host = {.PropertiesChanged = changed};
    require((*driver)->Initialize(driver, &host) == 0, "Initialize");
    AudioServerPlugInClientInfo client = {.mClientID = 1, .mProcessID = getpid(), .mIsNativeEndian = true};
    require((*driver)->AddDeviceClient(driver, 2, &client) == 0, "AddDeviceClient");
    CFStringRef uid = NULL;
    require(get(driver, 2, kAudioDevicePropertyDeviceUID, kAudioObjectPropertyScopeGlobal, sizeof(uid), &uid) == 0, "device UID");
    require(CFEqual(uid, CFSTR("com.neonmix.audio.virtual-output")), "stable UID");
    AudioObjectPropertyAddress translate = {kAudioPlugInPropertyTranslateUIDToDevice, kAudioObjectPropertyScopeGlobal, 0};
    AudioObjectID translated = 0; UInt32 translatedSize = 0;
    require((*driver)->HasProperty(driver, 1, 0, &translate), "UID translation advertised");
    require((*driver)->GetPropertyData(driver, 1, 0, &translate, sizeof(uid), &uid, sizeof(translated), &translatedSize, &translated) == 0 && translated == 2, "plug-in UID translation");
    CFStringRef missingUID = CFSTR("unrelated.device.uid");
    require((*driver)->GetPropertyData(driver, 1, 0, &translate, sizeof(missingUID), &missingUID, sizeof(translated), &translatedSize, &translated) == 0 && translated == kAudioObjectUnknown, "unknown UID is not substituted");
    require((*driver)->GetPropertyData(driver, 1, 0, &translate, 0, NULL, sizeof(translated), &translatedSize, &translated) != 0, "reject absent UID qualifier");
    require((*driver)->GetPropertyData(driver, 1, 0, &translate, sizeof(uid), &uid, 1, &translatedSize, &translated) != 0, "reject short UID output");
    CFStringRef resources = NULL;
    require(get(driver, 1, kAudioPlugInPropertyResourceBundle, kAudioObjectPropertyScopeGlobal, sizeof(resources), &resources) == 0 && CFEqual(resources, CFSTR("")), "plug-in resource bundle"); CFRelease(resources);
    CFRelease(uid);
    AudioStreamBasicDescription format = {0};
    require(get(driver, 4, kAudioStreamPropertyVirtualFormat, kAudioObjectPropertyScopeGlobal, sizeof(format), &format) == 0, "stream format");
    require(format.mSampleRate == 48000 && format.mChannelsPerFrame == 2 && format.mBitsPerChannel == 32, "48k stereo float32");
    AudioStreamRangedDescription available = {0};
    require(get(driver, 4, kAudioStreamPropertyAvailablePhysicalFormats, kAudioObjectPropertyScopeGlobal, sizeof(available), &available) == 0, "available physical formats");
    require(available.mFormat.mSampleRate == 48000 && available.mSampleRateRange.mMinimum == 48000 && available.mSampleRateRange.mMaximum == 48000, "fixed physical format range");
    UInt32 eligible = 0;
    require(get(driver, 2, kAudioDevicePropertyDeviceCanBeDefaultDevice, kAudioObjectPropertyScopeOutput, sizeof(eligible), &eligible) == 0 && eligible, "default output eligibility");
    require(get(driver, 2, kAudioDevicePropertyIsHidden, kAudioObjectPropertyScopeGlobal, sizeof(eligible), &eligible) == 0 && !eligible, "visible device");
    require((*driver)->StartIO(driver, 2, 1) == 0, "StartIO");
    Float64 sample = -1; UInt64 hostTime = 0, seed = 0;
    require((*driver)->GetZeroTimeStamp(driver, 2, 1, &sample, &hostTime, &seed) == 0 && seed > 0, "running clock");
    UInt64 firstSeed = seed;
    CFStringRef name = CFSTR("NeonMix — bundle probe");
    require(set(driver, 2, kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal, sizeof(name), &name) == 0, "rename");
    CFStringRef readName = NULL;
    require(get(driver, 2, kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal, sizeof(readName), &readName) == 0 && CFEqual(name, readName), "name readback"); CFRelease(readName);
    Float32 gain = 0.5;
    require(set(driver, 5, kAudioLevelControlPropertyScalarValue, kAudioObjectPropertyScopeGlobal, sizeof(gain), &gain) == 0, "volume");
    UInt32 mute = 0;
    require(set(driver, 6, kAudioBooleanControlPropertyValue, kAudioObjectPropertyScopeGlobal, sizeof(mute), &mute) == 0, "unmute");
    float source[8] = {0.25, -0.5, 0.75, -1, 0.125, -0.125, 0, 0.5}, output[8];
    AudioServerPlugInIOCycleInfo cycle = {0};
    cycle.mInputTime.mSampleTime = cycle.mOutputTime.mSampleTime = 1024;
    require((*driver)->DoIOOperation(driver, 2, 4, 1, kAudioServerPlugInIOOperationWriteMix, 4, &cycle, source, NULL) == 0, "WriteMix");
    require((*driver)->DoIOOperation(driver, 2, 3, 1, kAudioServerPlugInIOOperationReadInput, 4, &cycle, output, NULL) == 0, "ReadInput");
    for (int i = 0; i < 8; ++i) require(output[i] == source[i] * 0.5f, "volume applied once");
    mute = 1;
    require(set(driver, 6, kAudioBooleanControlPropertyValue, kAudioObjectPropertyScopeGlobal, sizeof(mute), &mute) == 0, "mute");
    cycle.mInputTime.mSampleTime = cycle.mOutputTime.mSampleTime = 1028;
    require((*driver)->DoIOOperation(driver, 2, 4, 1, kAudioServerPlugInIOOperationWriteMix, 4, &cycle, source, NULL) == 0, "muted WriteMix");
    require((*driver)->DoIOOperation(driver, 2, 3, 1, kAudioServerPlugInIOOperationReadInput, 4, &cycle, output, NULL) == 0, "muted ReadInput");
    for (int i = 0; i < 8; ++i) require(output[i] == 0, "digital mute");
    // Removing a client without StopIO must retire its run and reset the next epoch.
    require((*driver)->RemoveDeviceClient(driver, 2, &client) == 0, "abrupt client removal");
    UInt32 running = 1;
    require(get(driver, 2, kAudioDevicePropertyDeviceIsRunning, kAudioObjectPropertyScopeGlobal, sizeof(running), &running) == 0 && !running, "last removal stops device");
    require((*driver)->StartIO(driver, 2, 2) == 0, "restart");
    require((*driver)->GetZeroTimeStamp(driver, 2, 2, &sample, &hostTime, &seed) == 0 && seed > firstSeed, "fresh clock epoch");
    require((*driver)->DoIOOperation(driver, 2, 3, 2, kAudioServerPlugInIOOperationReadInput, 4, &cycle, output, NULL) == 0, "empty new epoch");
    for (int i = 0; i < 8; ++i) require(output[i] == 0, "no stale samples after restart");
    require((*driver)->StopIO(driver, 2, 2) == 0, "StopIO");
    require(notifications >= 8, "host property notifications");
    require((*driver)->Release(driver) == 0, "Release");
    CFRelease(factories); CFRelease(plugin);
    printf("{\"passed\":true,\"scope\":\"project bundle CFPlugIn host; not coreaudiod\",\"notifications\":%u}\n", notifications);
    return 0;
}

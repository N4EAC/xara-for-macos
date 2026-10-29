#include <AudioToolbox/AudioToolbox.h>
#include <CoreAudio/CoreAudio.h>
#include <CoreFoundation/CoreFoundation.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <time.h>

#define BLOCK 480
#define CAPACITY 48000
#define BUFFERS 3

typedef struct {
    pthread_mutex_t lock;
    pthread_cond_t changed;
    int16_t rx[CAPACITY], tx[CAPACITY];
    size_t rx_head, rx_count, tx_head, tx_count;
    int closing, overflow;
    AudioQueueRef input, output;
} XaraAudio;

static void capture(void *context, AudioQueueRef queue, AudioQueueBufferRef buffer,
                    const AudioTimeStamp *time, UInt32 packets, const AudioStreamPacketDescription *descriptions) {
    (void)time; (void)packets; (void)descriptions;
    XaraAudio *a = context;
    const int16_t *samples = buffer->mAudioData;
    size_t n = buffer->mAudioDataByteSize / sizeof(int16_t);
    pthread_mutex_lock(&a->lock);
    if (a->rx_count + n > CAPACITY) a->overflow = 1;
    else {
        for (size_t i = 0; i < n; i++) a->rx[(a->rx_head + a->rx_count + i) % CAPACITY] = samples[i];
        a->rx_count += n;
    }
    int closing = a->closing;
    pthread_cond_broadcast(&a->changed);
    pthread_mutex_unlock(&a->lock);
    if (!closing) AudioQueueEnqueueBuffer(queue, buffer, 0, NULL);
}

static void playback(void *context, AudioQueueRef queue, AudioQueueBufferRef buffer) {
    XaraAudio *a = context;
    int16_t *samples = buffer->mAudioData;
    pthread_mutex_lock(&a->lock);
    for (size_t i = 0; i < BLOCK; i++) {
        samples[i] = a->tx_count ? a->tx[a->tx_head] : 0;
        if (a->tx_count) { a->tx_head = (a->tx_head + 1) % CAPACITY; a->tx_count--; }
    }
    int closing = a->closing;
    pthread_cond_broadcast(&a->changed);
    pthread_mutex_unlock(&a->lock);
    buffer->mAudioDataByteSize = BLOCK * sizeof(int16_t);
    if (!closing) AudioQueueEnqueueBuffer(queue, buffer, 0, NULL);
}

static CFStringRef device_uid(const char *name) {
    if (!strcmp(name, "default")) return NULL;
    if (strncmp(name, "coreaudio:", 10)) return NULL;
    const char *hex = name + 10;
    size_t n = strlen(hex);
    if (!n || n % 2 || n > 2048) return NULL;
    UInt8 bytes[1024];
    for (size_t i = 0; i < n / 2; i++) {
        unsigned value;
        char pair[3] = { hex[i*2], hex[i*2+1], 0 };
        if (sscanf(pair, "%2x", &value) != 1) return NULL;
        bytes[i] = (UInt8)value;
    }
    return CFStringCreateWithBytes(NULL, bytes, n / 2, kCFStringEncodingUTF8, false);
}

static OSStatus select_device(AudioQueueRef queue, const char *name) {
    if (!strcmp(name, "default")) return noErr;
    CFStringRef uid = device_uid(name);
    if (!uid) return kAudio_ParamError;
    OSStatus result = AudioQueueSetProperty(queue, kAudioQueueProperty_CurrentDevice, &uid, sizeof(uid));
    CFRelease(uid);
    return result;
}

void xara_audio_close(void *context) {
    XaraAudio *a = context;
    if (!a) return;
    pthread_mutex_lock(&a->lock); a->closing = 1; pthread_cond_broadcast(&a->changed); pthread_mutex_unlock(&a->lock);
    if (a->input) { AudioQueueStop(a->input, true); AudioQueueDispose(a->input, true); }
    if (a->output) { AudioQueueStop(a->output, true); AudioQueueDispose(a->output, true); }
    pthread_cond_destroy(&a->changed); pthread_mutex_destroy(&a->lock); free(a);
}

void *xara_audio_open(const char *input, const char *output, int capture_only, int32_t *error) {
    XaraAudio *a = calloc(1, sizeof(*a));
    if (!a) { *error = ENOMEM; return NULL; }
    pthread_mutex_init(&a->lock, NULL); pthread_cond_init(&a->changed, NULL);
    AudioStreamBasicDescription format = {0};
    format.mSampleRate = 48000; format.mFormatID = kAudioFormatLinearPCM;
    format.mFormatFlags = kLinearPCMFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked;
    format.mBytesPerPacket = 2; format.mFramesPerPacket = 1; format.mBytesPerFrame = 2;
    format.mChannelsPerFrame = 1; format.mBitsPerChannel = 16;
    OSStatus result = AudioQueueNewInput(&format, capture, a, NULL, NULL, 0, &a->input);
    if (result == noErr) result = select_device(a->input, input);
    if (!capture_only && result == noErr) {
        result = AudioQueueNewOutput(&format, playback, a, NULL, NULL, 0, &a->output);
        if (result == noErr) result = select_device(a->output, output);
    }
    for (int i = 0; i < BUFFERS && result == noErr; i++) {
        AudioQueueBufferRef buffer = NULL;
        result = AudioQueueAllocateBuffer(a->input, BLOCK * 2, &buffer);
        if (result == noErr) result = AudioQueueEnqueueBuffer(a->input, buffer, 0, NULL);
        if (a->output && result == noErr) {
            result = AudioQueueAllocateBuffer(a->output, BLOCK * 2, &buffer);
            if (result == noErr) {
                memset(buffer->mAudioData, 0, BLOCK * 2); buffer->mAudioDataByteSize = BLOCK * 2;
                result = AudioQueueEnqueueBuffer(a->output, buffer, 0, NULL);
            }
        }
    }
    if (result == noErr && a->output) result = AudioQueueStart(a->output, NULL);
    if (result == noErr) result = AudioQueueStart(a->input, NULL);
    if (result != noErr) { *error = result; xara_audio_close(a); return NULL; }
    *error = 0; return a;
}

int xara_audio_read(void *context, int16_t *samples, size_t n) {
    XaraAudio *a = context;
    struct timespec deadline; clock_gettime(CLOCK_REALTIME, &deadline); deadline.tv_sec++;
    pthread_mutex_lock(&a->lock);
    int result = 0;
    while (a->rx_count < n && !a->closing && !a->overflow && result == 0)
        result = pthread_cond_timedwait(&a->changed, &a->lock, &deadline);
    if (a->closing) result = ECANCELED;
    if (a->overflow) result = EOVERFLOW;
    if (!result) {
        for (size_t i = 0; i < n; i++) samples[i] = a->rx[(a->rx_head + i) % CAPACITY];
        a->rx_head = (a->rx_head + n) % CAPACITY; a->rx_count -= n;
    }
    pthread_mutex_unlock(&a->lock); return result;
}

int xara_audio_write(void *context, const int16_t *samples, size_t n) {
    XaraAudio *a = context;
    struct timespec deadline; clock_gettime(CLOCK_REALTIME, &deadline); deadline.tv_sec++;
    pthread_mutex_lock(&a->lock);
    int result = 0;
    // Bound queued output to 100 ms; hardware stalls must release the radio via the watchdog.
    while (a->tx_count + n > BLOCK * 10 && !a->closing && result == 0)
        result = pthread_cond_timedwait(&a->changed, &a->lock, &deadline);
    if (a->closing) result = ECANCELED;
    if (!result) {
        for (size_t i = 0; i < n; i++) a->tx[(a->tx_head + a->tx_count + i) % CAPACITY] = samples[i];
        a->tx_count += n;
    }
    pthread_mutex_unlock(&a->lock); return result;
}

typedef void (*DeviceVisitor)(void *, const char *, const char *, int);
int32_t xara_audio_devices(DeviceVisitor visit, void *context) {
    AudioObjectPropertyAddress property = { kAudioHardwarePropertyDevices, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain };
    UInt32 size = 0;
    OSStatus result = AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &property, 0, NULL, &size);
    if (result != noErr) return result;
    AudioDeviceID *devices = malloc(size);
    if (!devices) return ENOMEM;
    result = AudioObjectGetPropertyData(kAudioObjectSystemObject, &property, 0, NULL, &size, devices);
    if (result != noErr) { free(devices); return result; }
    for (size_t i = 0; i < size / sizeof(AudioDeviceID); i++) {
        CFStringRef uid = NULL, label = NULL; UInt32 len = sizeof(CFStringRef);
        property.mSelector = kAudioDevicePropertyDeviceUID; property.mScope = kAudioObjectPropertyScopeGlobal;
        if (AudioObjectGetPropertyData(devices[i], &property, 0, NULL, &len, &uid) != noErr) continue;
        property.mSelector = kAudioObjectPropertyName; len = sizeof(CFStringRef);
        if (AudioObjectGetPropertyData(devices[i], &property, 0, NULL, &len, &label) != noErr) { CFRelease(uid); continue; }
        char uid_text[1024], label_text[1024], name[2060];
        int valid = CFStringGetCString(uid, uid_text, sizeof(uid_text), kCFStringEncodingUTF8) &&
                    CFStringGetCString(label, label_text, sizeof(label_text), kCFStringEncodingUTF8);
        if (valid) {
            strcpy(name, "coreaudio:"); size_t n = strlen(uid_text);
            for (size_t j = 0; j < n; j++) snprintf(name + 10 + j*2, 3, "%02x", (unsigned char)uid_text[j]);
            for (int input = 0; input <= 1; input++) {
                property.mSelector = kAudioDevicePropertyStreamConfiguration;
                property.mScope = input ? kAudioDevicePropertyScopeInput : kAudioDevicePropertyScopeOutput;
                UInt32 bytes = 0;
                if (AudioObjectGetPropertyDataSize(devices[i], &property, 0, NULL, &bytes) != noErr) continue;
                AudioBufferList *buffers = malloc(bytes);
                if (!buffers) continue;
                if (AudioObjectGetPropertyData(devices[i], &property, 0, NULL, &bytes, buffers) == noErr) {
                    UInt32 channels = 0;
                    for (UInt32 j = 0; j < buffers->mNumberBuffers; j++) channels += buffers->mBuffers[j].mNumberChannels;
                    if (channels) visit(context, name, label_text, input);
                }
                free(buffers);
            }
        }
        CFRelease(uid); CFRelease(label);
    }
    free(devices); return noErr;
}

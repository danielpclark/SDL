/* Upstream reference for coreaudio::tests::spawned_session_matches_c on
 * tvOS: spawned, not an app (diag branch only). */
#define SDL_MAIN_HANDLED
#include <SDL3/SDL.h>
#include <SDL3/SDL_main.h>
#import <AVFoundation/AVFoundation.h>
#include <stdio.h>

static SDL_AtomicInt counts[2];

static void session(const char *label)
{
    @autoreleasepool {
        AVAudioSession *s = [AVAudioSession sharedInstance];
        printf("trace: %s: session: category=%s options=0x%lx mode=%s\n", label,
               s.category ? s.category.UTF8String : "nil", (unsigned long)s.categoryOptions,
               s.mode ? s.mode.UTF8String : "nil");
    }
}

static void SDLCALL postmix(void *userdata, const SDL_AudioSpec *spec, float *buffer, int buflen)
{
    SDL_AddAtomicInt(&counts[(int)(intptr_t)userdata], 1);
}

static SDL_AudioDeviceID open_dev(const char *label, SDL_AudioDeviceID which, int idx)
{
    SDL_AudioDeviceID d = SDL_OpenAudioDevice(which, NULL);
    if (!d) {
        printf("trace: %s: open: error: %s\n", label, SDL_GetError());
        return 0;
    }
    SDL_AudioSpec spec;
    int frames = 0;
    SDL_GetAudioDeviceFormat(d, &spec, &frames);
    printf("trace: %s: open: ok format=0x%x channels=%d freq=%d frames=%d\n", label, (unsigned)spec.format,
           spec.channels, spec.freq, frames);
    SDL_SetAudioPostmixCallback(d, postmix, (void *)(intptr_t)idx);
    return d;
}

static void advancing(const char *label)
{
    SDL_Delay(250);
    int p0 = SDL_GetAtomicInt(&counts[0]), r0 = SDL_GetAtomicInt(&counts[1]);
    SDL_Delay(500);
    int p1 = SDL_GetAtomicInt(&counts[0]), r1 = SDL_GetAtomicInt(&counts[1]);
    printf("trace: %s: playback advanced=%s recording advanced=%s\n", label, p1 > p0 ? "yes" : "no", r1 > r0 ? "yes" : "no");
}

static void close_dev(const char *line, SDL_AudioDeviceID d)
{
    if (d) {
        SDL_CloseAudioDevice(d);
        printf("trace: %s\n", line);
    }
}

int main(int argc, char *argv[])
{
    SDL_SetMainReady();
    SDL_SetHint(SDL_HINT_AUDIO_DRIVER, "coreaudio");
    bool ok = SDL_InitSubSystem(SDL_INIT_AUDIO);
    SDL_ResetHint(SDL_HINT_AUDIO_DRIVER);
    printf("trace: init: %s driver=%s\n", ok ? "ok" : SDL_GetError(), SDL_GetCurrentAudioDriver() ? SDL_GetCurrentAudioDriver() : "(null)");
    int n = 0;
    SDL_AudioDeviceID *ids = SDL_GetAudioPlaybackDevices(&n);
    for (int i = 0; i < n; i++) printf("trace: playback %d: %s\n", i, SDL_GetAudioDeviceName(ids[i]));
    SDL_free(ids);
    ids = SDL_GetAudioRecordingDevices(&n);
    for (int i = 0; i < n; i++) printf("trace: recording %d: %s\n", i, SDL_GetAudioDeviceName(ids[i]));
    SDL_free(ids);

    SDL_AudioDeviceID p = open_dev("playback", SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, 0);
    session("playback");
    advancing("playback");
    close_dev("playback: closed", p);
    session("playback closed");

    SDL_AudioDeviceID r = open_dev("recording", SDL_AUDIO_DEVICE_DEFAULT_RECORDING, 1);
    session("recording");
    advancing("recording");
    close_dev("recording: closed", r);
    session("recording closed");

    p = open_dev("both: playback", SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, 0);
    r = open_dev("both: recording", SDL_AUDIO_DEVICE_DEFAULT_RECORDING, 1);
    session("both");
    advancing("both");
    close_dev("both: recording closed", r);
    session("both: recording closed");
    close_dev("both: playback closed", p);
    session("both: playback closed");

    SDL_SetHint(SDL_HINT_AUDIO_CATEGORY, "ambient");
    SDL_SetHint(SDL_HINT_AUDIO_DUCK_OTHERS, "1");
    p = open_dev("ambient", SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, 0);
    session("ambient");
    close_dev("ambient: closed", p);
    SDL_ResetHint(SDL_HINT_AUDIO_CATEGORY);
    p = open_dev("ducking", SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, 0);
    session("ducking");
    close_dev("ducking: closed", p);
    SDL_ResetHint(SDL_HINT_AUDIO_DUCK_OTHERS);

    SDL_QuitSubSystem(SDL_INIT_AUDIO);
    session("quit");
    printf("trace: done\n");
    return 0;
}

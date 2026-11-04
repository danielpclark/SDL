# diag only: the translation's Note (upstream) fix in the GPU API's Vulkan
# backend (sdl3/src/gpu/vulkan/device.rs, load_entry_points()):
# VULKAN_INTERNAL_LoadEntryPoints() takes a reference to the loader
# (SDL_Vulkan_LoadLibrary()) that it never releases, one more with each
# PrepareDriver and CreateDevice, so the loader is never unloaded; the
# translation releases it before returning (the caller's own reference,
# PrepareDriver's or the device's, keeps the loader loaded meanwhile).
import sys

p = sys.argv[1] + "/src/gpu/vulkan/SDL_gpu_vulkan.c"
s = open(p).read()


def rep(old, new):
    global s
    assert s.count(old) == 1, old
    s = s.replace(old, new)


rep('''    if (!SDL_Vulkan_LoadLibrary(NULL)) {
        SDL_LogWarn(SDL_LOG_CATEGORY_GPU, "Vulkan: SDL_Vulkan_LoadLibrary failed!");
        return;
    }
''', '''    if (!SDL_Vulkan_LoadLibrary(NULL)) {
        SDL_LogWarn(SDL_LOG_CATEGORY_GPU, "Vulkan: SDL_Vulkan_LoadLibrary failed!");
        return;
    }
    SDL_Vulkan_UnloadLibrary();
''')
open(p, "w").write(s)

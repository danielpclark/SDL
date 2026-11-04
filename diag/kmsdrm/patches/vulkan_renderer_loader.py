# diag only: the translation's Note (upstream) fix in render/vulkan
# (sdl3/src/render/vulkan/mod.rs, create_device_resources() and
# destroy_all()): the Vulkan renderer's reference to the loader
# (SDL_Vulkan_LoadLibrary() in VULKAN_CreateDeviceResources()) is released
# by VULKAN_DestroyAll(), which upstream never does.
import sys

p = sys.argv[1] + "/src/render/vulkan/SDL_render_vulkan.c"
s = open(p).read()


def rep(old, new):
    global s
    assert s.count(old) == 1, old
    s = s.replace(old, new)


rep("} VULKAN_RenderData;", "    bool loader_loaded;\n} VULKAN_RenderData;")
rep('''    if (!SDL_Vulkan_LoadLibrary(NULL)) {
        SDL_LogDebug(SDL_LOG_CATEGORY_RENDER, "SDL_Vulkan_LoadLibrary failed" );
        return VK_ERROR_UNKNOWN;
    }
''', '''    if (!SDL_Vulkan_LoadLibrary(NULL)) {
        SDL_LogDebug(SDL_LOG_CATEGORY_RENDER, "SDL_Vulkan_LoadLibrary failed" );
        return VK_ERROR_UNKNOWN;
    }
    rendererData->loader_loaded = true;
''')
rep('''    if (rendererData->instance != VK_NULL_HANDLE && !rendererData->instance_external) {
        vkDestroyInstance(rendererData->instance, NULL);
        rendererData->instance = VK_NULL_HANDLE;
    }
}
''', '''    if (rendererData->instance != VK_NULL_HANDLE && !rendererData->instance_external) {
        vkDestroyInstance(rendererData->instance, NULL);
        rendererData->instance = VK_NULL_HANDLE;
    }
    if (rendererData->loader_loaded) {
        SDL_Vulkan_UnloadLibrary();
        rendererData->loader_loaded = false;
    }
}
''')
open(p, "w").write(s)

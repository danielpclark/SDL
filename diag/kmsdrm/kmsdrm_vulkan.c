/* diag only: the C twin of sdl3/tests/kmsdrm_vulkan.rs. */
#include <vulkan/vulkan.h>
#include "kms_common.h"

static PFN_vkGetInstanceProcAddr gipa;

static void window_events(void)
{
    SDL_Event e;
    SDL_PumpEvents();
    while (SDL_PollEvent(&e)) {
        if (e.type >= SDL_EVENT_WINDOW_FIRST && e.type <= SDL_EVENT_WINDOW_LAST) {
            line("window event 0x%x %d %d", e.type, e.window.data1, e.window.data2);
        }
    }
}

static void window_state(const char *label, SDL_Window *window)
{
    int w = 0, h = 0;
    const SDL_DisplayMode *m = SDL_GetWindowFullscreenMode(window);
    SDL_GetWindowSize(window, &w, &h);
    line("%s: flags 0x%llx size %dx%d fullscreen mode %s", label,
         (unsigned long long)SDL_GetWindowFlags(window), w, h, m ? mode_line(m) : "none");
}

static void current_mode(SDL_Window *window)
{
    const SDL_DisplayMode *m = SDL_GetCurrentDisplayMode(SDL_GetDisplayForWindow(window));
    if (m) {
        line("current mode: %s", mode_line(m));
    }
}

static const char *device_type(VkPhysicalDeviceType t)
{
    switch ((int)t) {
    case 0: return "other";
    case 1: return "integrated GPU";
    case 2: return "discrete GPU";
    case 3: return "virtual GPU";
    case 4: return "CPU";
    default: return "unknown";
    }
}

static VkResult create_instance(char const *const *extensions, Uint32 count, VkInstance *instance)
{
    PFN_vkCreateInstance create = (PFN_vkCreateInstance)gipa(NULL, "vkCreateInstance");
    VkApplicationInfo app = { VK_STRUCTURE_TYPE_APPLICATION_INFO };
    VkInstanceCreateInfo info = { VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO };
    app.apiVersion = VK_API_VERSION_1_0;
    info.pApplicationInfo = &app;
    info.enabledExtensionCount = count;
    info.ppEnabledExtensionNames = extensions;
    *instance = NULL;
    return create(&info, NULL, instance);
}

static const char *vkresult(VkResult r)
{
    static char buf[64];
    if (r == VK_SUCCESS) {
        return "ok";
    }
    snprintf(buf, sizeof(buf), "VkResult %d", (int)r);
    return buf;
}

int main(void)
{
    Uint32 count = 0;
    char const *const *extensions;
    char joined[512] = "";
    VkInstance instance = NULL, plain = NULL;
    VkResult ri, rp;
    SDL_Window *window;
    static const char *const surface_only[] = { "VK_KHR_surface" };

    begin();
    ask("drm state");

    line("-- init");
    if (!init_video()) {
        return finish("kmsdrm_vulkan");
    }

    line("-- library");
    extensions = SDL_Vulkan_GetInstanceExtensions(&count);
    if (extensions) {
        for (Uint32 i = 0; i < count; i++) {
            if (i) {
                strcat(joined, ", ");
            }
            strcat(joined, extensions[i]);
        }
        line("instance extensions: %s", joined);
    } else {
        line("instance extensions: error: %s", SDL_GetError());
    }
    if (!SDL_Vulkan_LoadLibrary(NULL)) {
        line("load the default library: %s", result(false));
        quit_video();
        return finish("kmsdrm_vulkan");
    }
    line("load the default library: ok");
    line("load it again: %s", result(SDL_Vulkan_LoadLibrary(NULL)));
    line("load another library: %s", result(SDL_Vulkan_LoadLibrary("libvulkan.so")));
    gipa = (PFN_vkGetInstanceProcAddr)SDL_Vulkan_GetVkGetInstanceProcAddr();
    line("vkGetInstanceProcAddr: %s", result(gipa != NULL));

    line("-- instance");
    ri = create_instance(extensions, count, &instance);
    line("instance: %s", vkresult(ri));
    rp = create_instance(surface_only, 1, &plain);
    line("instance without VK_KHR_display: %s", vkresult(rp));
    if (ri == VK_SUCCESS) {
        PFN_vkEnumeratePhysicalDevices enumerate = (PFN_vkEnumeratePhysicalDevices)gipa(instance, "vkEnumeratePhysicalDevices");
        PFN_vkGetPhysicalDeviceProperties props = (PFN_vkGetPhysicalDeviceProperties)gipa(instance, "vkGetPhysicalDeviceProperties");
        PFN_vkGetPhysicalDeviceDisplayPropertiesKHR displays = (PFN_vkGetPhysicalDeviceDisplayPropertiesKHR)gipa(instance, "vkGetPhysicalDeviceDisplayPropertiesKHR");
        PFN_vkGetPhysicalDeviceDisplayPlanePropertiesKHR planes = (PFN_vkGetPhysicalDeviceDisplayPlanePropertiesKHR)gipa(instance, "vkGetPhysicalDeviceDisplayPlanePropertiesKHR");
        Uint32 n = 0;
        VkPhysicalDevice devices[16];
        enumerate(instance, &n, NULL);
        if (n > 16) {
            n = 16;
        }
        enumerate(instance, &n, devices);
        line("physical devices: %u", n);
        for (Uint32 i = 0; i < n; i++) {
            VkPhysicalDeviceProperties p;
            props(devices[i], &p);
            printf("note: physical device %u: %s\n", i, p.deviceName);
            if (displays && planes) {
                Uint32 d = 0, pl = 0;
                displays(devices[i], &d, NULL);
                planes(devices[i], &pl, NULL);
                line("physical device %u: %s, %u displays, %u planes", i, device_type(p.deviceType), d, pl);
            } else {
                line("physical device %u: %s, no VK_KHR_display functions", i, device_type(p.deviceType));
            }
        }
    }

    line("-- vulkan window");
    window = SDL_CreateWindow("kmsdrm_vulkan", 640, 480, SDL_WINDOW_VULKAN);
    line("vulkan window: %s", result(window != NULL));
    if (window) {
        SDL_Window *gl_window;
        const SDL_DisplayMode *closest;
        SDL_DisplayMode mode;

        window_state("vulkan window", window);
        window_events();
        ask("drm state");

        line("-- surface");
        if (ri == VK_SUCCESS) {
            VkSurfaceKHR surface = 0;
            bool ok = SDL_Vulkan_CreateSurface(window, instance, NULL, &surface);
            line("surface: %s", result(ok));
            if (ok) {
                SDL_Vulkan_DestroySurface(instance, surface, NULL);
            }
            line("presentation support: %s", SDL_Vulkan_GetPresentationSupport(instance, (VkPhysicalDevice)1, 0) ? "true" : "false");
        }
        if (rp == VK_SUCCESS) {
            VkSurfaceKHR surface = 0;
            line("surface without VK_KHR_display: %s", result(SDL_Vulkan_CreateSurface(window, plain, NULL, &surface)));
        }
        ask("drm state");

        line("-- vulkan mode");
        (void)closest;
        if (SDL_GetClosestFullscreenDisplayMode(SDL_GetDisplayForWindow(window), 800, 600, 0.0f, false, &mode)) {
            line("set fullscreen mode: %s", result(SDL_SetWindowFullscreenMode(window, &mode)));
        } else {
            line("set fullscreen mode: error: %s", SDL_GetError());
        }
        line("set fullscreen: %s", result(SDL_SetWindowFullscreen(window, true)));
        window_events();
        window_state("vulkan window", window);
        current_mode(window);
        line("set fullscreen false: %s", result(SDL_SetWindowFullscreen(window, false)));
        window_events();
        current_mode(window);
        line("set size: %s", result(SDL_SetWindowSize(window, 1024, 768)));
        window_events();
        window_state("vulkan window", window);
        ask("drm state");

        gl_window = SDL_CreateWindow("kmsdrm_vulkan gl", 640, 480, SDL_WINDOW_OPENGL);
        line("opengl window in vulkan mode: %s", result(gl_window != NULL));
        if (gl_window) {
            SDL_GLContext context;
            window_state("opengl window", gl_window);
            window_events();
            context = SDL_GL_CreateContext(gl_window);
            line("context: %s", result(context != NULL));
            if (context) {
                SDL_GL_DestroyContext(context);
            }
            SDL_DestroyWindow(gl_window);
            line("opengl window destroyed");
            window_events();
        }
        ask("drm state");
        SDL_DestroyWindow(window);
        line("vulkan window destroyed");
        window_events();
    }

    line("-- an opengl window after");
    window = SDL_CreateWindow("kmsdrm_vulkan", 1024, 768, 0);
    line("window: %s", result(window != NULL));
    if (window) {
        SDL_GLContext context;
        window_state("window", window);
        window_events();
        context = SDL_GL_CreateContext(window);
        line("context: %s", result(context != NULL));
        if (context) {
            gl_clear(1.0f, 0.0f, 0.0f);
            line("swap (%.1f, %.1f, %.1f): %s", 1.0, 0.0, 0.0, result(SDL_GL_SwapWindow(window)));
            gl_clear(0.0f, 1.0f, 0.0f);
            line("swap (%.1f, %.1f, %.1f): %s", 0.0, 1.0, 0.0, result(SDL_GL_SwapWindow(window)));
            window_events();
            ask("drm state");
            ask("drm crc");
            SDL_GL_DestroyContext(context);
        }
        SDL_DestroyWindow(window);
        line("window destroyed");
        window_events();
    }

    line("-- renderers on a vulkan window");
    window = SDL_CreateWindow("kmsdrm_vulkan", 1024, 768, SDL_WINDOW_VULKAN);
    line("vulkan window: %s", result(window != NULL));
    if (window) {
        static const char *const names[] = { "vulkan", "gpu" };
        window_events();
        for (int i = 0; i < 2; i++) {
            SDL_Renderer *r = SDL_CreateRenderer(window, names[i]);
            line("renderer %s: %s", names[i], result(r != NULL));
            if (r) {
                SDL_DestroyRenderer(r);
            }
            window_state("vulkan window", window);
        }
        SDL_DestroyWindow(window);
        line("vulkan window destroyed");
        window_events();
    }

    if (rp == VK_SUCCESS) {
        ((PFN_vkDestroyInstance)gipa(plain, "vkDestroyInstance"))(plain, NULL);
    }
    if (ri == VK_SUCCESS) {
        ((PFN_vkDestroyInstance)gipa(instance, "vkDestroyInstance"))(instance, NULL);
    }
    SDL_Vulkan_UnloadLibrary();
    SDL_Vulkan_UnloadLibrary();
    line("unloaded: vkGetInstanceProcAddr: %s", result(SDL_Vulkan_GetVkGetInstanceProcAddr() != NULL));
    quit_video();
    ask("drm state");
    return finish("kmsdrm_vulkan");
}

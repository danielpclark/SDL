/* diag only: which Vulkan devices see the vkms card's displays, through
 * VK_KHR_display alone and after VK_EXT_acquire_drm_display. Run as root
 * with the card free (no master). */
#include <vulkan/vulkan.h>
#include <xf86drm.h>
#include <xf86drmMode.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void counts(VkInstance inst, VkPhysicalDevice pd, const char *when)
{
    PFN_vkGetPhysicalDeviceDisplayPropertiesKHR dp = (void *)vkGetInstanceProcAddr(inst, "vkGetPhysicalDeviceDisplayPropertiesKHR");
    PFN_vkGetPhysicalDeviceDisplayPlanePropertiesKHR pp = (void *)vkGetInstanceProcAddr(inst, "vkGetPhysicalDeviceDisplayPlanePropertiesKHR");
    PFN_vkGetDisplayModePropertiesKHR mp = (void *)vkGetInstanceProcAddr(inst, "vkGetDisplayModePropertiesKHR");
    uint32_t d = 0, p = 0;
    VkResult rd = dp(pd, &d, NULL);
    VkResult rp = pp(pd, &p, NULL);
    printf("vkprobe:   %s: displays %u (%d), planes %u (%d)\n", when, d, rd, p, rp);
    if (d > 0) {
        VkDisplayPropertiesKHR props[8];
        if (d > 8) d = 8;
        dp(pd, &d, props);
        for (uint32_t i = 0; i < d; i++) {
            uint32_t m = 0;
            mp(pd, props[i].display, &m, NULL);
            printf("vkprobe:     display %u \"%s\" %ux%u, %u modes\n", i, props[i].displayName ? props[i].displayName : "(null)",
                   props[i].physicalResolution.width, props[i].physicalResolution.height, m);
        }
    }
}

int main(int argc, char **argv)
{
    const char *card = argc > 1 ? argv[1] : "/dev/dri/card0";
    const char *exts[] = { "VK_KHR_surface", "VK_KHR_display", "VK_EXT_direct_mode_display", "VK_EXT_acquire_drm_display", "VK_KHR_get_display_properties2", "VK_KHR_get_physical_device_properties2" };
    uint32_t n = 0;
    VkExtensionProperties ie[256];
    vkEnumerateInstanceExtensionProperties(NULL, &n, NULL);
    if (n > 256) n = 256;
    vkEnumerateInstanceExtensionProperties(NULL, &n, ie);
    printf("vkprobe: instance extensions:");
    for (uint32_t i = 0; i < n; i++) printf(" %s", ie[i].extensionName);
    printf("\n");

    VkApplicationInfo app = { VK_STRUCTURE_TYPE_APPLICATION_INFO };
    app.apiVersion = VK_API_VERSION_1_1;
    VkInstanceCreateInfo ci = { VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO };
    ci.pApplicationInfo = &app;
    ci.enabledExtensionCount = 6;
    ci.ppEnabledExtensionNames = exts;
    VkInstance inst;
    VkResult r = vkCreateInstance(&ci, NULL, &inst);
    printf("vkprobe: instance %d\n", r);
    if (r) return 1;
    PFN_vkGetDrmDisplayEXT getdrm = (void *)vkGetInstanceProcAddr(inst, "vkGetDrmDisplayEXT");
    PFN_vkAcquireDrmDisplayEXT acquire = (void *)vkGetInstanceProcAddr(inst, "vkAcquireDrmDisplayEXT");
    PFN_vkReleaseDisplayEXT release = (void *)vkGetInstanceProcAddr(inst, "vkReleaseDisplayEXT");
    VkPhysicalDevice pds[8];
    uint32_t np = 8;
    vkEnumeratePhysicalDevices(inst, &np, pds);
    printf("vkprobe: physical devices %u\n", np);

    int fd = open(card, O_RDWR | O_CLOEXEC);
    drmModeRes *res = fd >= 0 ? drmModeGetResources(fd) : NULL;
    printf("vkprobe: %s fd %d, master %d, connectors %d\n", card, fd, fd >= 0 ? drmSetMaster(fd) : -1, res ? res->count_connectors : -1);

    for (uint32_t i = 0; i < np; i++) {
        VkPhysicalDeviceProperties p;
        vkGetPhysicalDeviceProperties(pds[i], &p);
        uint32_t ne = 0;
        vkEnumerateDeviceExtensionProperties(pds[i], NULL, &ne, NULL);
        VkExtensionProperties *de = calloc(ne, sizeof(*de));
        vkEnumerateDeviceExtensionProperties(pds[i], NULL, &ne, de);
        printf("vkprobe: device %u \"%s\" type %d api %u.%u driver 0x%x, %u device extensions (swapchain %s)\n", i, p.deviceName, p.deviceType,
               VK_API_VERSION_MAJOR(p.apiVersion), VK_API_VERSION_MINOR(p.apiVersion), p.driverVersion, ne,
               ({ int s = 0; for (uint32_t k = 0; k < ne; k++) if (!strcmp(de[k].extensionName, "VK_KHR_swapchain")) s = 1; s ? "yes" : "no"; }));
        free(de);
        counts(inst, pds[i], "VK_KHR_display alone");
        if (!res || !getdrm || !acquire) {
            printf("vkprobe:   no vkGetDrmDisplayEXT/vkAcquireDrmDisplayEXT (%p %p)\n", (void *)getdrm, (void *)acquire);
            continue;
        }
        for (int c = 0; c < res->count_connectors; c++) {
            VkDisplayKHR disp = VK_NULL_HANDLE;
            r = getdrm(pds[i], fd, res->connectors[c], &disp);
            printf("vkprobe:   vkGetDrmDisplayEXT(connector %u) = %d, display %s\n", res->connectors[c], r, disp ? "set" : "null");
            if (r == VK_SUCCESS && disp) {
                r = acquire(pds[i], fd, disp);
                printf("vkprobe:   vkAcquireDrmDisplayEXT = %d\n", r);
                counts(inst, pds[i], "after the acquire");
                if (r == VK_SUCCESS && release) {
                    release(pds[i], disp);
                }
            }
        }
    }
    vkDestroyInstance(inst, NULL);
    return 0;
}

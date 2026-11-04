/* diag only: VK_KHR_display on each physical device (lavapipe, ...): the
 * displays, their modes and the display planes. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <vulkan/vulkan.h>

int main(void)
{
    const char *exts[] = { "VK_KHR_surface", "VK_KHR_display" };
    VkApplicationInfo app = { VK_STRUCTURE_TYPE_APPLICATION_INFO, NULL, "probe", 1, NULL, 0, VK_API_VERSION_1_0 };
    VkInstanceCreateInfo ci = { VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO, NULL, 0, &app, 0, NULL, 2, exts };
    VkInstance inst;
    VkResult r = vkCreateInstance(&ci, NULL, &inst);
    printf("vkCreateInstance(VK_KHR_surface, VK_KHR_display) = %d\n", r);
    if (r) return 1;
    uint32_t n = 0;
    vkEnumeratePhysicalDevices(inst, &n, NULL);
    VkPhysicalDevice devs[8];
    if (n > 8) n = 8;
    vkEnumeratePhysicalDevices(inst, &n, devs);
    PFN_vkGetPhysicalDeviceDisplayPropertiesKHR gdp = (PFN_vkGetPhysicalDeviceDisplayPropertiesKHR)vkGetInstanceProcAddr(inst, "vkGetPhysicalDeviceDisplayPropertiesKHR");
    PFN_vkGetPhysicalDeviceDisplayPlanePropertiesKHR gpp = (PFN_vkGetPhysicalDeviceDisplayPlanePropertiesKHR)vkGetInstanceProcAddr(inst, "vkGetPhysicalDeviceDisplayPlanePropertiesKHR");
    PFN_vkGetDisplayModePropertiesKHR gmp = (PFN_vkGetDisplayModePropertiesKHR)vkGetInstanceProcAddr(inst, "vkGetDisplayModePropertiesKHR");
    for (uint32_t i = 0; i < n; i++) {
        VkPhysicalDeviceProperties p;
        vkGetPhysicalDeviceProperties(devs[i], &p);
        uint32_t ne = 0;
        vkEnumerateDeviceExtensionProperties(devs[i], NULL, &ne, NULL);
        printf("device %u: %s (type %d)\n", i, p.deviceName, p.deviceType);
        uint32_t nd = 0;
        r = gdp(devs[i], &nd, NULL);
        printf("  displays %u (r=%d)\n", nd, r);
        VkDisplayPropertiesKHR dp[8];
        if (nd > 8) nd = 8;
        gdp(devs[i], &nd, dp);
        for (uint32_t d = 0; d < nd; d++) {
            printf("  display %s %ux%u\n", dp[d].displayName ? dp[d].displayName : "?", dp[d].physicalResolution.width, dp[d].physicalResolution.height);
            uint32_t nm = 0;
            gmp(devs[i], dp[d].display, &nm, NULL);
            printf("    modes %u\n", nm);
        }
        uint32_t np = 0;
        r = gpp(devs[i], &np, NULL);
        printf("  planes %u (r=%d)\n", np, r);
    }
    vkDestroyInstance(inst, NULL);
    return 0;
}

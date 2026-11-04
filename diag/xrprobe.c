/* Scratchpad-only probe (never committed): the OpenXR calls upstream's
 * SDL_openxrdyn.c, SDL_gpu_openxr.c and the Vulkan backend make, against
 * whatever runtime XR_RUNTIME_JSON names (Monado's null compositor here). */
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <vulkan/vulkan.h>
#define XR_USE_GRAPHICS_API_VULKAN 1
#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>

#define CHK(r, what) do { XrResult _r = (r); printf("%-40s %d\n", what, (int)_r); if (XR_FAILED(_r)) return 1; } while (0)

static PFN_xrGetInstanceProcAddr gipa;
#define LOADI(inst, name) PFN_##name name; gipa(inst, #name, (PFN_xrVoidFunction *)&name)

int main(int argc, char **argv)
{
    int use_layer = argc > 1 && strcmp(argv[1], "layer") == 0;
    void *lib = dlopen("libopenxr_loader.so.1", RTLD_NOW | RTLD_LOCAL);
    if (!lib) { printf("dlopen: %s\n", dlerror()); return 1; }
    gipa = (PFN_xrGetInstanceProcAddr)dlsym(lib, "xrGetInstanceProcAddr");
    PFN_xrEnumerateApiLayerProperties enumLayers = (PFN_xrEnumerateApiLayerProperties)dlsym(lib, "xrEnumerateApiLayerProperties");
    PFN_xrCreateInstance createInstance = (PFN_xrCreateInstance)dlsym(lib, "xrCreateInstance");
    PFN_xrEnumerateInstanceExtensionProperties enumExts = (PFN_xrEnumerateInstanceExtensionProperties)dlsym(lib, "xrEnumerateInstanceExtensionProperties");
    printf("syms %d %d %d %d\n", !!gipa, !!enumLayers, !!createInstance, !!enumExts);

    uint32_t n = 0;
    CHK(enumLayers(0, &n, NULL), "xrEnumerateApiLayerProperties");
    XrApiLayerProperties *layers = calloc(n ? n : 1, sizeof(*layers));
    for (uint32_t i = 0; i < n; i++) layers[i].type = XR_TYPE_API_LAYER_PROPERTIES;
    enumLayers(n, &n, layers);
    for (uint32_t i = 0; i < n; i++) printf("layer: %s\n", layers[i].layerName);

    CHK(enumExts(NULL, 0, &n, NULL), "xrEnumerateInstanceExtensionProperties");
    XrExtensionProperties *exts = calloc(n, sizeof(*exts));
    for (uint32_t i = 0; i < n; i++) exts[i].type = XR_TYPE_EXTENSION_PROPERTIES;
    enumExts(NULL, n, &n, exts);
    for (uint32_t i = 0; i < n; i++) printf("ext: %s %u\n", exts[i].extensionName, exts[i].extensionVersion);

    const char *extName = XR_KHR_VULKAN_ENABLE2_EXTENSION_NAME;
    const char *layerName = "XR_APILAYER_LUNARG_core_validation";
    XrInstanceCreateInfo ici = { XR_TYPE_INSTANCE_CREATE_INFO };
    ici.applicationInfo.apiVersion = XR_API_VERSION_1_0; /* upstream's default: 1.0.<header patch> */
    strcpy(ici.applicationInfo.applicationName, "SDL");
    strcpy(ici.applicationInfo.engineName, "SDLGPU");
    ici.enabledExtensionCount = 1;
    ici.enabledExtensionNames = &extName;
    ici.enabledApiLayerCount = use_layer ? 1 : 0;
    ici.enabledApiLayerNames = &layerName;
    XrInstance inst;
    printf("apiVersion %llx\n", (unsigned long long)ici.applicationInfo.apiVersion);
    CHK(createInstance(&ici, &inst), "xrCreateInstance");

    LOADI(inst, xrGetSystem); LOADI(inst, xrGetInstanceProperties); LOADI(inst, xrGetSystemProperties);
    LOADI(inst, xrGetVulkanGraphicsRequirements2KHR); LOADI(inst, xrCreateVulkanInstanceKHR);
    LOADI(inst, xrGetVulkanGraphicsDevice2KHR); LOADI(inst, xrCreateVulkanDeviceKHR);
    LOADI(inst, xrCreateSession); LOADI(inst, xrEnumerateSwapchainFormats); LOADI(inst, xrCreateSwapchain);
    LOADI(inst, xrEnumerateSwapchainImages); LOADI(inst, xrPollEvent); LOADI(inst, xrBeginSession);
    LOADI(inst, xrWaitFrame); LOADI(inst, xrBeginFrame); LOADI(inst, xrEndFrame); LOADI(inst, xrAcquireSwapchainImage);
    LOADI(inst, xrWaitSwapchainImage); LOADI(inst, xrReleaseSwapchainImage); LOADI(inst, xrCreateReferenceSpace);
    LOADI(inst, xrLocateViews); LOADI(inst, xrEnumerateViewConfigurationViews); LOADI(inst, xrRequestExitSession);
    LOADI(inst, xrEndSession); LOADI(inst, xrDestroySwapchain); LOADI(inst, xrDestroySession); LOADI(inst, xrDestroyInstance);
    LOADI(inst, xrDestroySpace);

    XrInstanceProperties ip = { XR_TYPE_INSTANCE_PROPERTIES };
    xrGetInstanceProperties(inst, &ip);
    printf("runtime: %s %llx\n", ip.runtimeName, (unsigned long long)ip.runtimeVersion);

    XrSystemGetInfo sgi = { XR_TYPE_SYSTEM_GET_INFO };
    sgi.formFactor = XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY;
    XrSystemId sys;
    CHK(xrGetSystem(inst, &sgi, &sys), "xrGetSystem");
    XrSystemProperties sp = { XR_TYPE_SYSTEM_PROPERTIES };
    xrGetSystemProperties(inst, sys, &sp);
    printf("system: %s vendor %u maxlayers %u maxswap %ux%u\n", sp.systemName, sp.vendorId, sp.graphicsProperties.maxLayerCount, sp.graphicsProperties.maxSwapchainImageWidth, sp.graphicsProperties.maxSwapchainImageHeight);

    XrGraphicsRequirementsVulkanKHR req = { XR_TYPE_GRAPHICS_REQUIREMENTS_VULKAN2_KHR };
    CHK(xrGetVulkanGraphicsRequirements2KHR(inst, sys, &req), "xrGetVulkanGraphicsRequirements2KHR");
    printf("vk min %llx max %llx\n", (unsigned long long)req.minApiVersionSupported, (unsigned long long)req.maxApiVersionSupported);

    void *vklib = dlopen("libvulkan.so.1", RTLD_NOW | RTLD_LOCAL);
    PFN_vkGetInstanceProcAddr vkgipa = (PFN_vkGetInstanceProcAddr)dlsym(vklib, "vkGetInstanceProcAddr");
    VkApplicationInfo ai = { VK_STRUCTURE_TYPE_APPLICATION_INFO };
    ai.pEngineName = "SDLGPU";
    ai.apiVersion = VK_MAKE_VERSION(1, 0, 0); /* upstream's default, ignoring the XR minimum */
    const char *vkexts[] = { "VK_KHR_surface", "VK_EXT_headless_surface", "VK_KHR_get_physical_device_properties2" };
    VkInstanceCreateInfo vici = { VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO };
    vici.pApplicationInfo = &ai;
    vici.enabledExtensionCount = 3;
    vici.ppEnabledExtensionNames = vkexts;
    XrVulkanInstanceCreateInfoKHR xvici = { XR_TYPE_VULKAN_INSTANCE_CREATE_INFO_KHR };
    xvici.systemId = sys;
    xvici.pfnGetInstanceProcAddr = vkgipa;
    xvici.vulkanCreateInfo = &vici;
    VkInstance vkinst; VkResult vr = 0;
    CHK(xrCreateVulkanInstanceKHR(inst, &xvici, &vkinst, &vr), "xrCreateVulkanInstanceKHR");
    printf("  vkResult %d\n", vr);

    XrVulkanGraphicsDeviceGetInfoKHR gdi = { XR_TYPE_VULKAN_GRAPHICS_DEVICE_GET_INFO_KHR };
    gdi.systemId = sys; gdi.vulkanInstance = vkinst;
    VkPhysicalDevice pd;
    CHK(xrGetVulkanGraphicsDevice2KHR(inst, &gdi, &pd), "xrGetVulkanGraphicsDevice2KHR");
    PFN_vkGetPhysicalDeviceProperties gpdp = (PFN_vkGetPhysicalDeviceProperties)vkgipa(vkinst, "vkGetPhysicalDeviceProperties");
    VkPhysicalDeviceProperties pdp; gpdp(pd, &pdp);
    printf("  physical device: %s api %x\n", pdp.deviceName, pdp.apiVersion);

    float prio = 1.0f;
    VkDeviceQueueCreateInfo qci = { VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO };
    qci.queueFamilyIndex = 0; qci.queueCount = 1; qci.pQueuePriorities = &prio;
    const char *devexts[] = { "VK_KHR_swapchain" };
    VkDeviceCreateInfo dci = { VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO };
    dci.queueCreateInfoCount = 1; dci.pQueueCreateInfos = &qci;
    dci.enabledExtensionCount = 1; dci.ppEnabledExtensionNames = devexts;
    XrVulkanDeviceCreateInfoKHR xdci = { XR_TYPE_VULKAN_DEVICE_CREATE_INFO_KHR };
    xdci.systemId = sys; xdci.pfnGetInstanceProcAddr = vkgipa; xdci.vulkanPhysicalDevice = pd; xdci.vulkanCreateInfo = &dci;
    VkDevice dev;
    CHK(xrCreateVulkanDeviceKHR(inst, &xdci, &dev, &vr), "xrCreateVulkanDeviceKHR");
    printf("  vkResult %d\n", vr);

    XrGraphicsBindingVulkanKHR gb = { XR_TYPE_GRAPHICS_BINDING_VULKAN_KHR };
    gb.instance = vkinst; gb.physicalDevice = pd; gb.device = dev; gb.queueFamilyIndex = 0; gb.queueIndex = 0;
    XrSessionCreateInfo sci = { XR_TYPE_SESSION_CREATE_INFO };
    sci.next = &gb; sci.systemId = sys;
    XrSession session;
    CHK(xrCreateSession(inst, &sci, &session), "xrCreateSession");

    CHK(xrEnumerateSwapchainFormats(session, 0, &n, NULL), "xrEnumerateSwapchainFormats");
    int64_t *fmts = calloc(n, sizeof(*fmts));
    xrEnumerateSwapchainFormats(session, n, &n, fmts);
    printf("formats:");
    for (uint32_t i = 0; i < n; i++) printf(" %lld", (long long)fmts[i]);
    printf("\n");

    uint32_t nviews = 0;
    xrEnumerateViewConfigurationViews(inst, sys, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, 0, &nviews, NULL);
    XrViewConfigurationView vcv[2] = { { XR_TYPE_VIEW_CONFIGURATION_VIEW }, { XR_TYPE_VIEW_CONFIGURATION_VIEW } };
    xrEnumerateViewConfigurationViews(inst, sys, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, 2, &nviews, vcv);
    printf("views %u: %ux%u (max %ux%u) samples %u\n", nviews, vcv[0].recommendedImageRectWidth, vcv[0].recommendedImageRectHeight, vcv[0].maxImageRectWidth, vcv[0].maxImageRectHeight, vcv[0].recommendedSwapchainSampleCount);

    XrSwapchainCreateInfo swci = { XR_TYPE_SWAPCHAIN_CREATE_INFO };
    swci.usageFlags = XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT | XR_SWAPCHAIN_USAGE_TRANSFER_SRC_BIT | XR_SWAPCHAIN_USAGE_SAMPLED_BIT;
    swci.format = VK_FORMAT_R8G8B8A8_UNORM;
    swci.sampleCount = 1; swci.width = 64; swci.height = 64; swci.faceCount = 1; swci.arraySize = 2; swci.mipCount = 1;
    XrSwapchain swapchain;
    CHK(xrCreateSwapchain(session, &swci, &swapchain), "xrCreateSwapchain (array 2)");
    uint32_t nimg = 0;
    CHK(xrEnumerateSwapchainImages(swapchain, 0, &nimg, NULL), "xrEnumerateSwapchainImages");
    XrSwapchainImageVulkan2KHR *imgs = calloc(nimg, sizeof(*imgs));
    for (uint32_t i = 0; i < nimg; i++) imgs[i].type = XR_TYPE_SWAPCHAIN_IMAGE_VULKAN2_KHR;
    CHK(xrEnumerateSwapchainImages(swapchain, nimg, &nimg, (XrSwapchainImageBaseHeader *)imgs), "xrEnumerateSwapchainImages (2)");
    printf("  images %u, first %p\n", nimg, (void *)imgs[0].image);

    XrReferenceSpaceCreateInfo rsci = { XR_TYPE_REFERENCE_SPACE_CREATE_INFO };
    rsci.referenceSpaceType = XR_REFERENCE_SPACE_TYPE_LOCAL;
    rsci.poseInReferenceSpace.orientation.w = 1;
    XrSpace space;
    CHK(xrCreateReferenceSpace(session, &rsci, &space), "xrCreateReferenceSpace");

    int running = 0, frames = 0, done = 0;
    for (int iter = 0; iter < 2000 && !done; iter++) {
        XrEventDataBuffer ev = { XR_TYPE_EVENT_DATA_BUFFER };
        while (xrPollEvent(inst, &ev) == XR_SUCCESS) {
            if (ev.type == XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED) {
                XrSessionState st = ((XrEventDataSessionStateChanged *)&ev)->state;
                printf("state %d\n", (int)st);
                if (st == XR_SESSION_STATE_READY) {
                    XrSessionBeginInfo sbi = { XR_TYPE_SESSION_BEGIN_INFO };
                    sbi.primaryViewConfigurationType = XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
                    CHK(xrBeginSession(session, &sbi), "xrBeginSession");
                    running = 1;
                } else if (st == XR_SESSION_STATE_STOPPING) {
                    CHK(xrEndSession(session), "xrEndSession");
                    running = 0; done = 1;
                } else if (st == XR_SESSION_STATE_EXITING || st == XR_SESSION_STATE_LOSS_PENDING) {
                    done = 1;
                }
            }
            ev.type = XR_TYPE_EVENT_DATA_BUFFER;
        }
        if (!running) continue;
        XrFrameWaitInfo fwi = { XR_TYPE_FRAME_WAIT_INFO };
        XrFrameState fs = { XR_TYPE_FRAME_STATE };
        if (XR_FAILED(xrWaitFrame(session, &fwi, &fs))) { printf("xrWaitFrame failed\n"); return 1; }
        XrFrameBeginInfo fbi = { XR_TYPE_FRAME_BEGIN_INFO };
        if (XR_FAILED(xrBeginFrame(session, &fbi))) { printf("xrBeginFrame failed\n"); return 1; }
        XrViewLocateInfo vli = { XR_TYPE_VIEW_LOCATE_INFO };
        vli.viewConfigurationType = XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO; vli.displayTime = fs.predictedDisplayTime; vli.space = space;
        XrViewState vs = { XR_TYPE_VIEW_STATE };
        XrView views[2] = { { XR_TYPE_VIEW }, { XR_TYPE_VIEW } };
        uint32_t nv = 0;
        xrLocateViews(session, &vli, &vs, 2, &nv, views);
        uint32_t idx;
        XrSwapchainImageAcquireInfo sai = { XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO };
        xrAcquireSwapchainImage(swapchain, &sai, &idx);
        XrSwapchainImageWaitInfo swi = { XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO };
        swi.timeout = XR_INFINITE_DURATION;
        xrWaitSwapchainImage(swapchain, &swi);
        XrSwapchainImageReleaseInfo sri = { XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO };
        xrReleaseSwapchainImage(swapchain, &sri);
        XrCompositionLayerProjectionView pv[2];
        for (int e = 0; e < 2; e++) {
            memset(&pv[e], 0, sizeof(pv[e]));
            pv[e].type = XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW;
            pv[e].pose = views[e].pose; pv[e].fov = views[e].fov;
            pv[e].subImage.swapchain = swapchain; pv[e].subImage.imageRect.extent.width = 64; pv[e].subImage.imageRect.extent.height = 64;
            pv[e].subImage.imageArrayIndex = e;
        }
        XrCompositionLayerProjection layer = { XR_TYPE_COMPOSITION_LAYER_PROJECTION };
        layer.space = space; layer.viewCount = 2; layer.views = pv;
        const XrCompositionLayerBaseHeader *lp = (XrCompositionLayerBaseHeader *)&layer;
        XrFrameEndInfo fei = { XR_TYPE_FRAME_END_INFO };
        fei.displayTime = fs.predictedDisplayTime; fei.environmentBlendMode = XR_ENVIRONMENT_BLEND_MODE_OPAQUE;
        fei.layerCount = fs.shouldRender ? 1 : 0; fei.layers = &lp;
        XrResult er = xrEndFrame(session, &fei);
        if (XR_FAILED(er)) { printf("xrEndFrame %d\n", er); return 1; }
        if (++frames == 1 || frames == 30) printf("frame %d shouldRender %d period %lld views %u\n", frames, fs.shouldRender, (long long)fs.predictedDisplayPeriod, nv);
        if (frames == 30) CHK(xrRequestExitSession(session), "xrRequestExitSession");
    }
    printf("frames %d\n", frames);
    CHK(xrDestroySpace(space), "xrDestroySpace");
    CHK(xrDestroySwapchain(swapchain), "xrDestroySwapchain");
    CHK(xrDestroySession(session), "xrDestroySession");
    CHK(xrDestroyInstance(inst), "xrDestroyInstance");
    printf("ok\n");
    return 0;
}

/* diag only (never committed to a PR branch): on the Windows runner, the
 * Khronos loader (openxr_loader.dll, loaded by name as upstream does) and
 * its runtime discovery, and DXGI's adapters and WARP's LUID. mingw-w64. */
#define COBJMACROS
#include <windows.h>
#include <dxgi1_4.h>
#include <stdio.h>
#include <openxr/openxr.h>

int main(int argc, char **argv)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    if (argc > 1 && strcmp(argv[1], "dxgi") == 0) {
        IDXGIFactory4 *f4 = NULL;
        IDXGIFactory1 *f1 = NULL;
        HRESULT hr = CreateDXGIFactory1(&IID_IDXGIFactory1, (void **)&f1);
        printf("CreateDXGIFactory1 0x%08lx\n", (unsigned long)hr);
        if (FAILED(hr)) return 1;
        IDXGIAdapter1 *a;
        for (UINT i = 0; IDXGIFactory1_EnumAdapters1(f1, i, &a) != DXGI_ERROR_NOT_FOUND; i++) {
            DXGI_ADAPTER_DESC1 d;
            IDXGIAdapter1_GetDesc1(a, &d);
            printf("EnumAdapters1 %u: %ls vendor 0x%04x device 0x%04x flags 0x%x luid %08lx:%08lx\n", i, d.Description, d.VendorId, d.DeviceId, d.Flags, (long)d.AdapterLuid.HighPart, (unsigned long)d.AdapterLuid.LowPart);
            IDXGIAdapter1_Release(a);
        }
        hr = IDXGIFactory1_QueryInterface(f1, &IID_IDXGIFactory4, (void **)&f4);
        printf("IDXGIFactory4 0x%08lx\n", (unsigned long)hr);
        if (SUCCEEDED(hr)) {
            IDXGIAdapter1 *w = NULL;
            hr = IDXGIFactory4_EnumWarpAdapter(f4, &IID_IDXGIAdapter1, (void **)&w);
            printf("EnumWarpAdapter 0x%08lx\n", (unsigned long)hr);
            if (SUCCEEDED(hr)) {
                DXGI_ADAPTER_DESC1 d;
                IDXGIAdapter1_GetDesc1(w, &d);
                printf("WARP: %ls vendor 0x%04x device 0x%04x flags 0x%x luid %08lx:%08lx\n", d.Description, d.VendorId, d.DeviceId, d.Flags, (long)d.AdapterLuid.HighPart, (unsigned long)d.AdapterLuid.LowPart);
            }
        }
        return 0;
    }

    HMODULE lib = LoadLibraryA("openxr_loader.dll");
    if (!lib) { printf("LoadLibrary(openxr_loader.dll): error %lu\n", GetLastError()); return 1; }
    char path[MAX_PATH];
    GetModuleFileNameA(lib, path, sizeof(path));
    printf("loaded %s\n", path);
    PFN_xrGetInstanceProcAddr gipa = (PFN_xrGetInstanceProcAddr)(void *)GetProcAddress(lib, "xrGetInstanceProcAddr");
    PFN_xrEnumerateApiLayerProperties layers = (PFN_xrEnumerateApiLayerProperties)(void *)GetProcAddress(lib, "xrEnumerateApiLayerProperties");
    PFN_xrCreateInstance create = (PFN_xrCreateInstance)(void *)GetProcAddress(lib, "xrCreateInstance");
    PFN_xrEnumerateInstanceExtensionProperties exts = (PFN_xrEnumerateInstanceExtensionProperties)(void *)GetProcAddress(lib, "xrEnumerateInstanceExtensionProperties");
    printf("syms %d %d %d %d\n", !!gipa, !!layers, !!create, !!exts);
    uint32_t n = 0;
    XrResult r = layers(0, &n, NULL);
    printf("xrEnumerateApiLayerProperties %d (%u)\n", (int)r, n);
    r = exts(NULL, 0, &n, NULL);
    printf("xrEnumerateInstanceExtensionProperties %d (%u)\n", (int)r, n);
    return 0;
}

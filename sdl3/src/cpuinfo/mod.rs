// Rust translation of src/cpuinfo/SDL_cpuinfo.c from Simple DirectMedia Layer.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! CPU feature detection for SDL: SIMD instruction sets, core count, cache
//! line size, RAM and page size.
//!
//! Upstream probes features with inline `cpuid`/`xgetbv` assembly, signal
//! handlers and `getauxval`; here `std::arch`'s runtime detection macros do
//! that work (they perform the same CPUID checks plus the OS-support checks
//! upstream does with `xgetbv`). `SDL_HINT_CPU_FEATURE_MASK` is honoured.
//!
//! Core count, RAM and page size come from `/sys` and `/proc` on Linux and
//! Android (pure Rust reads of what glibc's `sysconf` reads); other
//! platforms get the core count from `std` and report RAM/page size as
//! unknown (0) until the platform layer provides them.

use std::sync::atomic::{AtomicI32, AtomicU32, AtomicUsize, Ordering};

use crate::hints;

/// A guess for the cacheline size used for padding. Translation of `SDL_CACHELINE_SIZE`.
pub const CACHELINE_SIZE: i32 = 128;

/// A set of CPU features, as detected and masked by [`cpu_features`].
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct CpuFeatures(pub u32);

impl CpuFeatures {
    /// Translation of `CPU_HAS_ALTIVEC`.
    pub const ALTIVEC: CpuFeatures = CpuFeatures(1 << 0);
    /// Translation of `CPU_HAS_MMX`.
    pub const MMX: CpuFeatures = CpuFeatures(1 << 1);
    /// Translation of `CPU_HAS_SSE`.
    pub const SSE: CpuFeatures = CpuFeatures(1 << 2);
    /// Translation of `CPU_HAS_SSE2`.
    pub const SSE2: CpuFeatures = CpuFeatures(1 << 3);
    /// Translation of `CPU_HAS_SSE3`.
    pub const SSE3: CpuFeatures = CpuFeatures(1 << 4);
    /// Translation of `CPU_HAS_SSE41`.
    pub const SSE41: CpuFeatures = CpuFeatures(1 << 5);
    /// Translation of `CPU_HAS_SSE42`.
    pub const SSE42: CpuFeatures = CpuFeatures(1 << 6);
    /// Translation of `CPU_HAS_AVX`.
    pub const AVX: CpuFeatures = CpuFeatures(1 << 7);
    /// Translation of `CPU_HAS_AVX2`.
    pub const AVX2: CpuFeatures = CpuFeatures(1 << 8);
    /// Translation of `CPU_HAS_NEON`.
    pub const NEON: CpuFeatures = CpuFeatures(1 << 9);
    /// Translation of `CPU_HAS_AVX512F`.
    pub const AVX512F: CpuFeatures = CpuFeatures(1 << 10);
    /// Translation of `CPU_HAS_ARM_SIMD`.
    pub const ARM_SIMD: CpuFeatures = CpuFeatures(1 << 11);
    /// Translation of `CPU_HAS_LSX`.
    pub const LSX: CpuFeatures = CpuFeatures(1 << 12);
    /// Translation of `CPU_HAS_LASX`.
    pub const LASX: CpuFeatures = CpuFeatures(1 << 13);
    /// Translation of `CPU_HAS_SVE2`.
    pub const SVE2: CpuFeatures = CpuFeatures(1 << 14);

    /// True if every feature in `other` is present.
    pub const fn contains(self, other: CpuFeatures) -> bool {
        self.0 & other.0 == other.0
    }
}

// ---------------------------------------------------------------------------
// x86 CPUID
// ---------------------------------------------------------------------------

/// `cpuid(func, a, b, c, d)` with ecx cleared, as upstream's macro does.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn cpuid(func: u32) -> (u32, u32, u32, u32) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::__cpuid_count;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::__cpuid_count;
    #[allow(unused_unsafe)]
    // SAFETY: CPUID is available on every x86-64 CPU and on every x86 CPU
    // Rust targets (i586+; upstream's CPU_haveCPUID() check exists for 486s).
    // On newer toolchains the intrinsic is safe and this block is a no-op.
    let r = unsafe { __cpuid_count(func, 0) };
    (r.eax, r.ebx, r.ecx, r.edx)
}

/// Translation of `CPU_CPUIDMaxFunction` (computed by `CPU_calcCPUIDFeatures()`).
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn cpuid_max_function() -> u32 {
    cpuid(0).0
}

// ---------------------------------------------------------------------------
// Per-feature probes (the CPU_have*() functions)
// ---------------------------------------------------------------------------

fn cpu_have_altivec() -> bool {
    // AltiVec runtime detection is unstable in Rust; use the compile-time target feature.
    cfg!(all(
        any(target_arch = "powerpc", target_arch = "powerpc64"),
        target_feature = "altivec"
    ))
}

macro_rules! x86_feature {
    ($name:tt) => {{
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            std::arch::is_x86_feature_detected!($name)
        }
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
        {
            false
        }
    }};
}

fn cpu_have_mmx() -> bool {
    x86_feature!("mmx")
}
fn cpu_have_sse() -> bool {
    x86_feature!("sse")
}
fn cpu_have_sse2() -> bool {
    x86_feature!("sse2")
}
fn cpu_have_sse3() -> bool {
    x86_feature!("sse3")
}
fn cpu_have_sse41() -> bool {
    x86_feature!("sse4.1")
}
fn cpu_have_sse42() -> bool {
    x86_feature!("sse4.2")
}
/// `CPU_OSSavesYMM && (CPU_CPUIDFeatures[2] & 0x10000000)`
fn cpu_have_avx() -> bool {
    x86_feature!("avx")
}
/// `CPU_OSSavesYMM && CPUIDMaxFunction >= 7 && (cpuid(7).b & 0x00000020)`
fn cpu_have_avx2() -> bool {
    x86_feature!("avx2")
}
/// `CPU_OSSavesZMM && CPUIDMaxFunction >= 7 && (cpuid(7).b & 0x00010000)`
fn cpu_have_avx512f() -> bool {
    x86_feature!("avx512f")
}

/// The ARMv6 SIMD extensions (32-bit ARM only).
fn cpu_have_arm_simd() -> bool {
    cfg!(all(target_arch = "arm", target_feature = "v6"))
}

fn cpu_have_neon() -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        // ARMv8 (and later) always has NEON, but the hardware may disable it.
        std::arch::is_aarch64_feature_detected!("neon")
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        cfg!(all(target_arch = "arm", target_feature = "neon"))
    }
}

fn cpu_have_sve2() -> bool {
    #[cfg(target_arch = "aarch64")]
    {
        std::arch::is_aarch64_feature_detected!("sve2")
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        false
    }
}

fn cpu_have_lsx() -> bool {
    cfg!(all(target_arch = "loongarch64", target_feature = "lsx"))
}

fn cpu_have_lasx() -> bool {
    cfg!(all(target_arch = "loongarch64", target_feature = "lasx"))
}

// ---------------------------------------------------------------------------
// Public queries
// ---------------------------------------------------------------------------

/// Translation of `SDL_NumLogicalCPUCores`.
static NUM_LOGICAL_CPU_CORES: AtomicI32 = AtomicI32::new(0);

/// Parse a Linux CPU list (`/sys/devices/system/cpu/online`: "0-3,5,7-8").
fn count_cpu_list(list: &str) -> i32 {
    list.trim()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|range| match range.split_once('-') {
            Some((a, b)) => match (a.trim().parse::<i32>(), b.trim().parse::<i32>()) {
                (Ok(a), Ok(b)) if b >= a => b - a + 1,
                _ => 0,
            },
            None => range.trim().parse::<i32>().map(|_| 1).unwrap_or(0),
        })
        .sum()
}

/// The number of logical CPU cores available. Translation of `SDL_GetNumLogicalCPUCores()`.
pub fn num_logical_cpu_cores() -> i32 {
    let mut n = NUM_LOGICAL_CPU_CORES.load(Ordering::Relaxed);
    if n == 0 {
        // sysconf(_SC_NPROCESSORS_ONLN): what glibc reads on Linux
        if cfg!(any(target_os = "linux", target_os = "android")) {
            if let Ok(list) = std::fs::read_to_string("/sys/devices/system/cpu/online") {
                n = count_cpu_list(&list);
            }
        }
        if n <= 0 {
            n = std::thread::available_parallelism()
                .map(|p| p.get() as i32)
                .unwrap_or(0);
        }
        // There has to be at least 1, right? :)
        if n <= 0 {
            n = 1;
        }
        NUM_LOGICAL_CPU_CORES.store(n, Ordering::Relaxed);
    }
    n
}

/// The CPU vendor string from CPUID leaf 0 ("GenuineIntel", "AuthenticAMD",
/// ...), or "Unknown". Translation of the static `SDL_GetCPUType()`.
pub fn cpu_type() -> String {
    // Oh, such a sweet sweet trick, just not very useful. :)
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if cpuid_max_function() > 0 {
            // do we have CPUID at all?
            let (_, b, c, d) = cpuid(0x00000000);
            let mut bytes = Vec::with_capacity(12);
            bytes.extend_from_slice(&b.to_le_bytes());
            bytes.extend_from_slice(&d.to_le_bytes());
            bytes.extend_from_slice(&c.to_le_bytes());
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            if end > 0 {
                return String::from_utf8_lossy(&bytes[..end]).into_owned();
            }
        }
    }
    "Unknown".to_owned()
}

/// The L1 cache line size of the CPU, in bytes. Translation of `SDL_GetCPUCacheLineSize()`.
pub fn cpu_cache_line_size() -> i32 {
    let cpu_type = cpu_type();
    let mut cacheline_size = CACHELINE_SIZE; // initial guess

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if cpu_type == "GenuineIntel" || cpu_type == "CentaurHauls" || cpu_type == "  Shanghai  " {
            let (_, b, _, _) = cpuid(0x00000001);
            return (((b >> 8) & 0xff) * 8) as i32;
        } else if cpu_type == "AuthenticAMD" || cpu_type == "HygonGenuine" {
            let (_, _, c, _) = cpuid(0x80000005);
            return (c & 0xff) as i32;
        }
    }
    let _ = &cpu_type;

    if cfg!(any(target_os = "linux", target_os = "android")) {
        // (sysconf(_SC_LEVEL1_DCACHE_LINESIZE) reads the same file on glibc)
        if let Ok(s) =
            std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cache/index0/coherency_line_size")
        {
            if let Ok(size) = s.trim().parse::<i32>() {
                if size > 0 {
                    cacheline_size = size;
                }
            }
        }
    }
    cacheline_size
}

/// Translation of `SDL_CPUFEATURES_RESET_VALUE`.
const CPUFEATURES_RESET_VALUE: u32 = 0xFFFFFFFF;
/// Translation of `SDL_CPUFeatures`.
static SDL_CPU_FEATURES: AtomicU32 = AtomicU32::new(CPUFEATURES_RESET_VALUE);
/// Translation of `SDL_SIMDAlignment`.
static SDL_SIMD_ALIGNMENT: AtomicUsize = AtomicUsize::new(0xFFFFFFFF);

/// Translation of `ref_string_equals()`.
fn ref_string_equals(reference: &str, test: &str) -> bool {
    reference == test
}

/// Parse `SDL_HINT_CPU_FEATURE_MASK` ("-avx2,+sse41", "all", ...).
/// Translation of `SDL_CPUFeatureMaskFromHint()`.
fn cpu_feature_mask_from_hint() -> u32 {
    let mut result_mask = CPUFEATURES_RESET_VALUE;

    if let Some(hint) = hints::get(hints::CPU_FEATURE_MASK) {
        for spot in hint.split(',') {
            if spot.is_empty() {
                continue;
            }
            let (add_spot_mask, spot) = match spot.as_bytes()[0] {
                b'+' => (true, &spot[1..]),
                b'-' => (false, &spot[1..]),
                _ => (true, spot),
            };
            let spot_mask = if ref_string_equals("all", spot) {
                CPUFEATURES_RESET_VALUE
            } else {
                let feature = match spot {
                    "altivec" => CpuFeatures::ALTIVEC,
                    "mmx" => CpuFeatures::MMX,
                    "sse" => CpuFeatures::SSE,
                    "sse2" => CpuFeatures::SSE2,
                    "sse3" => CpuFeatures::SSE3,
                    "sse41" => CpuFeatures::SSE41,
                    "sse42" => CpuFeatures::SSE42,
                    "avx" => CpuFeatures::AVX,
                    "avx2" => CpuFeatures::AVX2,
                    "avx512f" => CpuFeatures::AVX512F,
                    "arm-simd" => CpuFeatures::ARM_SIMD,
                    "neon" => CpuFeatures::NEON,
                    "lsx" => CpuFeatures::LSX,
                    "lasx" => CpuFeatures::LASX,
                    "sve2" => CpuFeatures::SVE2,
                    // Ignore unknown/incorrect cpu feature(s)
                    _ => continue,
                };
                feature.0
            };
            if add_spot_mask {
                result_mask |= spot_mask;
            } else {
                result_mask &= !spot_mask;
            }
        }
    }
    result_mask
}

/// A feature probe, the flag it sets and the SIMD alignment it implies.
type Probe = (fn() -> bool, CpuFeatures, usize);

/// The detected CPU features, masked by `SDL_HINT_CPU_FEATURE_MASK`.
/// Translation of the static `SDL_GetCPUFeatures()`.
pub fn cpu_features() -> CpuFeatures {
    let cached = SDL_CPU_FEATURES.load(Ordering::Acquire);
    if cached != CPUFEATURES_RESET_VALUE {
        return CpuFeatures(cached);
    }

    let mut features = 0u32;
    let mut simd_alignment = std::mem::size_of::<*const ()>(); // a good safe base value
    let probes: [Probe; 15] = [
        (cpu_have_altivec, CpuFeatures::ALTIVEC, 16),
        (cpu_have_mmx, CpuFeatures::MMX, 8),
        (cpu_have_sse, CpuFeatures::SSE, 16),
        (cpu_have_sse2, CpuFeatures::SSE2, 16),
        (cpu_have_sse3, CpuFeatures::SSE3, 16),
        (cpu_have_sse41, CpuFeatures::SSE41, 16),
        (cpu_have_sse42, CpuFeatures::SSE42, 16),
        (cpu_have_avx, CpuFeatures::AVX, 32),
        (cpu_have_avx2, CpuFeatures::AVX2, 32),
        (cpu_have_avx512f, CpuFeatures::AVX512F, 64),
        (cpu_have_arm_simd, CpuFeatures::ARM_SIMD, 16),
        (cpu_have_neon, CpuFeatures::NEON, 16),
        (cpu_have_lsx, CpuFeatures::LSX, 16),
        (cpu_have_lasx, CpuFeatures::LASX, 32),
        (cpu_have_sve2, CpuFeatures::SVE2, 16),
    ];
    for (have, flag, align) in probes {
        if have() {
            features |= flag.0;
            simd_alignment = simd_alignment.max(align);
        }
    }

    features &= cpu_feature_mask_from_hint();
    SDL_SIMD_ALIGNMENT.store(simd_alignment, Ordering::Release);
    SDL_CPU_FEATURES.store(features, Ordering::Release);
    CpuFeatures(features)
}

/// Forget the cached features (e.g. after changing `SDL_HINT_CPU_FEATURE_MASK`).
/// Translation of `SDL_QuitCPUInfo()`.
pub(crate) fn quit_cpu_info() {
    SDL_CPU_FEATURES.store(CPUFEATURES_RESET_VALUE, Ordering::Release);
}

macro_rules! has_feature {
    ($($(#[$m:meta])* $fn:ident => $flag:ident;)*) => {
        $( $(#[$m])* pub fn $fn() -> bool { cpu_features().contains(CpuFeatures::$flag) } )*
    };
}

has_feature! {
    /// Translation of `SDL_HasAltiVec()`.
    has_altivec => ALTIVEC;
    /// Translation of `SDL_HasMMX()`.
    has_mmx => MMX;
    /// Translation of `SDL_HasSSE()`.
    has_sse => SSE;
    /// Translation of `SDL_HasSSE2()`.
    has_sse2 => SSE2;
    /// Translation of `SDL_HasSSE3()`.
    has_sse3 => SSE3;
    /// Translation of `SDL_HasSSE41()`.
    has_sse41 => SSE41;
    /// Translation of `SDL_HasSSE42()`.
    has_sse42 => SSE42;
    /// Translation of `SDL_HasAVX()`.
    has_avx => AVX;
    /// Translation of `SDL_HasAVX2()`.
    has_avx2 => AVX2;
    /// Translation of `SDL_HasAVX512F()`.
    has_avx512f => AVX512F;
    /// Translation of `SDL_HasARMSIMD()`.
    has_arm_simd => ARM_SIMD;
    /// Translation of `SDL_HasNEON()`.
    has_neon => NEON;
    /// Translation of `SDL_HasLSX()`.
    has_lsx => LSX;
    /// Translation of `SDL_HasLASX()`.
    has_lasx => LASX;
    /// Translation of `SDL_HasSVE2()`.
    has_sve2 => SVE2;
}

/// Translation of `SDL_SystemRAM`.
static SYSTEM_RAM: AtomicI32 = AtomicI32::new(0);

/// The amount of RAM configured in the system, in MiB (0 if unknown).
/// Translation of `SDL_GetSystemRAM()`.
pub fn system_ram() -> i32 {
    let mut ram = SYSTEM_RAM.load(Ordering::Relaxed);
    if ram == 0 {
        // sysconf(_SC_PHYS_PAGES) * sysconf(_SC_PAGESIZE): glibc reads sysinfo()'s
        // totalram, which /proc/meminfo reports as MemTotal (in KiB).
        if cfg!(any(target_os = "linux", target_os = "android")) {
            if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
                if let Some(line) = meminfo.lines().find(|l| l.starts_with("MemTotal:")) {
                    let kib: i64 = line
                        .trim_start_matches("MemTotal:")
                        .trim()
                        .trim_end_matches("kB")
                        .trim()
                        .parse()
                        .unwrap_or(0);
                    ram = (kib / 1024) as i32;
                }
            }
        }
        SYSTEM_RAM.store(ram, Ordering::Relaxed);
    }
    ram
}

/// Translation of `SDL_SystemPageSize`.
static SYSTEM_PAGE_SIZE: AtomicI32 = AtomicI32::new(-1);

/// Read `AT_PAGESZ` from the auxiliary vector (what `sysconf(_SC_PAGESIZE)` returns).
fn auxv_page_size() -> Option<i32> {
    const AT_PAGESZ: u64 = 6;
    let auxv = std::fs::read("/proc/self/auxv").ok()?;
    let word = std::mem::size_of::<usize>();
    for pair in auxv.chunks_exact(word * 2) {
        let read = |b: &[u8]| -> u64 {
            let mut buf = [0u8; 8];
            buf[..word].copy_from_slice(b);
            u64::from_ne_bytes(buf)
        };
        let (key, value) = (read(&pair[..word]), read(&pair[word..]));
        if key == AT_PAGESZ {
            return i32::try_from(value).ok();
        }
        if key == 0 {
            break; // AT_NULL
        }
    }
    None
}

/// The system page size, in bytes (0 if unknown). Translation of `SDL_GetSystemPageSize()`.
pub fn system_page_size() -> i32 {
    let mut page_size = SYSTEM_PAGE_SIZE.load(Ordering::Relaxed);
    if page_size == -1 {
        if cfg!(any(target_os = "linux", target_os = "android")) {
            page_size = auxv_page_size().unwrap_or(-1);
        }
        if page_size < 0 {
            // in case we got a weird result somewhere, or no better information, force it to 0.
            page_size = 0; // unknown page size, sorry.
        }
        SYSTEM_PAGE_SIZE.store(page_size, Ordering::Relaxed);
    }
    page_size
}

/// The alignment SIMD-friendly buffers need on this CPU, in bytes.
/// Translation of `SDL_GetSIMDAlignment()`.
pub fn simd_alignment() -> usize {
    if SDL_SIMD_ALIGNMENT.load(Ordering::Acquire) == 0xFFFFFFFF {
        cpu_features(); // make sure this has been calculated
    }
    let alignment = SDL_SIMD_ALIGNMENT.load(Ordering::Acquire);
    crate::sdl_assert!(alignment != 0);
    alignment
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        // (feature_mask_hint masks the cached features meanwhile)
        let _l = crate::test_support::test_lock();
        assert!(num_logical_cpu_cores() >= 1);
        let line = cpu_cache_line_size();
        assert!(line > 0 && (line as u32).is_power_of_two(), "{line}");
        assert!(simd_alignment() >= std::mem::size_of::<*const ()>());
        if cfg!(target_os = "linux") {
            assert!(system_ram() > 0);
            assert!(system_page_size() >= 4096);
        }
        assert!(!cpu_type().is_empty());
        if cfg!(target_arch = "x86_64") {
            assert!(has_sse2(), "SSE2 is part of x86-64");
            assert!(has_mmx() && has_sse());
            assert!(!has_neon());
        }
        if cfg!(target_arch = "aarch64") {
            assert!(has_neon());
        }
    }

    #[test]
    fn cpu_list_parsing() {
        assert_eq!(count_cpu_list("0-3\n"), 4);
        assert_eq!(count_cpu_list("0-3,5,7-8"), 7);
        assert_eq!(count_cpu_list("0"), 1);
        assert_eq!(count_cpu_list(""), 0);
    }

    #[test]
    fn feature_mask_hint() {
        let _l = crate::test_support::test_lock();
        quit_cpu_info();
        let all = cpu_features();
        hints::set(hints::CPU_FEATURE_MASK, "-sse2,-neon,bogus,+mmx").unwrap();
        quit_cpu_info();
        let masked = cpu_features();
        assert!(!masked.contains(CpuFeatures::SSE2));
        assert!(!masked.contains(CpuFeatures::NEON));
        assert_eq!(
            masked.contains(CpuFeatures::MMX),
            all.contains(CpuFeatures::MMX),
            "+ can't add what the CPU lacks"
        );
        hints::set(hints::CPU_FEATURE_MASK, "-all,+sse").unwrap();
        quit_cpu_info();
        assert_eq!(cpu_features().0 & !CpuFeatures::SSE.0, 0);
        hints::reset(hints::CPU_FEATURE_MASK);
        quit_cpu_info();
        assert_eq!(cpu_features(), all);
    }
}

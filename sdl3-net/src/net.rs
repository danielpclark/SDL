// Rust translation of src/SDL_net.c from SDL_net.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The whole library: `SDL_net.c`, in upstream's order.
//!
//! The C keeps one translation unit with the platform parts under
//! `#ifdef`; so does this file, with `#[cfg]`. The raw platform calls come
//! from [`sys`](crate::sys).

use std::cell::RefCell;
use std::ffi::CString;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use sdl3::properties::Properties;
use sdl3::thread::{Condition, InitState, ReentrantMutex, Thread};
use sdl3::{Error, Result};

use crate::sys::{
    self, c_char, c_int, pollfd, AddrInfo, AddressStorage, SockAddr, SockLen, Socket, AF_INET,
    AF_INET6, AF_UNSPEC, AI_NUMERICHOST, AI_NUMERICSERV, AI_PASSIVE, INVALID_SOCKET,
    NI_NUMERICHOST, NI_NUMERICSERV, POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT, SOCKET_ERROR,
    SOCK_DGRAM, SOCK_STREAM,
};

/// `#define NET_PROP_SERVER_REUSEADDR_BOOLEAN "NET.server.reuseaddr"`
pub const PROP_SERVER_REUSEADDR_BOOLEAN: &str = "NET.server.reuseaddr";
/// `#define NET_PROP_DATAGRAM_SOCKET_REUSEADDR_BOOLEAN "NET.datagram_socket.reuseaddr"`
pub const PROP_DATAGRAM_SOCKET_REUSEADDR_BOOLEAN: &str = "NET.datagram_socket.reuseaddr";
/// `#define NET_PROP_DATAGRAM_SOCKET_ALLOW_BROADCAST_BOOLEAN "NET.datagram_socket.allow_broadcast"`
pub const PROP_DATAGRAM_SOCKET_ALLOW_BROADCAST_BOOLEAN: &str =
    "NET.datagram_socket.allow_broadcast";

#[cfg(windows)]
fn write(s: Socket, buf: &[u8]) -> isize {
    // SAFETY: `buf` is valid for its length (clamped to WinSock's `int`).
    unsafe { sys::send(s, buf.as_ptr(), buf.len().min(i32::MAX as usize) as i32, 0) as isize }
}

#[cfg(windows)]
fn read(s: Socket, buf: &mut [u8]) -> isize {
    let wsabuf = sys::WSABUF {
        buf: buf.as_mut_ptr(),
        len: buf.len().min(u32::MAX as usize) as u32,
    };
    let mut count_received: u32 = 0;
    let mut flags: u32 = 0;
    // SAFETY: one valid WSABUF, no overlapped I/O.
    let res = unsafe {
        sys::WSARecv(
            s,
            &wsabuf,
            1,
            &mut count_received,
            &mut flags,
            std::ptr::null_mut(),
            None,
        )
    };
    if res != 0 {
        return -1;
    }
    count_received as isize
}

// WSAPoll doesn't exist on Windows before Vista, and isn't reliable before some version of Windows 10,
//  so for now we just fake it with select().
#[cfg(windows)]
fn windows_poll(fds: &mut [pollfd], timeout: c_int) -> c_int {
    debug_assert!(!fds.is_empty());

    let nfds = fds.len();
    let (mut nreadfds, mut nwritefds, nexceptfds) = (0usize, 0usize, nfds);
    for fd in fds.iter_mut() {
        fd.revents = 0;
        if (fd.events & POLLIN) != 0 {
            nreadfds += 1;
        }
        if (fd.events & POLLOUT) != 0 {
            nwritefds += 1;
        }
    }

    // FD_SETSIZE is 64 on Windows, but they don't use bitsets, so you can
    //  just supply your own struct that uses any length.
    //
    //   https://devblogs.microsoft.com/oldnewthing/20221102-00/?p=107343
    //
    // (Here: element 0 is the count, padded to a socket's size, then the
    // sockets; sys.rs checks that this is WinSock's `fd_set` layout.)
    fn alloc_fdset(n: usize) -> Option<Vec<Socket>> {
        (n > 0).then(|| vec![0 as Socket; 1 + n])
    }
    let mut readfds = alloc_fdset(nreadfds);
    let mut writefds = alloc_fdset(nwritefds);
    let mut exceptfds = alloc_fdset(nexceptfds);

    fn push(set: &mut Option<Vec<Socket>>, sock: Socket) {
        if let Some(set) = set {
            let count = set[0];
            set[1 + count] = sock;
            set[0] = count + 1;
        }
    }
    for fd in fds.iter() {
        push(&mut exceptfds, fd.fd);
        if (fd.events & POLLIN) != 0 {
            push(&mut readfds, fd.fd);
        }
        if (fd.events & POLLOUT) != 0 {
            push(&mut writefds, fd.fd);
        }
    }

    let tvtimeout;
    let mut ptvtimeout: *const sys::TIMEVAL = std::ptr::null();

    if timeout >= 0 {
        tvtimeout = sys::TIMEVAL {
            tv_sec: timeout / 1000,
            tv_usec: (timeout % 1000) * 1000,
        };
        ptvtimeout = &tvtimeout;
    }

    fn as_fdset(set: &mut Option<Vec<Socket>>) -> *mut sys::FD_SET {
        match set {
            Some(set) => set.as_mut_ptr().cast(),
            None => std::ptr::null_mut(),
        }
    }

    // WinSock's select() ignores the first parameter, since it doesn't use bitsets, and SOCKETs aren't small integers. Just specify zero here.
    // SAFETY: each set is a count followed by that many sockets (see above).
    let retval = unsafe {
        sys::select(
            0,
            as_fdset(&mut readfds),
            as_fdset(&mut writefds),
            as_fdset(&mut exceptfds),
            ptvtimeout,
        )
    };
    if retval > 0 {
        fn checkset(set: &Option<Vec<Socket>>, fds: &mut [pollfd], flag: sys::PollEvents) {
            if let Some(set) = set {
                // select() leaves just the ready sockets in the set, and the count of them.
                let count = (set[0] as u32) as usize;
                for &sock in &set[1..1 + count] {
                    for fd in fds.iter_mut() {
                        if fd.fd == sock {
                            fd.revents |= flag;
                        }
                    }
                }
            }
        }
        checkset(&readfds, fds, POLLIN);
        checkset(&writefds, fds, POLLOUT);
        checkset(&exceptfds, fds, POLLERR);
    }

    retval
}

#[cfg(unix)]
fn write(s: Socket, buf: &[u8]) -> isize {
    // SAFETY: `buf` is valid for its length.
    unsafe { libc::write(s, buf.as_ptr().cast(), buf.len()) }
}

#[cfg(unix)]
fn read(s: Socket, buf: &mut [u8]) -> isize {
    // SAFETY: `buf` is valid for its length.
    unsafe { libc::read(s, buf.as_mut_ptr().cast(), buf.len()) }
}

/// `poll()` (`#define poll WindowsPoll` on Windows).
fn poll(fds: &mut [pollfd], timeout: c_int) -> c_int {
    #[cfg(windows)]
    {
        windows_poll(fds, timeout)
    }
    #[cfg(unix)]
    {
        // SAFETY: `fds` is valid for its length.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) }
    }
}

/// `NET_SocketType`: what is behind a [`GenericSocket`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SocketType {
    Stream,
    Datagram,
    Server,
}

/// The version of SDL_net this crate translates. Translation of
/// `NET_Version()` (which reports the linked library's version; this is
/// [`VERSION`](crate::VERSION)).
pub fn version() -> sdl3::Version {
    crate::VERSION
}

/// A tri-state for asynchronous operations, less its failure state.
/// Translation of `NET_Status`.
///
/// Resolving a hostname and connecting a client take time, so the library
/// reports their progress: still in progress ([`Status::Waiting`],
/// `NET_WAITING`, 0) or complete and successful ([`Status::Success`],
/// `NET_SUCCESS`, 1). Complete and failed (`NET_FAILURE`, -1) is the `Err`
/// of the [`Result`] these come in, carrying the message upstream would put
/// in `SDL_GetError()`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Status {
    /// Async operation is still in progress, check again later. (`NET_WAITING`)
    Waiting = 0,
    /// Async operation complete, result was success. (`NET_SUCCESS`)
    Success = 1,
}

// `NET_Status` as upstream stores it (in `NET_Address::status` and
// `NET_StreamSocket::status`).
const NET_FAILURE: i32 = -1;
const NET_WAITING: i32 = 0;
const NET_SUCCESS: i32 = 1;

/// The parts of the first `struct addrinfo` of a resolution that the
/// library uses (`ai_family`, `ai_addrlen` and the `ai_addr` bytes).
///
/// Upstream keeps the `struct addrinfo` list `getaddrinfo()` returned until
/// the address is destroyed; this keeps a copy and frees the list at once,
/// so an [`Address`] holds no system resources (and can outlive
/// [`quit`]).
#[derive(Debug)]
struct AddrInfoData {
    family: c_int,
    addr: Vec<u8>,
}

impl AddrInfoData {
    /// SAFETY: `ainfo` must point to a valid `struct addrinfo`.
    unsafe fn from_addrinfo(ainfo: *const AddrInfo) -> AddrInfoData {
        // SAFETY: as the caller promises; `ai_addr` holds `ai_addrlen` bytes.
        unsafe {
            let ainfo = &*ainfo;
            let len = ainfo.ai_addrlen as usize;
            let addr = if ainfo.ai_addr.is_null() || len == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(ainfo.ai_addr as *const u8, len).to_vec()
            };
            AddrInfoData {
                family: ainfo.ai_family,
                addr,
            }
        }
    }

    fn sockaddr(&self) -> *const SockAddr {
        self.addr.as_ptr().cast()
    }
}

/// A `struct addrinfo` list from `getaddrinfo()`, freed on drop.
struct AddrInfoList(*mut AddrInfo);

impl AddrInfoList {
    fn iter(&self) -> impl Iterator<Item = &AddrInfo> {
        let mut next = self.0 as *const AddrInfo;
        std::iter::from_fn(move || {
            // SAFETY: the list stays alive (and unmodified) as long as `self`.
            let ai = unsafe { next.as_ref()? };
            next = ai.ai_next;
            Some(ai)
        })
    }

    fn first(&self) -> &AddrInfo {
        self.iter()
            .next()
            .expect("getaddrinfo() succeeded with no result")
    }
}

impl Drop for AddrInfoList {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the list came from getaddrinfo() and is freed once.
            unsafe { sys::freeaddrinfo(self.0) };
        }
    }
}

/// `struct NET_Address`.
struct AddressData {
    hostname: Option<CString>,
    human_readable: OnceLock<String>,
    errstr: OnceLock<String>,
    status: AtomicI32, // This is actually a NET_Status.
    ainfo: OnceLock<AddrInfoData>,
    // (`refcount` is the `Arc`'s, and `resolver_next` is the resolver's queue.)
}

/// A computer-readable network address. Translation of `NET_Address`.
///
/// SDL_net uses these to identify other servers; you use them to connect to
/// a remote machine, and you use them to find out who connected to you.
/// They are also used to decide what network interface to use when creating
/// a server. They are intended to be protocol-independent: a given address
/// might be for IPv4, IPv6, or something more esoteric.
///
/// Addresses are reference counted: `clone()` is `NET_RefAddress()` and
/// dropping one is `NET_UnrefAddress()`. They compare (and sort) as
/// [`compare_addresses`] does.
#[derive(Clone)]
pub struct Address(Arc<AddressData>);

impl std::fmt::Debug for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Address")
            .field("hostname", &self.0.hostname)
            .field("human_readable", &self.0.human_readable.get())
            .field("status", &self.0.status.load(Ordering::SeqCst))
            .finish()
    }
}

/// `struct NetworkInterface`.
struct NetworkInterface {
    #[allow(dead_code)] // (only the `#if 0` interface log reads it.)
    name: String,
    index: u32,
    address: Address,
    broadcast: Option<Address>,
}

const MIN_RESOLVER_THREADS: usize = 2;
const MAX_RESOLVER_THREADS: usize = 10;

// Address resolver state...
/// `resolver_queue` (the `NET_Address::resolver_next` list) and
/// `resolver_threads`, behind `resolver_lock`.
struct ResolverState {
    queue: Vec<Address>,
    threads: [Option<Thread>; MAX_RESOLVER_THREADS],
}

// (Upstream creates `resolver_lock` and `resolver_condition` in NET_Init()
// and destroys them in NET_Quit(); these can't fail to be created, so they
// are statics.)
static RESOLVER_LOCK: ReentrantMutex<RefCell<ResolverState>> =
    ReentrantMutex::new(RefCell::new(ResolverState {
        queue: Vec::new(),
        threads: [const { None }; MAX_RESOLVER_THREADS],
    }));
static RESOLVER_CONDITION: Condition = Condition::new();
static RESOLVER_SHUTDOWN: AtomicI32 = AtomicI32::new(0);
static RESOLVER_NUM_THREADS: AtomicI32 = AtomicI32::new(0);
static RESOLVER_NUM_REQUESTS: AtomicI32 = AtomicI32::new(0);
static RESOLVER_PERCENT_LOSS: AtomicI32 = AtomicI32::new(0);

// Network interface state...
static INTERFACE_INIT: InitState = InitState::new();
/// `interfaces` and `num_interfaces`, behind `interface_rwlock`.
static INTERFACES: RwLock<Vec<NetworkInterface>> = RwLock::new(Vec::new());
static INTERFACES_HAVE_CHANGED: AtomicI32 = AtomicI32::new(0);

// Other stuff...
static IPV6_BROADCAST_ADDR: Mutex<Option<Address>> = Mutex::new(None);

fn ipv6_broadcast_addr() -> Option<Address> {
    IPV6_BROADCAST_ADDR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

fn read_interfaces() -> std::sync::RwLockReadGuard<'static, Vec<NetworkInterface>> {
    INTERFACES.read().unwrap_or_else(|e| e.into_inner())
}

fn write_interfaces() -> std::sync::RwLockWriteGuard<'static, Vec<NetworkInterface>> {
    INTERFACES.write().unwrap_or_else(|e| e.into_inner())
}

/// WinSock's initialization (`WSAStartup()`), undone (`WSACleanup()`)
/// when the last holder lets go.
///
/// Upstream calls `WSAStartup()` in NET_Init() and `WSACleanup()` in
/// NET_Quit(), and the app must destroy its sockets before that. Here each
/// socket holds a reference, so a socket dropped after [`quit`] is still
/// closed while WinSock is up.
#[cfg(windows)]
#[derive(Debug)]
struct Winsock;

#[cfg(windows)]
impl Drop for Winsock {
    fn drop(&mut self) {
        // SAFETY: pairs the WSAStartup() that created this.
        unsafe { sys::WSACleanup() };
    }
}

#[cfg(windows)]
static WINSOCK: Mutex<Option<Arc<Winsock>>> = Mutex::new(None);

#[cfg(windows)]
type WinsockRef = Option<Arc<Winsock>>;
#[cfg(not(windows))]
type WinsockRef = ();

fn winsock_ref() -> WinsockRef {
    #[cfg(windows)]
    {
        WINSOCK.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

// between lo and hi (inclusive; it can return lo or hi itself, too!).
fn random_number_between(lo: i32, hi: i32) -> i32 {
    sdl3::stdlib::random::below((hi + 1) - lo) + lo
}

fn should_simulate_loss(percent_likely_to_lose: i32) -> bool {
    // these should be clamped when assigning them.
    debug_assert!(percent_likely_to_lose >= 0);
    debug_assert!(percent_likely_to_lose <= 100);
    if percent_likely_to_lose > 0 {
        // FIXME (upstream): this draws from 0 to 100, 101 values, so 100
        // percent loses only 100 of 101 times; the documentation promises
        // that 100 "means _everything_ fails unconditionally". Drawn from
        // 0 to 99 here.
        random_number_between(0, 99) < percent_likely_to_lose
    } else {
        false
    }
}

fn delay_ms(ms: i32) {
    sdl3::timer::delay(Duration::from_millis(ms.max(0) as u64));
}

fn get_ticks() -> u64 {
    sdl3::timer::ticks_ms()
}

fn close_socket_handle(handle: Socket) -> c_int {
    // SAFETY: the handle is ours and is closed once.
    #[cfg(windows)]
    unsafe {
        sys::closesocket(handle)
    }
    // SAFETY: the handle is ours and is closed once.
    #[cfg(unix)]
    unsafe {
        libc::close(handle)
    }
}

fn last_socket_error() -> c_int {
    #[cfg(windows)]
    {
        // SAFETY: no preconditions.
        unsafe { sys::WSAGetLastError() }
    }
    #[cfg(unix)]
    {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }
}

/// The text of a NUL-terminated C string in a buffer.
fn buf_to_string(buf: &[c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// SAFETY: `s` must be NULL or a NUL-terminated C string.
#[cfg(unix)]
unsafe fn c_str_to_string(s: *const c_char) -> String {
    if s.is_null() {
        return String::new();
    }
    // SAFETY: as the caller promises.
    unsafe { std::ffi::CStr::from_ptr(s) }
        .to_string_lossy()
        .into_owned()
}

fn create_socket_error_string(rc: c_int) -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            FormatMessageW, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
        };
        let mut msgbuf = [0u16; 256];
        // MAKELANGID(LANG_NEUTRAL, SUBLANG_DEFAULT): Default language
        const LANGID: u32 = (0x01 << 10) | 0x00;
        // SAFETY: the buffer is valid for its length.
        let bw = unsafe {
            FormatMessageW(
                FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
                std::ptr::null(),
                rc as u32,
                LANGID,
                msgbuf.as_mut_ptr(),
                msgbuf.len() as u32,
                std::ptr::null(),
            )
        };
        if bw == 0 {
            return "Unknown error".to_string();
        }
        let bytes: Vec<u8> = msgbuf[..(bw as usize + 1).min(msgbuf.len())]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        match sdl3::stdlib::iconv::iconv_string("UTF-8", "UTF-16LE", &bytes) {
            Ok(utf8) => {
                let end = utf8.iter().position(|&b| b == 0).unwrap_or(utf8.len());
                String::from_utf8_lossy(&utf8[..end]).into_owned()
            }
            Err(_) => String::new(),
        }
    }
    #[cfg(unix)]
    {
        // SAFETY: strerror() returns a valid C string.
        unsafe { c_str_to_string(libc::strerror(rc)) }
    }
}

fn create_get_addr_info_error_string(rc: c_int) -> String {
    #[cfg(windows)]
    {
        create_socket_error_string(rc) // same error codes.
    }
    #[cfg(unix)]
    {
        if rc == libc::EAI_SYSTEM {
            create_socket_error_string(last_socket_error())
        } else {
            // SAFETY: gai_strerror() returns a valid C string.
            unsafe { c_str_to_string(libc::gai_strerror(rc)) }
        }
    }
}

/// `SetSocketError()` (and `SetSocketErrorBool()`): the error to return.
fn set_socket_error(msg: &str, err: c_int) -> Error {
    let errmsg = create_socket_error_string(err);
    Error::new(format!("{msg}: {errmsg}"))
}

fn set_last_socket_error(msg: &str) -> Error {
    set_socket_error(msg, last_socket_error())
}

/// `SetGetAddrInfoError()` (and `SetGetAddrInfoErrorBool()`).
fn set_get_addr_info_error(msg: &str, err: c_int) -> Error {
    let errmsg = create_get_addr_info_error_string(err);
    Error::new(format!("{msg}: {errmsg}"))
}

fn make_socket_nonblocking(handle: Socket) -> c_int {
    #[cfg(windows)]
    {
        let mut one: u32 = 1;
        // SAFETY: FIONBIO takes a u_long.
        unsafe { sys::ioctlsocket(handle, sys::FIONBIO, &mut one) }
    }
    #[cfg(unix)]
    {
        // SAFETY: plain fcntl() calls on our descriptor.
        unsafe {
            libc::fcntl(
                handle,
                libc::F_SETFL,
                libc::fcntl(handle, libc::F_GETFL, 0) | libc::O_NONBLOCK,
            )
        }
    }
}

fn would_block(err: c_int) -> bool {
    #[cfg(windows)]
    {
        err == sys::WSAEWOULDBLOCK
    }
    #[cfg(unix)]
    {
        (err == libc::EWOULDBLOCK) || (err == libc::EAGAIN) || (err == libc::EINPROGRESS)
    }
}

/* Network interface enumeration... */
// (FreeNetworkInterfaces() is dropping the `Vec<NetworkInterface>`.)

/// Install a new interface list (the `succeeded:` part of every
/// RefreshInterfaces()).
fn replace_interfaces(new_interfaces: Vec<NetworkInterface>) {
    let old_interfaces = std::mem::replace(&mut *write_interfaces(), new_interfaces);
    drop(old_interfaces); // FreeNetworkInterfaces(), outside the lock.
}

#[cfg(windows)]
mod ifaces {
    //! The WINDOWS VERSION of the interface functions.

    use super::*;
    use core::ffi::c_void;
    use std::sync::atomic::AtomicUsize;
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, HANDLE, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        CancelMibChangeNotify2, GetAdaptersAddresses, NotifyIpInterfaceChange,
        GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_FRIENDLY_NAME, IP_ADAPTER_ADDRESSES_LH,
        IP_ADAPTER_UNICAST_ADDRESS_LH, MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE,
    };
    use windows_sys::Win32::Networking::WinSock::SOCKADDR_IN;

    // (A HANDLE, kept as an integer so the static is Sync.)
    static INTERFACE_CHANGE_NOTIFICATIONS_HANDLE: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "system" fn network_interface_changed_callback(
        _ctx: *const c_void,
        _row: *const MIB_IPINTERFACE_ROW,
        _type: MIB_NOTIFICATION_TYPE,
    ) {
        INTERFACES_HAVE_CHANGED.store(1, Ordering::SeqCst);
    }

    pub(super) fn init_interface_change_notifications() -> bool {
        // this would need to use NotifyAddrChange for Windows XP, but it doesn't work with IPv6 before Vista.
        //return (NotifyAddrChange(&addr_change_notification_handle, &overlapped) == ERROR_IO_PENDING);

        // !!! FIXME: this function is available on Vista and later.
        let mut handle: HANDLE = std::ptr::null_mut();
        // SAFETY: the callback has the right signature and lives forever.
        let rc = unsafe {
            NotifyIpInterfaceChange(
                AF_UNSPEC as u16,
                Some(network_interface_changed_callback),
                std::ptr::null(),
                false,
                &mut handle,
            )
        };
        INTERFACE_CHANGE_NOTIFICATIONS_HANDLE.store(handle as usize, Ordering::SeqCst);
        rc == NO_ERROR
    }

    pub(super) fn quit_interface_change_notifications() {
        let handle = INTERFACE_CHANGE_NOTIFICATIONS_HANDLE.swap(0, Ordering::SeqCst);
        // SAFETY: the handle came from NotifyIpInterfaceChange().
        unsafe { CancelMibChangeNotify2(handle as HANDLE) };
    }

    pub(super) fn refresh_interfaces() {
        // MSDN docs say start with a 15K buffer, which usually works on the first
        //  try, instead of trying to query for size, allocate, and then retry,
        //  since this tends to be more expensive.
        let mut buflen: u32 = 15 * 1024;
        let mut addrs: Vec<u64>; // (u64s, for IP_ADAPTER_ADDRESSES' alignment.)
        let mut rc;

        loop {
            addrs = vec![0u64; (buflen as usize).div_ceil(8)];
            let flags = GAA_FLAG_SKIP_DNS_SERVER | GAA_FLAG_SKIP_FRIENDLY_NAME;
            // SAFETY: the buffer holds `buflen` bytes.
            rc = unsafe {
                GetAdaptersAddresses(
                    AF_UNSPEC as u32,
                    flags,
                    std::ptr::null(),
                    addrs.as_mut_ptr().cast(),
                    &mut buflen,
                )
            };
            if rc != ERROR_BUFFER_OVERFLOW {
                break;
            }
        }

        if rc != NO_ERROR {
            return;
        }

        let mut new_interfaces = Vec::new();
        // SAFETY: GetAdaptersAddresses() filled the buffer with a linked
        // list of adapters, each with a list of unicast addresses.
        unsafe {
            let mut i = addrs.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
            while !i.is_null() {
                let adapter = &*i;
                let friendlyname = if adapter.FriendlyName.is_null() {
                    None
                } else {
                    let mut len = 0;
                    while *adapter.FriendlyName.add(len) != 0 {
                        len += 1;
                    }
                    let bytes: Vec<u8> = std::slice::from_raw_parts(adapter.FriendlyName, len + 1)
                        .iter()
                        .flat_map(|w| w.to_le_bytes())
                        .collect();
                    sdl3::stdlib::iconv::iconv_string("UTF-8", "UTF-16LE", &bytes)
                        .ok()
                        .map(|utf8| {
                            let end = utf8.iter().position(|&b| b == 0).unwrap_or(utf8.len());
                            String::from_utf8_lossy(&utf8[..end]).into_owned()
                        })
                };
                let mut j = adapter.FirstUnicastAddress as *const IP_ADAPTER_UNICAST_ADDRESS_LH;
                while !j.is_null() {
                    let unicast = &*j;
                    let saddr = unicast.Address.lpSockaddr;
                    if let Ok(addr) = create_sdl_net_addr_from_sock_addr(
                        saddr,
                        unicast.Address.iSockaddrLength as SockLen,
                    ) {
                        let mut iface = NetworkInterface {
                            name: friendlyname.clone().unwrap_or_default(),
                            index: 0,
                            address: addr,
                            broadcast: None,
                        };
                        let family = (*saddr).sa_family as c_int;
                        if family == AF_INET6 {
                            iface.index = adapter.Ipv6IfIndex;
                        } else if family == AF_INET {
                            // if this is IPv4, calculate the broadcast address.
                            debug_assert_eq!(
                                unicast.Address.iSockaddrLength as usize,
                                std::mem::size_of::<SOCKADDR_IN>()
                            );
                            let mut bcast: SOCKADDR_IN = std::ptr::read_unaligned(saddr.cast());
                            // FIXME (upstream): `1 << 32` for a prefix length
                            // of zero is undefined behavior in C; here it is
                            // all host bits, as the formula means.
                            let prefix = unicast.OnLinkPrefixLength as u32;
                            let host_bits = 1u32
                                .checked_shl(32u32.saturating_sub(prefix))
                                .unwrap_or(0)
                                .wrapping_sub(1);
                            bcast.sin_addr.S_un.S_addr |= host_bits.to_be();
                            iface.broadcast = create_sdl_net_addr_from_sock_addr(
                                (&bcast as *const SOCKADDR_IN).cast(),
                                std::mem::size_of::<SOCKADDR_IN>() as SockLen,
                            )
                            .ok();
                            iface.index = adapter.Anonymous1.Anonymous.IfIndex;
                        }
                        new_interfaces.push(iface);
                    }
                    j = unicast.Next;
                }
                i = adapter.Next;
            }
        }

        // (`new_num_interfaces = count;  // in case we dropped one somewhere.`)
        replace_interfaces(new_interfaces);
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod ifaces {
    //! The LINUX/BSD VERSION of the interface functions, with
    //! `USE_NETLINK`.

    // AF_NETLINK covers Linux (and by extension Android). PF_ROUTE covers the BSDs (and by extension Apple platforms).
    // This doesn't cover all Unix platforms that ever existed, but this hits just about everything that is still being maintained seriously.

    use super::*;

    static INTERFACE_CHANGE_NOTIFICATIONS_THREAD: Mutex<Option<Thread>> = Mutex::new(None);
    static INTERFACE_CHANGE_NOTIFICATIONS_FLAG: AtomicI32 = AtomicI32::new(0); // !!! FIXME

    // <linux/netlink.h> and <linux/rtnetlink.h>.
    const NLMSG_HDRLEN: usize = 16; // NLMSG_ALIGN(sizeof (struct nlmsghdr))
    const IFADDRMSG_LEN: usize = 8; // sizeof (struct ifaddrmsg)
    const RTATTR_LEN: usize = 4; // sizeof (struct rtattr)
    const NLMSG_ERROR: u16 = libc::NLMSG_ERROR as u16;
    const NLMSG_DONE: u16 = libc::NLMSG_DONE as u16;
    const RTM_NEWADDR: u16 = libc::RTM_NEWADDR;
    const RTM_GETADDR: u16 = libc::RTM_GETADDR;
    const IFA_ADDRESS: u16 = 1;
    const IFA_BROADCAST: u16 = 4;

    const fn nlmsg_align(len: usize) -> usize {
        (len + 3) & !3
    }
    const fn rta_align(len: usize) -> usize {
        (len + 3) & !3
    }

    fn linux_interface_change_notification_thread() -> i32 {
        // SAFETY: plain socket calls on a descriptor we own.
        unsafe {
            let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE);
            if fd == -1 {
                return 0; //  oh well.
            }

            // !!! FIXME: don't make this non-blocking, find a more efficient way to terminate the thread.
            libc::fcntl(
                fd,
                libc::F_SETFL,
                libc::fcntl(fd, libc::F_GETFL, 0) | libc::O_NONBLOCK,
            );

            let mut addr: libc::sockaddr_nl = std::mem::zeroed();
            addr.nl_family = libc::AF_NETLINK as libc::sa_family_t;
            addr.nl_pid = 0; // zero==let the kernel choose a unique number. THIS IS THE SOCKET PORT ID, NOT THE PROCESS ID!
            addr.nl_groups =
                (libc::RTMGRP_LINK | libc::RTMGRP_IPV4_IFADDR | libc::RTMGRP_IPV6_IFADDR) as u32;
            if libc::bind(
                fd,
                (&addr as *const libc::sockaddr_nl).cast(),
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            ) == -1
            {
                libc::close(fd);
                return 0; // oh well.
            }

            let mut buf = vec![0u8; 8192];
            while INTERFACE_CHANGE_NOTIFICATIONS_FLAG.load(Ordering::SeqCst) == 0 {
                let mut iov = libc::iovec {
                    iov_base: buf.as_mut_ptr().cast(),
                    iov_len: buf.len(),
                };
                let mut msg: libc::msghdr = std::mem::zeroed();
                msg.msg_iov = &mut iov;
                msg.msg_iovlen = 1;

                let mut sa: libc::sockaddr_nl = std::mem::zeroed();
                msg.msg_name = (&mut sa as *mut libc::sockaddr_nl).cast();
                msg.msg_namelen = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;

                let br = libc::recvmsg(fd, &mut msg, 0);
                if br < 0 {
                    if would_block(last_socket_error()) {
                        delay_ms(300);
                        continue;
                    }
                    // quit if there's a failure. Presumably this is because another thread close()'d the socket, to unblock the thread and request termination.
                    break;
                }

                // we don't currently bother parsing for specifics, we just use this as a signal to reenumerate interfaces.
                //for (struct nlmsghdr *i = buf; NLMSG_OK(i, br); i = NLMSG_NEXT(i, br)) {}
                //SDL_Log("Network interfaces changed!");
                INTERFACES_HAVE_CHANGED.store(1, Ordering::SeqCst);
            }

            libc::close(fd);
        }

        0
    }

    pub(super) fn init_interface_change_notifications() -> bool {
        // LINUX/BSD VERSION
        match Thread::spawn(
            "SDLNetIfaceEnum",
            linux_interface_change_notification_thread,
        ) {
            Ok(thread) => {
                *INTERFACE_CHANGE_NOTIFICATIONS_THREAD
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(thread);
                true
            }
            Err(_) => false,
        }
    }

    pub(super) fn quit_interface_change_notifications() {
        // LINUX/BSD VERSION
        let thread = INTERFACE_CHANGE_NOTIFICATIONS_THREAD
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(thread) = thread {
            INTERFACE_CHANGE_NOTIFICATIONS_FLAG.store(1, Ordering::SeqCst);
            thread.wait();
            INTERFACE_CHANGE_NOTIFICATIONS_FLAG.store(0, Ordering::SeqCst);
        }
    }

    fn u16_at(buf: &[u8], off: usize) -> u16 {
        u16::from_ne_bytes([buf[off], buf[off + 1]])
    }
    fn u32_at(buf: &[u8], off: usize) -> u32 {
        u32::from_ne_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
    }

    pub(super) fn refresh_interfaces() {
        // LINUX/BSD VERSION
        // Android has getifaddrs() in Android 24, but we still target 21, so do it the hard way. Since this works on normal Linux, we'll just use it there, too.
        // SAFETY: plain socket calls on a descriptor we own.
        let sock = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM, libc::NETLINK_ROUTE) };
        if sock < 0 {
            return; // oh well.
        }

        make_socket_nonblocking(sock);

        // Ask for the address information.
        // (struct reqstruct { struct nlmsghdr header; struct ifaddrmsg msg; }, laid out by hand.)
        let mut req = [0u8; NLMSG_HDRLEN + IFADDRMSG_LEN];
        let nlmsg_len = (NLMSG_HDRLEN + IFADDRMSG_LEN) as u32; // NLMSG_LENGTH(sizeof (req.msg))
        req[0..4].copy_from_slice(&nlmsg_len.to_ne_bytes());
        req[4..6].copy_from_slice(&RTM_GETADDR.to_ne_bytes());
        let nlmsg_flags = (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16;
        req[6..8].copy_from_slice(&nlmsg_flags.to_ne_bytes());
        req[NLMSG_HDRLEN] = AF_UNSPEC as u8; // req.msg.ifa_family

        // SAFETY: `req` is valid for its length.
        let sent = unsafe { libc::send(sock, req.as_ptr().cast(), req.len(), 0) };
        if sent != nlmsg_len as isize {
            close_socket_handle(sock);
            return; // oh well.
        }

        let mut new_interfaces: Vec<NetworkInterface> = Vec::new();

        let mut buffer = vec![0u8; 64 * 1024];
        let succeeded = 'recv: loop {
            // SAFETY: `buffer` is valid for its length.
            let br = unsafe { libc::recv(sock, buffer.as_mut_ptr().cast(), buffer.len(), 0) };
            if br <= 0 {
                break 'recv true;
            }
            let mut len = br; // (NLMSG_NEXT() counts this down.)
            let mut header = 0usize;
            // NLMSG_OK(header, br)
            while len >= NLMSG_HDRLEN as isize
                && u32_at(&buffer, header) as usize >= NLMSG_HDRLEN
                && u32_at(&buffer, header) as isize <= len
            {
                let header_len = u32_at(&buffer, header) as usize;
                let header_type = u16_at(&buffer, header + 4);
                if header_type == NLMSG_DONE {
                    break 'recv true; // we got it all.
                } else if header_type == NLMSG_ERROR {
                    break 'recv false; // uhoh.
                } else if header_type == RTM_NEWADDR {
                    // strictly speaking, one interface can have multiple IP addresses, so you might have multiple addresses with the same interface index.
                    let mut address: Option<Address> = None;
                    let mut broadcast: Option<Address> = None;

                    let msg = header + NLMSG_HDRLEN; // NLMSG_DATA(header)
                    let ifa_family = buffer[msg] as c_int;
                    let ifa_index = u32_at(&buffer, msg + 4);
                    // IFA_PAYLOAD(header)
                    let mut payload_len =
                        header_len.saturating_sub(nlmsg_align(NLMSG_HDRLEN + IFADDRMSG_LEN));
                    // IFA_RTA(msg)
                    let mut attr = msg + nlmsg_align(IFADDRMSG_LEN);
                    // RTA_OK(attr, payload_len)
                    while payload_len >= RTATTR_LEN
                        && u16_at(&buffer, attr) as usize >= RTATTR_LEN
                        && u16_at(&buffer, attr) as usize <= payload_len
                    {
                        let rta_len = u16_at(&buffer, attr) as usize;
                        let rta_type = u16_at(&buffer, attr + 2);
                        let isbroadcast = rta_type == IFA_BROADCAST;
                        if isbroadcast || (rta_type == IFA_ADDRESS) {
                            // this gives us the raw bytes of an address, but not the actual sockaddr_* layout, so we have to go with known protocols.  :/
                            let data = &buffer[attr + RTATTR_LEN..attr + rta_len]; // RTA_DATA(attr), RTA_PAYLOAD(attr)
                                                                                   // SAFETY: an all-zero sockaddr_storage is valid.
                            let mut addrstorage: AddressStorage = unsafe { std::mem::zeroed() };
                            let addrlen;
                            addrstorage.ss_family = ifa_family as libc::sa_family_t;
                            if ifa_family == AF_INET {
                                let sa = (&mut addrstorage as *mut AddressStorage)
                                    .cast::<libc::sockaddr_in>();
                                // (Upstream copies RTA_PAYLOAD() bytes; this
                                // copies no more than the address holds.)
                                // SAFETY: sockaddr_storage is big enough for a sockaddr_in.
                                unsafe {
                                    let dst =
                                        (&mut (*sa).sin_addr as *mut libc::in_addr).cast::<u8>();
                                    std::ptr::copy_nonoverlapping(
                                        data.as_ptr(),
                                        dst,
                                        data.len().min(4),
                                    );
                                }
                                addrlen = std::mem::size_of::<libc::sockaddr_in>();
                            } else if ifa_family == AF_INET6 {
                                let sa6 = (&mut addrstorage as *mut AddressStorage)
                                    .cast::<libc::sockaddr_in6>();
                                // SAFETY: sockaddr_storage is big enough for a sockaddr_in6.
                                unsafe {
                                    let dst =
                                        (&mut (*sa6).sin6_addr as *mut libc::in6_addr).cast::<u8>();
                                    std::ptr::copy_nonoverlapping(
                                        data.as_ptr(),
                                        dst,
                                        data.len().min(16),
                                    );
                                }
                                addrlen = std::mem::size_of::<libc::sockaddr_in6>();
                            } else {
                                // unknown protocol family.
                                // RTA_NEXT(attr, payload_len)
                                payload_len -= rta_align(rta_len).min(payload_len);
                                attr += rta_align(rta_len);
                                continue;
                            }

                            let paddr = if isbroadcast {
                                &mut broadcast
                            } else {
                                &mut address
                            };
                            debug_assert!(paddr.is_none()); // shouldn't be two of these attributes on a single RTM_NEWADDR.
                                                            // SAFETY: the storage holds `addrlen` bytes of a socket address.
                            *paddr = unsafe {
                                create_sdl_net_addr_from_sock_addr(
                                    (&addrstorage as *const AddressStorage).cast(),
                                    addrlen as SockLen,
                                )
                            }
                            .ok();
                        }
                        // RTA_NEXT(attr, payload_len)
                        payload_len -= rta_align(rta_len).min(payload_len);
                        attr += rta_align(rta_len);
                    }

                    if let Some(address) = address {
                        let index = ifa_index;

                        let mut interface_name = [0 as c_char; libc::IF_NAMESIZE];
                        // SAFETY: the buffer is IF_NAMESIZE bytes, as if_indextoname() wants.
                        if unsafe { libc::if_indextoname(index, interface_name.as_mut_ptr()) }
                            .is_null()
                        {
                            // miraculously, this function is in Android 21; nothing else is, though! We could have done this with netlink too, but this is the easy button.
                            interface_name[0] = 0;
                        }

                        new_interfaces.push(NetworkInterface {
                            name: buf_to_string(&interface_name),
                            index,
                            address,
                            broadcast,
                        });
                    } else {
                        drop(broadcast); // just in case.
                    }
                }
                // NLMSG_NEXT(header, br)
                len -= nlmsg_align(header_len) as isize;
                header += nlmsg_align(header_len);
            }
        };

        close_socket_handle(sock);
        if succeeded {
            replace_interfaces(new_interfaces);
        } else {
            drop(new_interfaces);
            // if this was a failure case, oh well, old (maybe incorrect) ones will have to stand.
        }
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
mod ifaces {
    //! The LINUX/BSD VERSION of the interface functions, with
    //! `HAVE_GETIFADDRS` (and the `PF_ROUTE` network monitor on the BSDs
    //! and Apple platforms).

    use super::*;

    #[cfg(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    mod monitor {
        use super::*;

        pub(super) static INTERFACE_CHANGE_NOTIFICATIONS_THREAD: Mutex<Option<Thread>> =
            Mutex::new(None);
        pub(super) static INTERFACE_CHANGE_NOTIFICATIONS_FLAG: AtomicI32 = AtomicI32::new(0); // !!! FIXME

        pub(super) fn interface_change_notification_thread() -> i32 {
            // SAFETY: plain socket calls on a descriptor we own.
            unsafe {
                let fd = libc::socket(libc::PF_ROUTE, libc::SOCK_RAW, 0);
                if fd == -1 {
                    return 0; //  oh well.
                }

                // !!! FIXME: don't make this non-blocking, find a more efficient way to terminate the thread.
                libc::fcntl(
                    fd,
                    libc::F_SETFL,
                    libc::fcntl(fd, libc::F_GETFL, 0) | libc::O_NONBLOCK,
                );

                // (Upstream sets an RO_MSGFILTER where the system has one,
                // NetBSD; without it, every routing message is a change.)

                let mut buf = vec![0u8; 8192];
                while INTERFACE_CHANGE_NOTIFICATIONS_FLAG.load(Ordering::SeqCst) == 0 {
                    let mut iov = libc::iovec {
                        iov_base: buf.as_mut_ptr().cast(),
                        iov_len: buf.len(),
                    };
                    let mut msg: libc::msghdr = std::mem::zeroed();
                    msg.msg_iov = &mut iov;
                    msg.msg_iovlen = 1;

                    let br = libc::recvmsg(fd, &mut msg, 0);
                    if br < 0 {
                        if would_block(last_socket_error()) {
                            delay_ms(300);
                            continue;
                        }
                        // quit if there's a failure. Presumably this is because another thread close()'d the socket, to unblock the thread and request termination.
                        break;
                    }

                    // we don't currently bother parsing for specifics, we just use this as a signal to reenumerate interfaces.
                    INTERFACES_HAVE_CHANGED.store(1, Ordering::SeqCst);
                }

                libc::close(fd);
            }

            0
        }
    }

    pub(super) fn init_interface_change_notifications() -> bool {
        // LINUX/BSD VERSION
        #[cfg(any(
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "dragonfly"
        ))]
        {
            match Thread::spawn(
                "SDLNetIfaceEnum",
                monitor::interface_change_notification_thread,
            ) {
                Ok(thread) => {
                    *monitor::INTERFACE_CHANGE_NOTIFICATIONS_THREAD
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = Some(thread);
                    true
                }
                Err(_) => false,
            }
        }
        #[cfg(not(any(
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "dragonfly"
        )))]
        {
            // (unknown network monitoring system for this platform - will not report network changes)
            true
        }
    }

    pub(super) fn quit_interface_change_notifications() {
        // LINUX/BSD VERSION
        #[cfg(any(
            target_vendor = "apple",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "dragonfly"
        ))]
        {
            let thread = monitor::INTERFACE_CHANGE_NOTIFICATIONS_THREAD
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(thread) = thread {
                monitor::INTERFACE_CHANGE_NOTIFICATIONS_FLAG.store(1, Ordering::SeqCst);
                thread.wait();
                monitor::INTERFACE_CHANGE_NOTIFICATIONS_FLAG.store(0, Ordering::SeqCst);
            }
        }
    }

    pub(super) fn refresh_interfaces() {
        // LINUX/BSD VERSION
        let mut ifaddr: *mut libc::ifaddrs = std::ptr::null_mut();
        // SAFETY: getifaddrs() fills in a list we free below.
        if unsafe { libc::getifaddrs(&mut ifaddr) } == -1 {
            return; // oh well.
        }

        let mut new_interfaces = Vec::new();
        let mut succeeded = true;
        // SAFETY: walking the list getifaddrs() returned.
        unsafe {
            let mut i = ifaddr as *const libc::ifaddrs;
            while !i.is_null() {
                let ifa = &*i;
                i = ifa.ifa_next;
                if ifa.ifa_name.is_null()
                    || ifa.ifa_addr.is_null()
                    || ((ifa.ifa_flags & libc::IFF_UP as libc::c_uint) == 0)
                {
                    continue;
                }

                let mut address;
                let mut broadcast = None;
                // !!! FIXME: getifaddrs doesn't return the sockaddr length, so we have to go with known protocols.  :/
                let family = (*ifa.ifa_addr).sa_family as c_int;
                if family == AF_INET {
                    address = create_sdl_net_addr_from_sock_addr(
                        ifa.ifa_addr,
                        std::mem::size_of::<libc::sockaddr_in>() as SockLen,
                    )
                    .ok();
                    if (ifa.ifa_flags & libc::IFF_BROADCAST as libc::c_uint) != 0
                        && !ifa.ifa_dstaddr.is_null()
                    {
                        // (ifa_broadaddr is ifa_dstaddr on these systems.)
                        broadcast = create_sdl_net_addr_from_sock_addr(
                            ifa.ifa_dstaddr,
                            std::mem::size_of::<libc::sockaddr_in>() as SockLen,
                        )
                        .ok();
                    }
                } else if family == AF_INET6 {
                    address = create_sdl_net_addr_from_sock_addr(
                        ifa.ifa_addr,
                        std::mem::size_of::<libc::sockaddr_in6>() as SockLen,
                    )
                    .ok();
                } else {
                    continue;
                }

                if let Some(addr) = address.as_mut() {
                    // chop off interface name.
                    if let Some(data) = Arc::get_mut(&mut addr.0) {
                        if let Some(human_readable) = data.human_readable.get_mut() {
                            if let Some(pos) = human_readable.find('%') {
                                human_readable.truncate(pos);
                            }
                        }
                    }
                    new_interfaces.push(NetworkInterface {
                        name: c_str_to_string(ifa.ifa_name),
                        index: libc::if_nametoindex(ifa.ifa_name),
                        address: address.unwrap(),
                        broadcast,
                    });
                } else {
                    drop(broadcast); // just in case.
                    succeeded = false;
                    break;
                }
            }

            libc::freeifaddrs(ifaddr);
        }

        if succeeded {
            replace_interfaces(new_interfaces);
        }
        // if this was a failure case, oh well, old (maybe incorrect) ones will have to stand.
    }
}

#[cfg(not(any(unix, windows)))]
mod ifaces {
    // implement me for your platform.
    pub(super) fn init_interface_change_notifications() -> bool {
        true
    }

    pub(super) fn quit_interface_change_notifications() {}

    pub(super) fn refresh_interfaces() {}
}

use ifaces::{
    init_interface_change_notifications, quit_interface_change_notifications, refresh_interfaces,
};

fn interfaces_ready() -> bool {
    if INTERFACE_INIT.should_init() {
        if !init_interface_change_notifications() {
            INTERFACE_INIT.set_initialized(false);
            return false;
        }
        INTERFACES_HAVE_CHANGED.store(1, Ordering::SeqCst); // force refresh at start.
        INTERFACE_INIT.set_initialized(true);
    }

    // if there were changes, mark it as unchanged and then we'll enumerate. So if it changes again while enumerating, we'll pick that up next time.
    if INTERFACES_HAVE_CHANGED
        .compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        refresh_interfaces();
        /* #if 0
        SDL_Log("NETWORK INTERFACE REFRESH");
        ... (each interface's index, name, address and broadcast address)
        #endif */
    }

    true
}

// this blocks!
fn resolve_address(addr: &Address) -> i32 {
    let data = &*addr.0;
    let mut ainfo: *mut AddrInfo = std::ptr::null_mut();

    let hostname = data
        .hostname
        .as_ref()
        .expect("only NET_ResolveHostname() addresses get resolved"); // we control all this, so this shouldn't happen.
                                                                      //SDL_Log("getaddrinfo '%s'", addr->hostname);
                                                                      // SAFETY: a valid C string, no hints, and an out-pointer.
    let rc = unsafe {
        sys::getaddrinfo(
            hostname.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            &mut ainfo,
        )
    };
    //SDL_Log("rc=%d", rc);
    if rc != 0 {
        let _ = data.errstr.set(create_get_addr_info_error_string(rc));
        return NET_FAILURE; // error
    } else if ainfo.is_null() {
        let _ = data
            .errstr
            .set("Unknown error (query succeeded but result was NULL!)".to_string());
        return NET_FAILURE;
    }
    let ainfo = AddrInfoList(ainfo);

    let mut buf = [0 as c_char; 128];
    let first = ainfo.first();
    // SAFETY: `ai_addr` holds `ai_addrlen` bytes; the buffer is valid for its length.
    let rc = unsafe {
        sys::getnameinfo(
            first.ai_addr,
            first.ai_addrlen as SockLen,
            buf.as_mut_ptr(),
            buf.len(),
            std::ptr::null_mut(),
            0,
            NI_NUMERICHOST,
        )
    };
    if rc != 0 {
        let _ = data.errstr.set(create_get_addr_info_error_string(rc));
        return NET_FAILURE; // error (`ainfo` is freed on the way out)
    }

    let _ = data.human_readable.set(buf_to_string(&buf));
    // SAFETY: `first` is a valid addrinfo.
    let _ = data
        .ainfo
        .set(unsafe { AddrInfoData::from_addrinfo(first) });

    NET_SUCCESS // success (zero means "still in progress").
}

fn resolver_thread(threadnum: usize) -> i32 {
    //SDL_Log("ResolverThread #%d starting up!", threadnum);

    let mut guard = RESOLVER_LOCK.lock();

    while RESOLVER_SHUTDOWN.load(Ordering::SeqCst) == 0 {
        let addr = guard.borrow_mut().queue.pop(); // take this task off the list
        let Some(addr) = addr else {
            if RESOLVER_NUM_THREADS.load(Ordering::SeqCst) > MIN_RESOLVER_THREADS as i32 {
                // nothing pending and too many threads waiting in reserve? Quit.
                let me = guard.borrow_mut().threads[threadnum].take();
                if let Some(me) = me {
                    me.detach(); // detach ourselves so no one has to wait on us.
                }
                break; // we quit. They'll spawn new threads if necessary.
            }

            // Block until there's something to do.
            RESOLVER_CONDITION.wait(&guard); // surrenders the lock, sleeps until alerted, then relocks.
            continue; // check for shutdown and new work again!
        };

        // then release the lock so others can work.
        drop(guard);

        //SDL_Log("ResolverThread #%d got new task ('%s')", threadnum, addr->hostname);

        let simulated_loss = RESOLVER_PERCENT_LOSS.load(Ordering::SeqCst);

        if should_simulate_loss(simulated_loss) {
            // won the percent_loss lottery? Delay resolving this address between 250 and 7000 milliseconds
            delay_ms(random_number_between(250, 2000 + (50 * simulated_loss)));
        }

        let outcome = if should_simulate_loss(simulated_loss) {
            let _ = addr.0.errstr.set("simulated failure".to_string());
            NET_FAILURE
        } else {
            resolve_address(&addr)
        };

        addr.0.status.store(outcome, Ordering::SeqCst);
        //SDL_Log("ResolverThread #%d finished current task (%s, '%s' => '%s')", threadnum, (outcome == NET_FAILURE) ? "failure" : "success", addr->hostname, (outcome < 0) ? addr->errstr : addr->human_readable);

        drop(addr); // we're done with it, but others might still own it.

        RESOLVER_NUM_REQUESTS.fetch_add(-1, Ordering::SeqCst);

        // okay, we're done with this task, grab the lock so we can see what's next.
        guard = RESOLVER_LOCK.lock();
        RESOLVER_CONDITION.broadcast(); // wake up anything waiting on results, and also give all resolver threads a chance to see if they are still needed.
    }

    RESOLVER_NUM_THREADS.fetch_add(-1, Ordering::SeqCst);
    drop(guard); // we're quitting, let go of the lock.

    //SDL_Log("ResolverThread #%d ending!", threadnum);
    0
}

/// Start resolver thread `num` (`resolver_lock` held). Returns whether it
/// started.
fn spin_resolver_thread(state: &RefCell<ResolverState>, num: usize) -> Result<()> {
    let name = format!("SDLNetRslv{num}");
    debug_assert!(state.borrow().threads[num].is_none());
    RESOLVER_NUM_THREADS.fetch_add(1, Ordering::SeqCst);
    match Thread::builder()
        .name(name)
        .stack_size(64 * 1024)
        .spawn(move || resolver_thread(num))
    {
        Ok(thread) => {
            state.borrow_mut().threads[num] = Some(thread);
            Ok(())
        }
        Err(e) => {
            RESOLVER_NUM_THREADS.fetch_add(-1, Ordering::SeqCst);
            Err(e)
        }
    }
}

// (DestroyAddress() is dropping the last `Address`.)

/// SAFETY: `saddr` must hold `saddrlen` bytes of a socket address.
unsafe fn create_sdl_net_addr_from_sock_addr(
    saddr: *const SockAddr,
    saddrlen: SockLen,
) -> Result<Address> {
    // !!! FIXME: this all seems inefficient in the name of keeping addresses generic.
    let mut hostbuf = [0 as c_char; 128];
    // SAFETY: as the caller promises; the buffer is valid for its length.
    let gairc = unsafe {
        sys::getnameinfo(
            saddr,
            saddrlen,
            hostbuf.as_mut_ptr(),
            hostbuf.len(),
            std::ptr::null_mut(),
            0,
            NI_NUMERICHOST,
        )
    };
    if gairc != 0 {
        return Err(set_get_addr_info_error(
            "Failed to determine address",
            gairc,
        ));
    }

    // SAFETY: an all-zero addrinfo is valid hints.
    let mut hints: AddrInfo = unsafe { std::mem::zeroed() };
    // SAFETY: as the caller promises.
    hints.ai_family = unsafe { (*saddr).sa_family } as c_int;
    hints.ai_socktype = SOCK_DGRAM;
    hints.ai_protocol = 0;
    hints.ai_flags = AI_NUMERICHOST;

    let mut ainfo: *mut AddrInfo = std::ptr::null_mut();
    // SAFETY: a valid C string, valid hints and an out-pointer.
    let gairc = unsafe { sys::getaddrinfo(hostbuf.as_ptr(), std::ptr::null(), &hints, &mut ainfo) };
    if gairc != 0 {
        return Err(set_get_addr_info_error(
            "Failed to determine address",
            gairc,
        ));
    }
    let ainfo = AddrInfoList(ainfo);

    let data = AddressData {
        hostname: None,
        human_readable: OnceLock::new(),
        errstr: OnceLock::new(),
        status: AtomicI32::new(NET_SUCCESS),
        ainfo: OnceLock::new(),
    };
    let _ = data.human_readable.set(buf_to_string(&hostbuf));
    // SAFETY: a valid addrinfo from getaddrinfo().
    let _ = data
        .ainfo
        .set(unsafe { AddrInfoData::from_addrinfo(ainfo.first()) });

    Ok(Address(Arc::new(data)))
}

static INITIALIZE_COUNT: AtomicI32 = AtomicI32::new(0);

/// Initialize the SDL_net library. Translation of `NET_Init()`.
///
/// This must be successfully called once before (almost) any other SDL_net
/// function can be used. It is safe to call this multiple times; the
/// library will only initialize once, and won't deinitialize until
/// [`quit`] has been called a matching number of times. Extra attempts to
/// init report success.
pub fn init() -> Result<()> {
    if INITIALIZE_COUNT.fetch_add(1, Ordering::SeqCst) > 0 {
        return Ok(()); // already initialized, call it a success.
    }

    #[cfg(windows)]
    {
        // SAFETY: an all-zero WSADATA is a valid out-parameter.
        let mut data: sys::WSADATA = unsafe { std::mem::zeroed() };
        // MAKEWORD(1, 1)
        // SAFETY: a valid out-parameter.
        let rc = unsafe { sys::WSAStartup(0x0101, &mut data) };
        if rc != 0 {
            // FIXME (upstream): this returns without undoing the
            // `initialize_count` bump above, so every later NET_Init()
            // would report success with WinSock down; it is undone here.
            // (And WSAStartup() returns its error rather than setting
            // WSAGetLastError(), so this uses the return value.)
            INITIALIZE_COUNT.fetch_add(-1, Ordering::SeqCst);
            return Err(set_socket_error("WSAStartup() failed", rc));
        }
        *WINSOCK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(Winsock));
    }
    #[cfg(unix)]
    {
        // SAFETY: ignoring SIGPIPE, as upstream does.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    }

    let result = (|| -> Result<()> {
        {
            let guard = RESOLVER_LOCK.lock();
            let mut state = guard.borrow_mut();
            state.threads = [const { None }; MAX_RESOLVER_THREADS];
            state.queue.clear();
        }
        RESOLVER_SHUTDOWN.store(0, Ordering::SeqCst);
        RESOLVER_NUM_THREADS.store(0, Ordering::SeqCst);
        RESOLVER_NUM_REQUESTS.store(0, Ordering::SeqCst);
        RESOLVER_PERCENT_LOSS.store(0, Ordering::SeqCst);

        // (resolver_lock and resolver_condition are statics.)

        {
            let guard = RESOLVER_LOCK.lock();
            for i in 0..MIN_RESOLVER_THREADS {
                spin_resolver_thread(&guard, i)?;
            }
        }

        // SAFETY: an all-zero sockaddr_in6 is valid.
        let mut sa_in6: sys::SockAddrIn6 = unsafe { std::mem::zeroed() };
        if let Some(bytes) = sys::inet_pton6("ff02::1") {
            // SAFETY: sin6_addr is 16 bytes.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    &mut sa_in6.sin6_addr as *mut _ as *mut u8,
                    16,
                )
            };
            sa_in6.sin6_family = AF_INET6 as _;
            // SAFETY: a sockaddr_in6 of the size given.
            let addr = unsafe {
                create_sdl_net_addr_from_sock_addr(
                    (&sa_in6 as *const sys::SockAddrIn6).cast(),
                    std::mem::size_of::<sys::SockAddrIn6>() as SockLen,
                )
            };
            *IPV6_BROADCAST_ADDR
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = addr.ok();
        }

        // (interface_rwlock is a static.)

        Ok(()) // good to go.
    })();

    if let Err(origerrstr) = result {
        // failed:
        quit();
        return Err(origerrstr);
    }

    Ok(())
}

/// Deinitialize the SDL_net library. Translation of `NET_Quit()`.
///
/// It is safe to call this multiple times; the library will only
/// deinitialize once, when this function is called the same number of
/// times as [`init`] was successfully called. Once deinitialized, it is
/// safe to call [`init`] again.
///
/// Addresses and sockets stay valid after this (upstream wants them
/// destroyed first); on Windows, WinSock stays up until the last socket is
/// dropped. Addresses still being resolved stay unresolved.
pub fn quit() {
    let prevcount = INITIALIZE_COUNT.fetch_add(-1, Ordering::SeqCst);
    if prevcount <= 0 {
        INITIALIZE_COUNT.fetch_add(1, Ordering::SeqCst); // bump back up.
        return; // we weren't initialized!
    } else if prevcount > 1 {
        return; // need to quit more, to match previous init calls.
    }

    {
        let mut guard = RESOLVER_LOCK.lock();
        RESOLVER_SHUTDOWN.store(1, Ordering::SeqCst);
        for i in 0..MAX_RESOLVER_THREADS {
            let thread = guard.borrow_mut().threads[i].take();
            if let Some(thread) = thread {
                RESOLVER_CONDITION.broadcast();
                drop(guard);
                thread.wait();
                guard = RESOLVER_LOCK.lock();
                // (resolver_threads[i] = NULL; was done by the take().)
            }
        }
        drop(guard);
    }

    RESOLVER_SHUTDOWN.store(0, Ordering::SeqCst);
    RESOLVER_NUM_THREADS.store(0, Ordering::SeqCst);
    RESOLVER_NUM_REQUESTS.store(0, Ordering::SeqCst);
    RESOLVER_PERCENT_LOSS.store(0, Ordering::SeqCst);

    // (resolver_condition and resolver_lock are statics.)

    // (Upstream leaks the queue's references to addresses that were never
    // resolved; they are dropped here.)
    let queue = std::mem::take(&mut RESOLVER_LOCK.lock().borrow_mut().queue);
    drop(queue);

    if INTERFACE_INIT.should_quit() {
        quit_interface_change_notifications();
        INTERFACES_HAVE_CHANGED.store(0, Ordering::SeqCst);
        let interfaces = std::mem::take(&mut *write_interfaces());
        drop(interfaces);
        INTERFACE_INIT.set_initialized(false);
    }

    // asserts to catch that these shouldn't have ever been set if we never initialized interface_init...
    debug_assert!(read_interfaces().is_empty());

    // (interface_rwlock is a static.)

    let ipv6 = IPV6_BROADCAST_ADDR
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    drop(ipv6);

    #[cfg(windows)]
    {
        // WSACleanup(), once the last socket lets go of WinSock.
        let winsock = WINSOCK.lock().unwrap_or_else(|e| e.into_inner()).take();
        drop(winsock);
    }
}

/// `SDL_isspace()`.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0B' | b'\x0C' | b'\r')
}

fn trim_whitespace(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|&c| !is_space(c)).unwrap_or(s.len());
    let s = &s[start..];
    let end = s.iter().rposition(|&c| !is_space(c)).map_or(0, |i| i + 1);
    &s[..end]
}

/// Resolve a human-readable hostname. Translation of `NET_ResolveHostname()`.
///
/// This converts a hostname (like `www.libsdl.org`), or an IP address string
/// (like `"159.203.69.7"`), into an [`Address`]. Resolving is asynchronous,
/// done on a background thread: this doesn't block, and returns an
/// unresolved address that can't be used until it resolves. Wait for that
/// with [`Address::wait_until_resolved`], or poll with [`Address::status`].
///
/// Whitespace around the name is ignored, and the name ends at a NUL
/// character, as in C. [`init`] must have been called.
pub fn resolve_hostname(host: &str) -> Result<Address> {
    // If this isn't true, we'll spin up resolver threads without locking that will be orphaned in NET_Init()
    if INITIALIZE_COUNT.load(Ordering::SeqCst) <= 0 {
        // (Upstream asserts this; a build without asserts queues an address that never resolves.)
        return Err(Error::new("SDL_net is not initialized"));
    }

    // (The C string ends at the first NUL.)
    let host = host.as_bytes();
    let host = &host[..host.iter().position(|&c| c == 0).unwrap_or(host.len())];

    // remove whitespace around name, just in case. https://github.com/libsdl-org/SDL_net/issues/148
    let hostname = CString::new(trim_whitespace(host)).expect("no NULs left");

    let addr = Address(Arc::new(AddressData {
        hostname: Some(hostname),
        human_readable: OnceLock::new(),
        errstr: OnceLock::new(),
        status: AtomicI32::new(NET_WAITING),
        ainfo: OnceLock::new(),
    }));

    // (refcount 2: one for creation, one for the resolver thread to unref when done.)
    let guard = RESOLVER_LOCK.lock();

    // !!! FIXME: this should append to the list, not prepend; as is, new requests will make existing pending requests take longer to start processing.
    // (The queue's end is the list's head.)
    guard.borrow_mut().queue.push(addr.clone());

    let num_threads = RESOLVER_NUM_THREADS.load(Ordering::SeqCst);
    let num_requests = RESOLVER_NUM_REQUESTS.fetch_add(1, Ordering::SeqCst) + 1;
    //SDL_Log("num_threads=%d, num_requests=%d", num_threads, num_requests);
    if (num_requests >= num_threads) && (num_threads < MAX_RESOLVER_THREADS as i32) {
        // all threads are busy? Maybe spawn a new one.
        // if this didn't actually spin one up, it is what it is...the existing threads will eventually get there.
        let free = guard.borrow().threads.iter().position(|t| t.is_none());
        if let Some(i) = free {
            let _ = spin_resolver_thread(&guard, i);
        }
    }

    RESOLVER_CONDITION.signal();
    drop(guard);

    Ok(addr)
}

impl Address {
    /// Block until the address is resolved. Translation of
    /// `NET_WaitUntilResolved()`.
    ///
    /// `timeout` is in milliseconds: -1 waits indefinitely, 0 checks the
    /// current status and returns immediately (as [`status`](Self::status)
    /// does). Returns [`Status::Success`] once resolved,
    /// [`Status::Waiting`] if this timed out first, and the error if
    /// resolution failed.
    pub fn wait_until_resolved(&self, timeout: i32) -> Result<Status> {
        // we _could_ use a different lock for this, but this is Good Enough.

        if timeout != 0 {
            let guard = RESOLVER_LOCK.lock();
            if timeout < 0 {
                while self.0.status.load(Ordering::SeqCst) == NET_WAITING {
                    RESOLVER_CONDITION.wait(&guard);
                }
            } else {
                let endtime = get_ticks() + timeout as u64;
                while self.0.status.load(Ordering::SeqCst) == NET_WAITING {
                    let now = get_ticks();
                    if now >= endtime {
                        break;
                    }
                    RESOLVER_CONDITION
                        .wait_timeout(&guard, Some(Duration::from_millis(endtime - now)));
                }
            }
            drop(guard);
        }

        self.status() // so we set the error string if necessary.
    }

    /// Check if the address is resolved, without blocking. Translation of
    /// `NET_GetAddressStatus()`.
    pub fn status(&self) -> Result<Status> {
        let retval = self.0.status.load(Ordering::SeqCst);
        match retval {
            NET_FAILURE => Err(Error::new(self.0.errstr.get().cloned().unwrap_or_default())),
            NET_SUCCESS => Ok(Status::Success),
            _ => Ok(Status::Waiting),
        }
    }

    /// A human-readable string for the resolved address, like
    /// `"159.203.69.7"` or `"2604:a880:800:a1::71f:3001"` (not the original
    /// hostname). Translation of `NET_GetAddressString()`.
    ///
    /// Fails while resolution is in progress ("Address not yet resolved")
    /// or if it failed.
    pub fn address_string(&self) -> Result<&str> {
        let retval = self.0.human_readable.get();
        match retval {
            Some(s) => Ok(s),
            None => {
                self.status()?; // if NET_FAILURE, it'll set the error message.
                Err(Error::new("Address not yet resolved")) // if this resolved in a race condition, too bad, try again.
            }
        }
    }

    /// The protocol-level bytes of the resolved address (the system's
    /// socket address structure: not human-readable, protocol-specific,
    /// and with no promise the format won't change). Translation of
    /// `NET_GetAddressBytes()`.
    pub fn bytes(&self) -> Result<&[u8]> {
        let rc = self.status()?; // if NET_FAILURE, it'll set the error message.
        if rc == Status::Success {
            let ainfo = self.0.ainfo.get();
            debug_assert!(ainfo.is_some());
            Ok(ainfo.map_or(&[][..], |a| &a.addr[..]))
        } else {
            Err(Error::new("Address not yet resolved"))
        }
    }

    fn is_resolved(&self) -> bool {
        self.0.status.load(Ordering::SeqCst) == NET_SUCCESS
    }

    fn human_readable(&self) -> Option<&str> {
        self.0.human_readable.get().map(String::as_str)
    }
}

/// Compare two addresses (`None` is a NULL address), for sorting.
/// Translation of `NET_CompareAddresses()`.
///
/// Addresses order by protocol family, then by the length of the system's
/// address structure, then by its bytes; the same address object is equal
/// to itself, and a NULL address sorts after any other. Unresolved
/// addresses sort after resolved ones.
pub fn compare_addresses(
    sdlneta: Option<&Address>,
    sdlnetb: Option<&Address>,
) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let (sdlneta, sdlnetb) = match (sdlneta, sdlnetb) {
        (None, None) => return Equal, // same pointer?
        (Some(_), None) => return Less,
        (None, Some(_)) => return Greater,
        (Some(a), Some(b)) => (a, b),
    };
    if Arc::ptr_eq(&sdlneta.0, &sdlnetb.0) {
        return Equal; // same pointer?
    }

    let a = sdlneta.0.ainfo.get();
    let b = sdlnetb.0.ainfo.get();
    let (a, b) = match (a, b) {
        (None, None) => return Equal, // same pointer?
        (Some(_), None) => return Less,
        (None, Some(_)) => return Greater,
        (Some(a), Some(b)) => (a, b),
    };
    if a.family < b.family {
        return Less;
    } else if a.family > b.family {
        return Greater;
    } else if a.addr.len() < b.addr.len() {
        return Less;
    } else if a.addr.len() > b.addr.len() {
        return Greater;
    }

    a.addr.cmp(&b.addr) // (SDL_memcmp())
}

impl PartialEq for Address {
    fn eq(&self, other: &Address) -> bool {
        compare_addresses(Some(self), Some(other)) == std::cmp::Ordering::Equal
    }
}

impl Eq for Address {}

impl PartialOrd for Address {
    fn partial_cmp(&self, other: &Address) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Address {
    fn cmp(&self, other: &Address) -> std::cmp::Ordering {
        compare_addresses(Some(self), Some(other))
    }
}

// (NET_RefAddress() is `Clone`, NET_UnrefAddress() is `Drop`.)

/// Enable simulated address resolution failures. Translation of
/// `NET_SimulateAddressResolutionLoss()`.
///
/// `percent_loss` (clamped to 0..=100) of resolutions will be delayed (by
/// 250 milliseconds to several seconds) and that many again will fail, with
/// "simulated failure". Zero (the default) disables the simulation. This is
/// for debugging, to simulate real-world conditions.
pub fn simulate_address_resolution_loss(percent_loss: i32) {
    RESOLVER_PERCENT_LOSS.store(percent_loss.clamp(0, 100), Ordering::SeqCst);
}

/// The addresses of the system's network interfaces: the ones a socket
/// can theoretically be bound to. Translation of `NET_GetLocalAddresses()`
/// (and, as the `Vec` is freed on drop, `NET_FreeLocalAddresses()`).
///
/// You almost never need this: usually it's better to create servers and
/// datagram sockets with no address, to bind to all interfaces.
pub fn get_local_addresses() -> Result<Vec<Address>> {
    if interfaces_ready() {
        let interfaces = read_interfaces();
        Ok(interfaces.iter().map(|i| i.address.clone()).collect())
    } else {
        // (InterfacesReady() fails only when the platform's change
        // notifications can't be set up.)
        Err(Error::new("Failed to monitor network interfaces"))
    }
}

fn make_addr_info_with_port(
    addr: Option<&Address>,
    socktype: c_int,
    port: u16,
) -> Result<AddrInfoList> {
    let ainfo = addr.and_then(|a| a.0.ainfo.get());
    debug_assert!(addr.is_none() || ainfo.is_some());

    // we need to set up a sockaddr with the port in it for connect(), etc, which is kind of a pain, since we
    // want to keep things generic and also not set up a port at resolve time.
    // SAFETY: an all-zero addrinfo is valid hints.
    let mut hints: AddrInfo = unsafe { std::mem::zeroed() };
    hints.ai_family = ainfo.map_or(AF_UNSPEC, |a| a.family);
    hints.ai_socktype = socktype;
    //hints.ai_protocol = ainfo ? ainfo->ai_protocol : 0;
    hints.ai_flags = AI_NUMERICHOST | AI_NUMERICSERV | if ainfo.is_none() { AI_PASSIVE } else { 0 };

    let service = CString::new(format!("{}", port as i32)).expect("digits");

    let node = addr
        .and_then(|a| a.human_readable())
        .map(|s| CString::new(s).expect("no NULs in an address string"));

    let mut addrwithport: *mut AddrInfo = std::ptr::null_mut();
    // SAFETY: valid (or NULL) C strings, valid hints and an out-pointer.
    let rc = unsafe {
        sys::getaddrinfo(
            node.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
            service.as_ptr(),
            &hints,
            &mut addrwithport,
        )
    };
    if rc != 0 {
        let errstr = create_get_addr_info_error_string(rc);
        return Err(Error::new(format!(
            "Failed to prepare address with port: {errstr}"
        )));
    }

    Ok(AddrInfoList(addrwithport))
}

/// The `SDL_PropertiesID` boolean lookup with a default
/// (`SDL_GetBooleanProperty()`); `None` is property group zero.
fn get_boolean_property(props: Option<&Properties>, name: &str, default_value: bool) -> bool {
    props
        .and_then(|p| p.get_bool(name))
        .unwrap_or(default_value)
}

/// A reliable, stream-oriented connection to another system (TCP).
/// Translation of `NET_StreamSocket`.
///
/// Created by [`StreamSocket::create_client`] (connecting to a server) or
/// [`Server::accept_client`] (a client connecting to us). Dropping it is
/// `NET_DestroyStreamSocket()`: it disconnects, abandoning data still
/// queued for sending (see [`wait_until_drained`](Self::wait_until_drained)).
///
/// Stream sockets are `Send` but not `Sync`: one thread at a time, as
/// upstream requires.
pub struct StreamSocket {
    addr: Address,
    port: u16,
    handle: Socket,
    status: i32,
    /// The error that came with `status == NET_FAILURE` (the message
    /// upstream leaves in `SDL_GetError()` when the connection fails).
    status_error: Option<Error>,
    /// `pending_output_buffer`; its length is `pending_output_len` and its
    /// capacity `pending_output_allocation`.
    pending_output: Vec<u8>,
    percent_loss: i32,
    simulated_failure_until: u64,
    _winsock: WinsockRef,
}

impl std::fmt::Debug for StreamSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamSocket")
            .field("addr", &self.addr)
            .field("port", &self.port)
            .field("status", &self.status)
            .field("pending_output_len", &self.pending_output.len())
            .field("percent_loss", &self.percent_loss)
            .finish()
    }
}

impl StreamSocket {
    /// Begin connecting a socket as a client to a remote server. Translation
    /// of `NET_CreateClient()`.
    ///
    /// Connecting is asynchronous: this returns before the connection is
    /// complete; see [`wait_until_connected`](Self::wait_until_connected)
    /// and [`connection_status`](Self::connection_status). `address` must
    /// be resolved. The port is a normal integer, in host byte order.
    /// There are no properties for clients yet; `props` is for future use.
    pub fn create_client(
        addr: &Address,
        port: u16,
        props: Option<&Properties>,
    ) -> Result<StreamSocket> {
        let _ = props;
        if !addr.is_resolved() {
            return Err(Error::new("Address is not resolved"));
        }

        // we need to set up a sockaddr with the port in it for connect(), which is kind of a pain, since we
        // want to keep things generic and also not set up a port at resolve time.
        let addrwithport = make_addr_info_with_port(Some(addr), SOCK_STREAM, port)?;
        let ai = addrwithport.first();

        let winsock = winsock_ref();
        // SAFETY: plain socket creation.
        let handle = unsafe { sys::socket(ai.ai_family, ai.ai_socktype, ai.ai_protocol) };
        if handle == INVALID_SOCKET {
            return Err(set_last_socket_error("Failed to create socket"));
        }

        if make_socket_nonblocking(handle) < 0 {
            close_socket_handle(handle);
            return Err(Error::new("Failed to make new socket non-blocking"));
        }

        // SAFETY: `ai_addr` holds `ai_addrlen` bytes.
        let rc = unsafe { sys::connect(handle, ai.ai_addr, ai.ai_addrlen as SockLen) };

        drop(addrwithport);

        if rc == SOCKET_ERROR {
            let err = last_socket_error();
            if !would_block(err) {
                let e = set_socket_error("Connection failed at startup", err);
                close_socket_handle(handle);
                return Err(e);
            }
        }

        Ok(StreamSocket {
            addr: addr.clone(),
            port,
            handle,
            status: NET_WAITING,
            status_error: None,
            pending_output: Vec::new(),
            percent_loss: 0,
            simulated_failure_until: 0,
            _winsock: winsock,
        })
    }

    fn check_client_connection(&mut self, timeoutms: i32) -> Result<Status> {
        if self.status == NET_WAITING {
            // still pending?
            /* !!! FIXME: add this later?
            if (sock->simulated_failure_ticks) {
                if (SDL_GetTicks() >= sock->simulated_failure_ticks) {
                    sock->status = (NET_Status) SDL_SetError("simulated failure");
            } else */
            let rc = wait_until_input_available(&mut [GenericSocket::Stream(self)], timeoutms);
            if let Err(e) = rc {
                self.status = NET_FAILURE; // just abandon the whole enterprise.
                self.status_error = Some(e);
            }
        }
        match self.status {
            NET_SUCCESS => Ok(Status::Success),
            NET_WAITING => Ok(Status::Waiting),
            _ => Err(self
                .status_error
                .clone()
                .unwrap_or_else(|| Error::new("Socket failed to connect"))),
        }
    }

    /// Block until the socket has connected to the server. Translation of
    /// `NET_WaitUntilConnected()`.
    ///
    /// `timeout` is in milliseconds: -1 waits indefinitely, 0 checks the
    /// current status and returns immediately (as
    /// [`connection_status`](Self::connection_status) does). Returns
    /// [`Status::Success`] once connected, [`Status::Waiting`] if this
    /// timed out first, and the error if the connection failed.
    pub fn wait_until_connected(&mut self, timeout: i32) -> Result<Status> {
        self.check_client_connection(timeout)
    }

    /// Check if the socket is connected, without blocking. Translation of
    /// `NET_GetConnectionStatus()`.
    ///
    /// This only covers getting connected: a connection that succeeded and
    /// was dropped later still reports success (reads and writes fail).
    pub fn connection_status(&mut self) -> Result<Status> {
        self.check_client_connection(0)
    }
}

/// The receiving end of stream connections, what BSD sockets calls a
/// "listen socket". Translation of `NET_Server`.
///
/// Dropping it is `NET_DestroyServer()`: pending connections not yet
/// accepted are disconnected; accepted ones carry on.
pub struct Server {
    addr: Option<Address>, // bound to this address (NULL for any).
    port: u16,
    handles: Vec<Socket>, // for INADDR_ANY things, one handle per network family.
    _winsock: WinsockRef,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("addr", &self.addr)
            .field("port", &self.port)
            .field("num_handles", &self.handles.len())
            .finish()
    }
}

impl Server {
    /// Create a server, listening for connections to accept. Translation of
    /// `NET_CreateServer()`.
    ///
    /// `addr` is the local address to listen on (one of
    /// [`get_local_addresses`], say), or `None` to listen on every
    /// interface. The port is a normal integer, in host byte order. This
    /// doesn't block: if it succeeds, connections can be accepted with
    /// [`accept_client`](Self::accept_client) at once.
    ///
    /// Properties: [`PROP_SERVER_REUSEADDR_BOOLEAN`] (default `true`), to
    /// create the server even if a previous one recently used the address.
    pub fn create(addr: Option<&Address>, port: u16, props: Option<&Properties>) -> Result<Server> {
        if let Some(addr) = addr {
            if !addr.is_resolved() {
                return Err(Error::new("Address is not resolved")); // strictly speaking, this should be a local interface, but a resolved address can fail later.
            }
        }

        let addrwithport = make_addr_info_with_port(addr, SOCK_STREAM, port)?;

        let mut server = Server {
            addr: addr.cloned(),
            port,
            handles: Vec::new(),
            _winsock: winsock_ref(),
        };

        let num_handles = if addr.is_some() {
            1
        } else {
            // bind to all interfaces.
            addrwithport.iter().count()
        };

        let reuseaddr: c_int = if get_boolean_property(props, PROP_SERVER_REUSEADDR_BOOLEAN, true) {
            1
        } else {
            0
        };

        // Make sockets for all desired interfaces; if addr!=NULL, this is one socket on one interface,
        //  but if addr==NULL, it might be multiple sockets for IPv4, IPv6, etc, bound to their INADDR_ANY equivalent.
        // (On failure, dropping `server` closes the handles made so far.)
        for ainfo in addrwithport.iter().take(num_handles) {
            // SAFETY: plain socket creation.
            let handle =
                unsafe { sys::socket(ainfo.ai_family, ainfo.ai_socktype, ainfo.ai_protocol) };
            if handle == INVALID_SOCKET {
                return Err(set_last_socket_error("Failed to create listen socket"));
            }

            server.handles.push(handle);

            if make_socket_nonblocking(handle) < 0 {
                return Err(Error::new("Failed to make new listen socket non-blocking"));
            }

            let one: c_int = 1;
            if ainfo.ai_family == AF_INET6 {
                // SAFETY: an int option.
                unsafe {
                    sys::setsockopt(
                        handle,
                        sys::IPPROTO_IPV6,
                        sys::IPV6_V6ONLY,
                        (&one as *const c_int).cast(),
                        std::mem::size_of::<c_int>() as SockLen,
                    )
                }; // if this fails, oh well.
            }

            // SAFETY: an int option.
            unsafe {
                sys::setsockopt(
                    handle,
                    sys::SOL_SOCKET,
                    sys::SO_REUSEADDR,
                    (&reuseaddr as *const c_int).cast(),
                    std::mem::size_of::<c_int>() as SockLen,
                )
            };

            // SAFETY: `ai_addr` holds `ai_addrlen` bytes.
            let rc = unsafe { sys::bind(handle, ainfo.ai_addr, ainfo.ai_addrlen as SockLen) };
            if rc == SOCKET_ERROR {
                let err = last_socket_error();
                debug_assert!(!would_block(err)); // binding shouldn't be a blocking operation.
                return Err(set_socket_error("Failed to bind listen socket", err));
            }

            // SAFETY: plain listen().
            let rc = unsafe { sys::listen(handle, 16) };
            if rc == SOCKET_ERROR {
                let err = last_socket_error();
                debug_assert!(!would_block(err)); // listen shouldn't be a blocking operation.
                return Err(set_socket_error("Failed to listen on socket", err));
            }
        }

        Ok(server)
    }

    /// A stream socket for the next pending client connection, or `None`
    /// if no connection is pending (not an error: call this in a loop until
    /// it returns `None`, to accept all pending connections in a batch).
    /// Translation of `NET_AcceptClient()`.
    ///
    /// The new socket is already connected. To sleep until a connection
    /// arrives, use [`wait_until_input_available`].
    pub fn accept_client(&mut self) -> Result<Option<StreamSocket>> {
        for &server_handle in &self.handles {
            // SAFETY: an all-zero sockaddr_storage is valid.
            let mut from: AddressStorage = unsafe { std::mem::zeroed() };
            let mut fromlen = std::mem::size_of::<AddressStorage>() as SockLen;
            // SAFETY: `from` holds `fromlen` bytes.
            let handle = unsafe {
                sys::accept(
                    server_handle,
                    (&mut from as *mut AddressStorage).cast(),
                    &mut fromlen,
                )
            };
            if handle == INVALID_SOCKET {
                let err = last_socket_error();
                if would_block(err) {
                    continue;
                }
                return Err(set_socket_error("Failed to accept new connection", err));
            }

            if make_socket_nonblocking(handle) < 0 {
                close_socket_handle(handle);
                return Err(Error::new("Failed to make incoming socket non-blocking"));
            }

            let mut portbuf = [0 as c_char; 16];
            // SAFETY: `from` holds `fromlen` bytes; the buffer is valid for its length.
            let gairc = unsafe {
                sys::getnameinfo(
                    (&from as *const AddressStorage).cast(),
                    fromlen,
                    std::ptr::null_mut(),
                    0,
                    portbuf.as_mut_ptr(),
                    portbuf.len(),
                    NI_NUMERICSERV,
                )
            };
            if gairc != 0 {
                close_socket_handle(handle);
                return Err(set_get_addr_info_error(
                    "Failed to determine port number",
                    gairc,
                ));
            }

            // SAFETY: `from` holds `fromlen` bytes.
            let fromaddr = match unsafe {
                create_sdl_net_addr_from_sock_addr((&from as *const AddressStorage).cast(), fromlen)
            } {
                Ok(fromaddr) => fromaddr,
                Err(e) => {
                    close_socket_handle(handle);
                    return Err(e); // error string was already set.
                }
            };

            let sock = StreamSocket {
                addr: fromaddr,
                port: atoi(&buf_to_string(&portbuf)) as u16,
                handle,
                status: NET_SUCCESS, // connected
                status_error: None,
                pending_output: Vec::new(),
                percent_loss: 0,
                simulated_failure_until: 0,
                _winsock: winsock_ref(),
            };

            return Ok(Some(sock)); // we got one!
        }

        Ok(None) // nothing new.
    }
}

impl Drop for Server {
    /// Translation of `NET_DestroyServer()`.
    fn drop(&mut self) {
        for &handle in &self.handles {
            if handle != INVALID_SOCKET {
                close_socket_handle(handle);
            }
        }
        // (The address is unref'd, and WinSock let go of, as the fields drop.)
    }
}

/// `SDL_atoi()`: leading whitespace, a sign, digits.
fn atoi(s: &str) -> i32 {
    let s = s.trim_start();
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut value: i32 = 0;
    for c in digits.bytes().take_while(u8::is_ascii_digit) {
        value = value.wrapping_mul(10).wrapping_add((c - b'0') as i32);
    }
    if neg {
        value.wrapping_neg()
    } else {
        value
    }
}

impl StreamSocket {
    /// The remote address of the socket (which might still be connecting).
    /// Translation of `NET_GetStreamSocketAddress()`.
    pub fn address(&self) -> Address {
        self.addr.clone()
    }

    fn update_simulated_failure(&mut self) {
        if should_simulate_loss(self.percent_loss) {
            // won the percent_loss lottery? Refuse to move more data for between 250 and 7000 milliseconds.
            self.simulated_failure_until =
                get_ticks() + random_number_between(250, 2000 + (50 * self.percent_loss)) as u64;
        } else {
            self.simulated_failure_until = 0;
        }
    }

    // see if any pending data can finally be sent, etc
    fn pump(&mut self) -> Result<()> {
        if !self.pending_output.is_empty() {
            // !!! FIXME: there should be some small chance of streams dropping connection to simulate failure.
            if self.simulated_failure_until != 0 && (get_ticks() < self.simulated_failure_until) {
                return Ok(()); // streams are reliable, so instead of packet loss, we introduce lag.
            }

            let bw = write(self.handle, &self.pending_output);
            if bw < 0 {
                let err = last_socket_error();
                return if would_block(err) {
                    Ok(())
                } else {
                    Err(set_socket_error("Failed to write to socket", err))
                };
            }
            // (memmove the rest down, and pending_output_len -= bw.)
            self.pending_output.drain(..bw as usize);

            self.update_simulated_failure();
        }

        Ok(())
    }

    /// Send bytes over the socket, or queue them for later transmission if
    /// they can't all be sent at once. Translation of
    /// `NET_WriteToStreamSocket()`.
    ///
    /// This never blocks. An error means the connection failed (the remote
    /// side dropped us, or one of a million other networking failures);
    /// the socket is then no longer usable.
    pub fn write(&mut self, buf: &[u8]) -> Result<()> {
        self.pump()?; // try to flush any queued data to the socket now, before we handle more.
        if buf.is_empty() {
            return Ok(()); // nothing to do.
        }

        let mut buf = buf;
        if self.pending_output.is_empty() {
            // nothing queued? See if we can just send this without queueing.
            // don't ever try to send directly if simulating packet loss; we'll always queue and mess with it then.
            if self.percent_loss == 0 {
                let bw = write(self.handle, buf);
                if bw < 0 {
                    let err = last_socket_error();
                    if !would_block(err) {
                        return Err(set_socket_error("Failed to write to socket", err));
                    }
                } else if bw as usize == buf.len() {
                    // sent the whole thing? We're good to go here.
                    return Ok(());
                } else {
                    /*if (bw < buflen)*/
                    // partial write? We'll queue the rest.
                    buf = &buf[bw as usize..];
                }
            }
        }

        // queue this up for sending later.
        let min_alloc = self.pending_output.len() + buf.len();
        if min_alloc > self.pending_output.capacity() {
            let mut newlen = self.pending_output.capacity().max(1);
            while newlen < min_alloc {
                newlen *= 2;
                if newlen > i32::MAX as usize {
                    // uhoh, overflowed! That's a lot of memory!!
                    return Err(Error::out_of_memory());
                }
            }
            self.pending_output
                .try_reserve_exact(newlen - self.pending_output.len())
                .map_err(|_| Error::out_of_memory())?;
        }

        self.pending_output.extend_from_slice(buf);

        Ok(())
    }

    /// The number of bytes still queued for transmission, after trying to
    /// send more (without blocking). Translation of
    /// `NET_GetStreamSocketPendingWrites()`.
    pub fn pending_writes(&mut self) -> Result<usize> {
        self.pump()?;
        Ok(self.pending_output.len())
    }

    /// Block until all queued data is sent, or `timeout` milliseconds pass
    /// (-1 waits indefinitely, 0 doesn't wait); returns the number of
    /// bytes still queued. Translation of
    /// `NET_WaitUntilStreamSocketDrained()`.
    pub fn wait_until_drained(&mut self, timeout: i32) -> Result<usize> {
        let mut timeoutms = timeout;
        if timeoutms != 0 {
            let endtime = if timeoutms > 0 {
                get_ticks() + timeoutms as u64
            } else {
                0
            };
            while matches!(self.pending_writes(), Ok(n) if n > 0) {
                let mut pfd = [pollfd {
                    fd: self.handle,
                    events: POLLOUT,
                    revents: 0,
                }];
                let rc = poll(&mut pfd, timeoutms);
                if rc == SOCKET_ERROR {
                    return Err(set_last_socket_error("Socket poll failed"));
                } else if rc == 0 {
                    break; // timed out
                }

                if timeoutms > 0 {
                    // We must have woken up for a pending write, etc. Figure out remaining wait time.
                    let now = get_ticks();
                    if now < endtime {
                        timeoutms = (endtime - now) as i32;
                    } else {
                        break; // time has expired, break out.
                    }
                } // else timeout is meant to be infinite, but we woke up for a write, etc, so go back to an infinite poll until we fail or buffer is drained.
            }
        }

        self.pending_writes()
    }

    /// Receive bytes the remote system sent: up to `buf.len()` bytes, in
    /// the order they were sent. Returns 0 if nothing is available (this
    /// never blocks). Translation of `NET_ReadFromStreamSocket()`.
    ///
    /// An error means the connection failed or ended ("End of stream" when
    /// the remote side closed it); the socket is then no longer usable.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.pump()?; // try to flush any queued data to the socket now, before we go further.
        if self.simulated_failure_until != 0 && (get_ticks() < self.simulated_failure_until) {
            return Ok(0); // streams are reliable, so instead of packet loss, we introduce lag.
        }

        if buf.is_empty() {
            return Ok(0); // nothing to do.
        }

        let br = read(self.handle, buf);
        if br == 0 {
            return Err(Error::new("End of stream"));
        } else if br < 0 {
            let err = last_socket_error();
            return if would_block(err) {
                Ok(0)
            } else {
                Err(set_socket_error("Failed to read from socket", err))
            };
        }

        self.update_simulated_failure();

        Ok(br as usize)
    }

    /// Enable simulated stream failures. Translation of
    /// `NET_SimulateStreamPacketLoss()`.
    ///
    /// Streams are reliable, so lost packets would be retransmitted: this
    /// introduces delays (of 250 milliseconds to several seconds) before
    /// data is sent or received, more often at a higher `percent_loss`
    /// (clamped to 0..=100; zero, the default, disables the simulation).
    pub fn simulate_packet_loss(&mut self, percent_loss: i32) {
        let _ = self.pump();

        self.percent_loss = percent_loss.clamp(0, 100);

        self.update_simulated_failure();
    }
}

// !!! FIXME: docs should note that this will THROW AWAY pending writes in _our_ buffers (not the kernel-level buffers) if you didn't wait for them to finish.
impl Drop for StreamSocket {
    /// Translation of `NET_DestroyStreamSocket()`.
    fn drop(&mut self) {
        let _ = self.pump(); // try one last time to send any last pending data.

        if self.handle != INVALID_SOCKET {
            close_socket_handle(self.handle); // !!! FIXME: what does this do with non-blocking sockets? Release the descriptor but the kernel continues sending queued buffers behind the scenes?
        }
    }
}

/// `NET_DatagramSocketHandle`.
struct DatagramSocketHandle {
    handle: Socket,
    family: c_int,
    protocol: c_int,
    broadcast: Option<Address>,
}

/// A packet waiting in a datagram socket's queue (a `NET_Datagram` whose
/// address may be NULL, for broadcasts).
struct PendingDatagram {
    addr: Option<Address>,
    port: u16,
    buf: Vec<u8>,
}

/// A packet received with [`DatagramSocket::receive`]. Translation of
/// `NET_Datagram` (dropping it is `NET_DestroyDatagram()`).
#[derive(Clone, Debug)]
pub struct Datagram {
    /// Sender's address. Clone it to keep it past the datagram.
    pub addr: Address,
    /// Sender's port. These do not have to come from the same port the
    /// receiver is bound to. These are in host byte order, don't byteswap them!
    pub port: u16,
    /// The payload of this datagram.
    pub buf: Vec<u8>,
}

const RECV_BUFFER_SIZE: usize = 64 * 1024;
const LATEST_RECV_ADDRS: usize = 64;

/// An unreliable, packet-oriented socket (UDP). Translation of
/// `NET_DatagramSocket`.
///
/// Packets ("datagrams") are sent to any address and port, one at a time,
/// and arrive whenever they get there, maybe out of order, maybe not at
/// all. Dropping the socket is `NET_DestroyDatagramSocket()`: packets
/// still queued for sending are abandoned, and ones that arrived but
/// weren't received are lost.
pub struct DatagramSocket {
    addr: Option<Address>, // bound to this address (NULL for any).
    port: u16,
    percent_loss: i32,
    recv_buffer: Box<[u8]>,
    latest_recv_addrs: [Option<Address>; LATEST_RECV_ADDRS],
    latest_recv_addrs_idx: usize,
    handles: Vec<DatagramSocketHandle>, // for INADDR_ANY things, one handle (etc) per network family.
    /// `pending_output`, `pending_output_len`, `pending_output_allocation`.
    pending_output: Vec<PendingDatagram>,
    allow_broadcast: bool,
    _winsock: WinsockRef,
}

impl std::fmt::Debug for DatagramSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatagramSocket")
            .field("addr", &self.addr)
            .field("port", &self.port)
            .field("percent_loss", &self.percent_loss)
            .field("num_handles", &self.handles.len())
            .field("pending_output_len", &self.pending_output.len())
            .field("allow_broadcast", &self.allow_broadcast)
            .finish()
    }
}

fn find_broadcast_address(ainfo: &AddrInfo, interface_index: &mut u32) -> Option<Address> {
    let mut retval = None;

    // SAFETY: `ai_addr` holds `ai_addrlen` bytes.
    let iface =
        unsafe { create_sdl_net_addr_from_sock_addr(ainfo.ai_addr, ainfo.ai_addrlen as SockLen) }
            .ok()?;

    if !interfaces_ready() {
        return None;
    }

    {
        let interfaces = read_interfaces();

        let ni = interfaces.iter().find(|i| {
            compare_addresses(Some(&iface), Some(&i.address)) == std::cmp::Ordering::Equal
        });

        match ni {
            None => {
                // SDL_SetError("Not a network interface address");
                // (The caller replaces this error with its own.)
            }
            Some(ni) => {
                *interface_index = ni.index;
                if let Some(broadcast) = &ni.broadcast {
                    retval = Some(broadcast.clone()); // we calculated the broadcast address when discovering this interface. It's probably IPv4. We're good to go.
                } else if ainfo.ai_family == AF_INET6 {
                    // we fake this for IPv6 on the all-nodes link-local multicast group.
                    retval = ipv6_broadcast_addr();
                } else {
                    // SDL_SetError("Can't determine broadcast address for this interface");
                }
            }
        }
    }

    if retval.is_none() && (ainfo.ai_family == AF_INET6) {
        // we fake this for IPv6 on the all-nodes link-local multicast group.
        retval = ipv6_broadcast_addr();
    }

    retval
}

impl DatagramSocket {
    /// Create and bind a new datagram socket. Translation of
    /// `NET_CreateDatagramSocket()`.
    ///
    /// `addr` is the local address to bind to, or `None` for all of them;
    /// `port` is where packets for this socket arrive (a server should pick
    /// a well-known one; zero lets the system pick an unused port). This
    /// doesn't block.
    ///
    /// Properties: [`PROP_DATAGRAM_SOCKET_REUSEADDR_BOOLEAN`] (default
    /// `true`) and [`PROP_DATAGRAM_SOCKET_ALLOW_BROADCAST_BOOLEAN`] (default
    /// `false`; `SO_BROADCAST` for IPv4, joining the all-nodes link-local
    /// multicast group ff02::1 for IPv6).
    pub fn create(
        addr: Option<&Address>,
        port: u16,
        props: Option<&Properties>,
    ) -> Result<DatagramSocket> {
        if let Some(addr) = addr {
            if !addr.is_resolved() {
                return Err(Error::new("Address is not resolved")); // strictly speaking, this should be a local interface, but a resolved address can fail later.
            }
        }

        let addrwithport = make_addr_info_with_port(addr, SOCK_DGRAM, port)?;

        let allow_broadcast =
            get_boolean_property(props, PROP_DATAGRAM_SOCKET_ALLOW_BROADCAST_BOOLEAN, false);
        let mut sock = DatagramSocket {
            addr: addr.cloned(),
            port,
            percent_loss: 0,
            recv_buffer: vec![0u8; RECV_BUFFER_SIZE].into_boxed_slice(),
            latest_recv_addrs: [const { None }; LATEST_RECV_ADDRS],
            latest_recv_addrs_idx: 0,
            handles: Vec::new(),
            pending_output: Vec::new(),
            allow_broadcast,
            _winsock: winsock_ref(),
        };

        let reuseaddr: c_int =
            if get_boolean_property(props, PROP_DATAGRAM_SOCKET_REUSEADDR_BOOLEAN, true) {
                1
            } else {
                0
            };

        let bcast: c_int = if sock.allow_broadcast { 1 } else { 0 };

        let num_handles = if addr.is_some() {
            1
        } else {
            // bind to all interfaces.
            addrwithport.iter().count()
        };

        /* #if 0
        for (struct addrinfo *i = addrwithport; i != NULL; i = i->ai_next) {
            SDL_Log("addr:"); (ai_flags, ai_family, ai_socktype, ai_protocol, ai_canonname)
        }
        #endif */

        // Make sockets for all desired interfaces; if addr!=NULL, this is one socket on one interface,
        //  but if addr==NULL, it might be multiple sockets for IPv4, IPv6, etc, bound to their INADDR_ANY equivalent.
        // (On failure, dropping `sock` closes the handles made so far.)
        for ainfo in addrwithport.iter().take(num_handles) {
            // SAFETY: plain socket creation.
            let handle =
                unsafe { sys::socket(ainfo.ai_family, ainfo.ai_socktype, ainfo.ai_protocol) };

            if handle == INVALID_SOCKET {
                return Err(set_last_socket_error("Failed to create socket"));
            }

            sock.handles.push(DatagramSocketHandle {
                handle,
                family: ainfo.ai_family,
                protocol: ainfo.ai_protocol,
                broadcast: None,
            });

            if make_socket_nonblocking(handle) < 0 {
                return Err(Error::new("Failed to make new socket non-blocking"));
            }

            // SAFETY: int options.
            unsafe {
                sys::setsockopt(
                    handle,
                    sys::SOL_SOCKET,
                    sys::SO_REUSEADDR,
                    (&reuseaddr as *const c_int).cast(),
                    std::mem::size_of::<c_int>() as SockLen,
                );
                sys::setsockopt(
                    handle,
                    sys::SOL_SOCKET,
                    sys::SO_BROADCAST,
                    (&bcast as *const c_int).cast(),
                    std::mem::size_of::<c_int>() as SockLen,
                );
            }

            if ainfo.ai_family == AF_INET6 {
                let one: c_int = 1;
                // SAFETY: an int option.
                unsafe {
                    sys::setsockopt(
                        handle,
                        sys::IPPROTO_IPV6,
                        sys::IPV6_V6ONLY,
                        (&one as *const c_int).cast(),
                        std::mem::size_of::<c_int>() as SockLen,
                    )
                }; // if this fails, oh well.
            }

            // SAFETY: `ai_addr` holds `ai_addrlen` bytes.
            let rc = unsafe { sys::bind(handle, ainfo.ai_addr, ainfo.ai_addrlen as SockLen) };
            if rc == SOCKET_ERROR {
                let err = last_socket_error();
                debug_assert!(!would_block(err)); // binding shouldn't be a blocking operation.
                return Err(set_socket_error("Failed to bind socket", err));
            }

            if sock.allow_broadcast {
                let mut interface_index: u32 = 0; // this will stay zero for INADDR6_ANY, to pick a default interface.
                let broadcast = find_broadcast_address(ainfo, &mut interface_index);
                sock.handles.last_mut().unwrap().broadcast = broadcast.clone();
                // if failed but addr==NULL, this is IPv4 and we can't find an interface for INADDR_ANY. We'll broadcast to all interfaces when sending (because 255.255.255.255 doesn't work on Windows).
                // alternately, we're IPv6 and ipv6_broadcast_addr failed to init for some reason. Just panic and fail here if that happens, I guess.
                if broadcast.is_none() && (addr.is_some() || (ainfo.ai_family == AF_INET6)) {
                    return Err(Error::new(
                        "Failed to determine broadcast address for this interface",
                    ));
                } else if ainfo.ai_family == AF_INET6 {
                    // fake broadcast support via multicasting to all-nodes link-local group, ff02::1.
                    let broadcast = broadcast.expect("checked above");
                    debug_assert!(
                        ipv6_broadcast_addr().is_some_and(|b| Arc::ptr_eq(&b.0, &broadcast.0))
                    );
                    let bai = broadcast.0.ainfo.get().expect("a resolved address");
                    debug_assert!(!bai.addr.is_empty());
                    debug_assert_eq!(bai.family, AF_INET6);
                    // SAFETY: an all-zero ipv6_mreq is valid.
                    let mut mreq6: sys::Ipv6Mreq = unsafe { std::mem::zeroed() };
                    // SAFETY: the address is a sockaddr_in6.
                    let sin6: sys::SockAddrIn6 =
                        unsafe { std::ptr::read_unaligned(bai.sockaddr().cast()) };
                    mreq6.ipv6mr_multiaddr = sin6.sin6_addr;
                    mreq6.ipv6mr_interface = interface_index as _;
                    //SDL_Log("Add membership for %s, index=%d", socket_handle->broadcast->human_readable, (int) interface_index);
                    // SAFETY: an ipv6_mreq option.
                    if unsafe {
                        sys::setsockopt(
                            handle,
                            sys::IPPROTO_IPV6,
                            sys::IPV6_JOIN_GROUP,
                            (&mreq6 as *const sys::Ipv6Mreq).cast(),
                            std::mem::size_of::<sys::Ipv6Mreq>() as SockLen,
                        )
                    } < 0
                    {
                        return Err(Error::new(
                            "Failed to join all-nodes link-local multicast group for broadcasting",
                        ));
                    }
                    let ifidx = interface_index as c_int;
                    // SAFETY: an int option.
                    unsafe {
                        sys::setsockopt(
                            handle,
                            sys::IPPROTO_IPV6,
                            sys::IPV6_MULTICAST_IF,
                            (&ifidx as *const c_int).cast(),
                            std::mem::size_of::<c_int>() as SockLen,
                        )
                    }; // multicast sends go through the same interface.
                }
            }
        }

        Ok(sock)
    }

    /// `sendto()` on one of our handles, with WinSock's error, or 0.
    fn send_to(handle: Socket, buf: &[u8], addrwithport: &AddrInfo) -> (isize, c_int) {
        // SAFETY: `buf` is valid for its length; `ai_addr` holds `ai_addrlen` bytes.
        let rc = unsafe {
            sys::sendto(
                handle,
                buf.as_ptr(),
                buf.len(),
                0,
                addrwithport.ai_addr,
                addrwithport.ai_addrlen as SockLen,
            )
        };
        let err = if rc == SOCKET_ERROR as isize {
            last_socket_error()
        } else {
            0
        };
        (rc, err)
    }

    fn send_one_datagram(&self, addr: Option<&Address>, port: u16, buf: &[u8]) -> Result<Status> {
        if let Some(addr) = addr {
            // unicast to a specific address.
            let addrwithport = make_addr_info_with_port(Some(addr), SOCK_DGRAM, port)?;
            let ai = addrwithport.first();

            let family = ai.ai_family;
            let protocol = ai.ai_protocol;
            for handle in &self.handles {
                if (handle.family == family) && (handle.protocol == protocol) {
                    // !!! FIXME: strictly speaking, this _probably_ just needs to check `family`, right?
                    let (rc, err) = Self::send_to(handle.handle, buf, ai);
                    drop(addrwithport);
                    if err != 0 {
                        return if would_block(err) {
                            Ok(Status::Waiting)
                        } else {
                            Err(set_socket_error("Failed to send from socket", err))
                        };
                    }
                    debug_assert_eq!(rc as usize, buf.len());
                    return Ok(Status::Success); // !!! FIXME: should we sent to _all_ interfaces in this family?
                }
            }
            return Err(Error::new(
                "Unsupported network family in destination address",
            ));
        }

        // broadcast (or fake with multicast) this packet.
        debug_assert!(self.allow_broadcast); // we should have checked this in NET_SendDatagram!

        // FIXME (upstream): `all_wouldblock` starts false and is only ever
        // set to false, so a broadcast that would block everywhere reports
        // NET_FAILURE (with no error message set) instead of NET_WAITING.
        // That is kept; the failure gets the message of the last real
        // error, or a generic one.
        let mut all_wouldblock = false;
        let mut last_error: Option<Error> = None;
        let mut retval = false; // (NET_FAILURE; true is NET_SUCCESS)
        for handle in &self.handles {
            if let Some(broadcast) = &handle.broadcast {
                let Ok(addrwithport) = make_addr_info_with_port(Some(broadcast), SOCK_DGRAM, port)
                else {
                    continue; // oh well, lost UDP packet, I guess.
                };

                //SDL_Log("Broadcasting on %s ...", handle->broadcast->human_readable);
                let (_, err) = Self::send_to(handle.handle, buf, addrwithport.first());
                drop(addrwithport);
                if err == 0 {
                    retval = true; // it went to at least one interface's broadcast address, we'll call it success.
                } else if !would_block(err) {
                    all_wouldblock = false;
                    last_error = Some(set_socket_error("Failed to send from socket", err));
                    // so there's a clear error message, but keep going, maybe something else works out.
                }
            } else if addr.is_none() && interfaces_ready() {
                // iterate all interfaces for this broadcast.
                let interfaces = read_interfaces();
                for iface in interfaces.iter() {
                    let Some(bc) = &iface.broadcast else {
                        continue;
                    };
                    let ainfo = bc.0.ainfo.get();
                    debug_assert!(ainfo.is_some());
                    if ainfo.map(|a| a.family) != Some(handle.family) {
                        continue;
                    }

                    let Ok(addrwithport) = make_addr_info_with_port(Some(bc), SOCK_DGRAM, port)
                    else {
                        continue; // oh well, lost UDP packet, I guess.
                    };

                    //SDL_Log("Broadcasting on %s ...", bc->human_readable);
                    let (_, err) = Self::send_to(handle.handle, buf, addrwithport.first());
                    drop(addrwithport);
                    if err == 0 {
                        retval = true; // it went to at least one interface's broadcast address, we'll call it success.
                    } else if !would_block(err) {
                        all_wouldblock = false;
                        last_error = Some(set_socket_error("Failed to send from socket", err));
                        // so there's a clear error message, but keep going, maybe something else works out.
                    }
                }
            }
        }

        if retval {
            Ok(Status::Success)
        } else if all_wouldblock {
            Ok(Status::Waiting)
        } else {
            Err(last_error.unwrap_or_else(|| Error::new("Failed to broadcast datagram")))
        }
    }

    // see if any pending data can finally be sent, etc
    fn pump(&mut self) -> Result<()> {
        while !self.pending_output.is_empty() {
            let dgram = &self.pending_output[0];
            let rc = self.send_one_datagram(dgram.addr.as_ref(), dgram.port, &dgram.buf)?;
            if rc == Status::Waiting {
                break; // stop trying to send packets for now.
            }

            // else if (rc == NET_SUCCESS)
            self.pending_output.remove(0); // (NET_DestroyDatagram() and the memmove)
        }

        Ok(())
    }

    /// Send a packet to `addr` and `port` (a normal integer, in host byte
    /// order), or queue it for later if it can't be sent at once (this
    /// never blocks). Translation of `NET_SendDatagram()`.
    ///
    /// A `None` address broadcasts the packet (more or less) to every
    /// machine on the LAN, which needs a socket created with
    /// [`PROP_DATAGRAM_SOCKET_ALLOW_BROADCAST_BOOLEAN`]; IPv6 fakes this
    /// with the all-nodes link-local multicast group ff02::1. An error
    /// means the socket is no longer usable.
    pub fn send(&mut self, addr: Option<&Address>, port: u16, buf: &[u8]) -> Result<()> {
        self.pump()?; // try to flush any queued data to the socket now, before we handle more.
        if addr.is_none() && !self.allow_broadcast {
            return Err(Error::new(
                "Datagram socket was not created with broadcast support",
            ));
        } else if buf.len() > (64 * 1024) {
            return Err(Error::new(
                "buffer is too large to send in a single datagram packet",
            ));
        } else if buf.is_empty() {
            return Ok(()); // nothing to do.  (!!! FIXME: but strictly speaking, a UDP packet with no payload is legal.)
        } else if should_simulate_loss(self.percent_loss) {
            return Ok(()); // you won the percent_loss lottery. Drop this packet as if you sent it and it never arrived.
        }

        if self.pending_output.is_empty() {
            // nothing queued? See if we can just send this without queueing.
            let rc = self.send_one_datagram(addr, port, buf)?; // error string was already set in SendOneDatagram.
            if rc == Status::Success {
                return Ok(()); // successfully sent.
            }
            // if rc==NET_WAITING, it wasn't sent, because we would have blocked. Queue it for later, below.
        }

        // queue this up for sending later.
        let min_alloc = self.pending_output.len() + 1;
        if min_alloc > self.pending_output.capacity() {
            let mut newlen = self.pending_output.capacity().max(1);
            while newlen < min_alloc {
                newlen *= 2;
                if newlen > i32::MAX as usize {
                    // uhoh, overflowed! That's a lot of memory!!
                    return Err(Error::out_of_memory());
                }
            }
            self.pending_output
                .try_reserve_exact(newlen - self.pending_output.len())
                .map_err(|_| Error::out_of_memory())?;
        }

        self.pending_output.push(PendingDatagram {
            addr: addr.cloned(),
            port,
            buf: buf.to_vec(),
        });

        Ok(())
    }

    /// The next packet that arrived, or `None` if none did (this never
    /// blocks). Translation of `NET_ReceiveDatagram()`.
    ///
    /// The datagram says who sent it, which is the only way to know whom to
    /// reply to. An error means the socket is no longer usable.
    pub fn receive(&mut self) -> Result<Option<Datagram>> {
        self.pump()?; // try to flush any queued data to the socket now, before we go further.

        for i in 0..self.handles.len() {
            // SAFETY: an all-zero sockaddr_storage is valid.
            let mut from: AddressStorage = unsafe { std::mem::zeroed() };
            let mut fromlen = std::mem::size_of::<AddressStorage>() as SockLen;
            // WinSock's recvfrom wants a `char *` buffer instead of `void *`. The cast here is harmless on BSD Sockets.
            // SAFETY: the buffer is valid for its length; `from` holds `fromlen` bytes.
            let br = unsafe {
                sys::recvfrom(
                    self.handles[i].handle,
                    self.recv_buffer.as_mut_ptr(),
                    self.recv_buffer.len(),
                    0,
                    (&mut from as *mut AddressStorage).cast(),
                    &mut fromlen,
                )
            };
            if br == SOCKET_ERROR as isize {
                let err = last_socket_error();
                if would_block(err) {
                    continue;
                }
                return Err(set_socket_error("Failed to receive datagrams", err));
            } else if should_simulate_loss(self.percent_loss) {
                // you won the percent_loss lottery. Drop this packet as if it never arrived.
                continue;
            }
            let br = br as usize;

            let mut hostbuf = [0 as c_char; 128];
            let mut portbuf = [0 as c_char; 16];
            // SAFETY: `from` holds `fromlen` bytes; the buffers are valid for their lengths.
            let rc = unsafe {
                sys::getnameinfo(
                    (&from as *const AddressStorage).cast(),
                    fromlen,
                    hostbuf.as_mut_ptr(),
                    hostbuf.len(),
                    portbuf.as_mut_ptr(),
                    portbuf.len(),
                    NI_NUMERICHOST | NI_NUMERICSERV,
                )
            };
            if rc != 0 {
                return Err(set_get_addr_info_error(
                    "Failed to determine incoming packet's address",
                    rc,
                ));
            }
            let hostbuf = buf_to_string(&hostbuf);

            // Cache the last X addresses we saw; if we see it again, refcount it and reuse it.
            let mut fromaddr: Option<Address> = None;
            for j in (0..self.latest_recv_addrs_idx).rev() {
                let a = self.latest_recv_addrs[j].as_ref();
                debug_assert!(a.is_some()); // can't be NULL, we either set this before or wrapped around to set again, but it can't be NULL.
                if let Some(a) = a {
                    if a.human_readable() == Some(hostbuf.as_str()) {
                        fromaddr = Some(a.clone());
                        break;
                    }
                }
            }

            if fromaddr.is_none() {
                let idx = self.latest_recv_addrs_idx;
                for j in (idx..LATEST_RECV_ADDRS).rev() {
                    let Some(a) = self.latest_recv_addrs[j].as_ref() else {
                        break; // ran out of already-seen entries.
                    };
                    if a.human_readable() == Some(hostbuf.as_str()) {
                        fromaddr = Some(a.clone());
                        break;
                    }
                }
            }

            let create_fromaddr = fromaddr.is_none();
            let fromaddr = match fromaddr {
                Some(fromaddr) => fromaddr,
                // SAFETY: `from` holds `fromlen` bytes.
                None => unsafe {
                    create_sdl_net_addr_from_sock_addr(
                        (&from as *const AddressStorage).cast(),
                        fromlen,
                    )?
                }, // already set the error string.
            };

            let dg = Datagram {
                addr: fromaddr.clone(),
                port: atoi(&buf_to_string(&portbuf)) as u16,
                buf: self.recv_buffer[..br].to_vec(),
            };

            if create_fromaddr {
                // keep track of the last X addresses we saw.
                let idx = self.latest_recv_addrs_idx;
                self.latest_recv_addrs[idx] = Some(fromaddr); // (unrefs the oldest; okay if "oldest" address slot is still NULL.)
                self.latest_recv_addrs_idx = (idx + 1) % LATEST_RECV_ADDRS;
            }

            return Ok(Some(dg)); // we got one!
        }

        Ok(None) // nothing new.
    }

    /// Enable simulated datagram failures: `percent_loss` (clamped to
    /// 0..=100) of packets, incoming and outgoing, are randomly lost. Zero,
    /// the default, disables the simulation. Translation of
    /// `NET_SimulateDatagramPacketLoss()`.
    pub fn simulate_packet_loss(&mut self, percent_loss: i32) {
        let _ = self.pump();
        self.percent_loss = percent_loss.clamp(0, 100);
    }
}

impl Drop for DatagramSocket {
    /// Translation of `NET_DestroyDatagramSocket()`.
    fn drop(&mut self) {
        let _ = self.pump(); // try one last time to send any last pending data.

        for handle in &self.handles {
            close_socket_handle(handle.handle); // !!! FIXME: what does this do with non-blocking sockets? Release the descriptor but the kernel continues sending queued buffers behind the scenes?
        }
        // (The broadcast addresses, the recently seen addresses, the
        // pending packets and the socket's address are released as the
        // fields drop.)
    }
}

/// Something [`wait_until_input_available`] can wait on. Translation of
/// `NET_GenericSocket` (the `void *` entries of `vsockets`).
#[derive(Debug)]
pub enum GenericSocket<'a> {
    /// A [`StreamSocket`]: new data to read, or (for a client) its
    /// connection completed (or failed).
    Stream(&'a mut StreamSocket),
    /// A [`DatagramSocket`]: a new packet to receive.
    Datagram(&'a mut DatagramSocket),
    /// A [`Server`]: a new connection to accept.
    Server(&'a mut Server),
}

impl GenericSocket<'_> {
    fn socktype(&self) -> SocketType {
        match self {
            GenericSocket::Stream(_) => SocketType::Stream,
            GenericSocket::Datagram(_) => SocketType::Datagram,
            GenericSocket::Server(_) => SocketType::Server,
        }
    }
}

impl<'a> From<&'a mut StreamSocket> for GenericSocket<'a> {
    fn from(sock: &'a mut StreamSocket) -> Self {
        GenericSocket::Stream(sock)
    }
}

impl<'a> From<&'a mut DatagramSocket> for GenericSocket<'a> {
    fn from(sock: &'a mut DatagramSocket) -> Self {
        GenericSocket::Datagram(sock)
    }
}

impl<'a> From<&'a mut Server> for GenericSocket<'a> {
    fn from(server: &'a mut Server) -> Self {
        GenericSocket::Server(server)
    }
}

/// Block until at least one of `sockets` has new input, or `timeout`
/// milliseconds pass (-1 waits indefinitely, 0 checks once without
/// waiting). Translation of `NET_WaitUntilInputAvailable()`.
///
/// Returns how many of the sockets have new input (zero on timeout), but
/// not which: access is non-blocking, so just try each of them. Queued
/// output is sent while waiting, and a client socket's connection
/// completing (or failing) counts as input.
pub fn wait_until_input_available(
    sockets: &mut [GenericSocket<'_>],
    timeout: i32,
) -> Result<usize> {
    let mut timeoutms = timeout;
    if sockets.is_empty() {
        return Ok(0);
    }

    let mut numhandles = 0;
    for sock in sockets.iter() {
        match sock {
            GenericSocket::Stream(_) => numhandles += 1,
            GenericSocket::Datagram(d) => numhandles += d.handles.len(),
            GenericSocket::Server(s) => numhandles += s.handles.len(),
        }
    }

    // (`stack_pfds`, or `malloced_pfds` if there's a _ton_ of these.)
    let mut pfds: Vec<pollfd> = Vec::with_capacity(numhandles);

    let mut retval = 0;
    let endtime = if timeoutms > 0 {
        get_ticks() + timeoutms as u64
    } else {
        0
    };

    loop {
        pfds.clear();

        for sock in sockets.iter() {
            match sock {
                GenericSocket::Stream(stream) => {
                    let events = if stream.status == NET_WAITING {
                        POLLOUT // marked as writable when connection is complete.
                    } else if !stream.pending_output.is_empty() {
                        POLLIN | POLLOUT // poll for input or when we can write more of the pending buffer.
                    } else {
                        POLLIN // poll for input or when we can write more of the pending buffer.
                    };
                    pfds.push(pollfd {
                        fd: stream.handle,
                        events,
                        revents: 0,
                    });
                }

                GenericSocket::Datagram(dgram) => {
                    for handle in &dgram.handles {
                        let events = if !dgram.pending_output.is_empty() {
                            POLLIN | POLLOUT // poll for input or when we can write more of the pending buffer.
                        } else {
                            POLLIN // poll for input or when we can write more of the pending buffer.
                        };
                        pfds.push(pollfd {
                            fd: handle.handle,
                            events,
                            revents: 0,
                        });
                    }
                }

                GenericSocket::Server(server) => {
                    for &handle in &server.handles {
                        pfds.push(pollfd {
                            fd: handle,
                            events: POLLIN, // poll for new connections.
                            revents: 0,
                        });
                    }
                }
            }
        }

        // (Unlike poll(), WindowsPoll() needs at least one handle.)
        let rc = if pfds.is_empty() {
            0
        } else {
            poll(&mut pfds, timeoutms)
        };

        if rc == SOCKET_ERROR {
            return Err(set_last_socket_error("Socket poll failed"));
        }

        // !!! FIXME: skip this loop if rc == 0.
        let mut pfd = pfds.iter();
        for sock in sockets.iter_mut() {
            let mut count_it = false;

            debug_assert!(matches!(
                sock.socktype(),
                SocketType::Stream | SocketType::Datagram | SocketType::Server
            ));
            match sock {
                GenericSocket::Stream(stream) => {
                    let p = pfd.next().expect("one pollfd per stream");
                    debug_assert!(p.fd == stream.handle);
                    let failed = (p.revents & (POLLERR | POLLHUP | POLLNVAL)) != 0;
                    let writable = (p.revents & POLLOUT) != 0;
                    let readable = (p.revents & POLLIN) != 0;

                    if readable || failed {
                        count_it = true;
                    }

                    if stream.status == NET_WAITING {
                        if failed {
                            let mut err: c_int = 0;
                            let mut errsize = std::mem::size_of::<c_int>() as SockLen;
                            // SAFETY: an int option.
                            unsafe {
                                sys::getsockopt(
                                    p.fd,
                                    sys::SOL_SOCKET,
                                    sys::SO_ERROR,
                                    (&mut err as *mut c_int).cast(),
                                    &mut errsize,
                                )
                            };
                            stream.status = NET_FAILURE;
                            stream.status_error =
                                Some(set_socket_error("Socket failed to connect", err));
                        } else if writable {
                            stream.status = NET_SUCCESS;
                            count_it = true;
                        }
                    } else if writable {
                        let _ = stream.pump();
                    }
                }

                GenericSocket::Datagram(dgram) => {
                    let mut pump_socket = false;
                    for handle in &dgram.handles {
                        let p = pfd.next().expect("one pollfd per handle");
                        debug_assert!(p.fd == handle.handle);
                        let failed = (p.revents & (POLLERR | POLLHUP | POLLNVAL)) != 0;
                        let writable = (p.revents & POLLOUT) != 0;
                        let readable = (p.revents & POLLIN) != 0;

                        if readable || failed {
                            count_it = true;
                        }

                        if writable {
                            pump_socket = true;
                        }
                    }

                    if pump_socket {
                        let _ = dgram.pump();
                    }
                }

                GenericSocket::Server(server) => {
                    for &handle in &server.handles {
                        let p = pfd.next().expect("one pollfd per handle");
                        debug_assert!(p.fd == handle);
                        let failed = (p.revents & (POLLERR | POLLHUP | POLLNVAL)) != 0;
                        let readable = (p.revents & POLLIN) != 0;
                        if readable || failed {
                            count_it = true;
                        }
                    }
                }
            }

            if count_it {
                retval += 1;
            }
        }

        // Note (upstream): `endtime` is zero for an infinite wait (-1) as
        // well as for a no-block poll, so an infinite wait that woke up
        // only to send queued output returns 0 here too; the "else" branch
        // below is never reached with a negative timeout.
        if (retval > 0) || (endtime == 0) {
            break; // something has input available, or we are doing a no-block poll.
        } else if timeoutms > 0 {
            // We must have woken up for a pending write, etc. Figure out remaining wait time.
            let now = get_ticks();
            if now < endtime {
                timeoutms = (endtime - now) as i32;
            } else {
                break; // time has expired, break out.
            }
        } // else timeout is meant to be infinite, but we woke up for a write, etc, so go back to an infinite poll.
    }

    Ok(retval)
}

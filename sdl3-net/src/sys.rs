// Rust translation of the platform includes and typedefs at the top of
// src/SDL_net.c from SDL_net.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The socket API of the platform: BSD sockets through `libc` on Unix,
//! WinSock through `windows-sys` on Windows. Upstream gets these from
//! `<sys/socket.h>` and friends or `<winsock2.h>`; this module gives both
//! the same names and types (`c_int` constants, `Socket`, `SockLen`,
//! `AddressStorage`, and the socket calls with one signature) so the
//! translation in `net.rs` reads as upstream does. Everything here is a
//! declaration or a thin call; no C is compiled.

#![allow(non_camel_case_types, dead_code, clippy::missing_safety_doc)]

#[cfg(unix)]
mod imp {
    pub use libc::{
        addrinfo as AddrInfo, c_char, c_int, c_void, ipv6_mreq as Ipv6Mreq, pollfd,
        sockaddr as SockAddr, sockaddr_in6 as SockAddrIn6, sockaddr_storage as AddressStorage,
        socklen_t as SockLen, AF_INET, AF_INET6, AF_UNSPEC, AI_NUMERICHOST, AI_NUMERICSERV,
        AI_PASSIVE, IPPROTO_IPV6, IPV6_MULTICAST_IF, IPV6_V6ONLY, NI_NUMERICHOST, NI_NUMERICSERV,
        POLLERR, POLLHUP, POLLIN, POLLNVAL, POLLOUT, SOCK_DGRAM, SOCK_STREAM, SOL_SOCKET,
        SO_BROADCAST, SO_ERROR, SO_REUSEADDR,
    };

    /// `IPV6_JOIN_GROUP` (glibc and bionic call it `IPV6_ADD_MEMBERSHIP`).
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub const IPV6_JOIN_GROUP: c_int = libc::IPV6_ADD_MEMBERSHIP;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    pub const IPV6_JOIN_GROUP: c_int = libc::IPV6_JOIN_GROUP;

    /// `typedef int Socket;`
    pub type Socket = c_int;
    /// `#define INVALID_SOCKET -1`
    pub const INVALID_SOCKET: Socket = -1;
    /// `#define SOCKET_ERROR -1`
    pub const SOCKET_ERROR: c_int = -1;
    /// The type of `pollfd::events`.
    pub type PollEvents = libc::c_short;

    pub unsafe fn socket(family: c_int, ty: c_int, protocol: c_int) -> Socket {
        unsafe { libc::socket(family, ty, protocol) }
    }
    pub unsafe fn bind(s: Socket, addr: *const SockAddr, len: SockLen) -> c_int {
        unsafe { libc::bind(s, addr, len) }
    }
    pub unsafe fn connect(s: Socket, addr: *const SockAddr, len: SockLen) -> c_int {
        unsafe { libc::connect(s, addr, len) }
    }
    pub unsafe fn listen(s: Socket, backlog: c_int) -> c_int {
        unsafe { libc::listen(s, backlog) }
    }
    pub unsafe fn accept(s: Socket, addr: *mut SockAddr, len: *mut SockLen) -> Socket {
        unsafe { libc::accept(s, addr, len) }
    }
    pub unsafe fn setsockopt(
        s: Socket,
        level: c_int,
        name: c_int,
        val: *const c_void,
        len: SockLen,
    ) -> c_int {
        unsafe { libc::setsockopt(s, level, name, val, len) }
    }
    pub unsafe fn getsockopt(
        s: Socket,
        level: c_int,
        name: c_int,
        val: *mut c_void,
        len: *mut SockLen,
    ) -> c_int {
        unsafe { libc::getsockopt(s, level, name, val, len) }
    }
    pub unsafe fn sendto(
        s: Socket,
        buf: *const u8,
        len: usize,
        flags: c_int,
        to: *const SockAddr,
        tolen: SockLen,
    ) -> isize {
        unsafe { libc::sendto(s, buf.cast(), len, flags, to, tolen) }
    }
    pub unsafe fn recvfrom(
        s: Socket,
        buf: *mut u8,
        len: usize,
        flags: c_int,
        from: *mut SockAddr,
        fromlen: *mut SockLen,
    ) -> isize {
        unsafe { libc::recvfrom(s, buf.cast(), len, flags, from, fromlen) }
    }
    pub unsafe fn getaddrinfo(
        node: *const c_char,
        service: *const c_char,
        hints: *const AddrInfo,
        res: *mut *mut AddrInfo,
    ) -> c_int {
        unsafe { libc::getaddrinfo(node, service, hints, res) }
    }
    pub unsafe fn freeaddrinfo(ai: *mut AddrInfo) {
        unsafe { libc::freeaddrinfo(ai) }
    }
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn getnameinfo(
        sa: *const SockAddr,
        salen: SockLen,
        host: *mut c_char,
        hostlen: usize,
        serv: *mut c_char,
        servlen: usize,
        flags: c_int,
    ) -> c_int {
        unsafe { libc::getnameinfo(sa, salen, host, hostlen as _, serv, servlen as _, flags) }
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Networking::WinSock as ws;

    pub use core::ffi::{c_char, c_int, c_void};
    pub use ws::{
        closesocket, ioctlsocket, select, send, WSACleanup, WSAGetLastError, WSARecv, WSAStartup,
        FD_SET, FIONBIO, TIMEVAL, WSABUF, WSADATA, WSAEWOULDBLOCK,
    };
    pub use ws::{
        ADDRINFOA as AddrInfo, IPV6_MREQ as Ipv6Mreq, SOCKADDR as SockAddr,
        SOCKADDR_IN6 as SockAddrIn6, SOCKADDR_STORAGE as AddressStorage, SOCKET as Socket,
        WSAPOLLFD as pollfd,
    };

    /// `typedef int SockLen;`
    pub type SockLen = c_int;
    pub const INVALID_SOCKET: Socket = ws::INVALID_SOCKET;
    pub const SOCKET_ERROR: c_int = ws::SOCKET_ERROR;
    /// The type of `pollfd::events`.
    pub type PollEvents = i16;

    pub const AF_INET: c_int = ws::AF_INET as c_int;
    pub const AF_INET6: c_int = ws::AF_INET6 as c_int;
    pub const AF_UNSPEC: c_int = ws::AF_UNSPEC as c_int;
    pub const AI_NUMERICHOST: c_int = ws::AI_NUMERICHOST as c_int;
    pub const AI_NUMERICSERV: c_int = ws::AI_NUMERICSERV as c_int;
    pub const AI_PASSIVE: c_int = ws::AI_PASSIVE as c_int;
    pub const NI_NUMERICHOST: c_int = ws::NI_NUMERICHOST as c_int;
    pub const NI_NUMERICSERV: c_int = ws::NI_NUMERICSERV as c_int;
    pub const SOCK_STREAM: c_int = ws::SOCK_STREAM;
    pub const SOCK_DGRAM: c_int = ws::SOCK_DGRAM;
    pub const SOL_SOCKET: c_int = ws::SOL_SOCKET;
    pub const SO_REUSEADDR: c_int = ws::SO_REUSEADDR;
    pub const SO_BROADCAST: c_int = ws::SO_BROADCAST;
    pub const SO_ERROR: c_int = ws::SO_ERROR;
    pub const IPPROTO_IPV6: c_int = ws::IPPROTO_IPV6;
    pub const IPV6_V6ONLY: c_int = ws::IPV6_V6ONLY;
    pub const IPV6_JOIN_GROUP: c_int = ws::IPV6_JOIN_GROUP;
    pub const IPV6_MULTICAST_IF: c_int = ws::IPV6_MULTICAST_IF;
    pub const POLLIN: PollEvents = ws::POLLIN;
    pub const POLLOUT: PollEvents = ws::POLLOUT;
    pub const POLLERR: PollEvents = ws::POLLERR;
    pub const POLLHUP: PollEvents = ws::POLLHUP;
    pub const POLLNVAL: PollEvents = ws::POLLNVAL;

    // WindowsPoll (in net.rs) hands select() its own variable-length
    // version of `fd_set`: a count, then the sockets. That only works if
    // WinSock's `fd_set` puts the array right after a count padded to a
    // socket's size, as it does on both 32 and 64-bit Windows.
    const _: () = assert!(core::mem::offset_of!(FD_SET, fd_count) == 0);
    const _: () =
        assert!(core::mem::offset_of!(FD_SET, fd_array) == core::mem::size_of::<Socket>());
    const _: () = assert!(core::mem::size_of::<AddressStorage>() == 128);

    pub unsafe fn socket(family: c_int, ty: c_int, protocol: c_int) -> Socket {
        unsafe { ws::socket(family, ty, protocol) }
    }
    pub unsafe fn bind(s: Socket, addr: *const SockAddr, len: SockLen) -> c_int {
        unsafe { ws::bind(s, addr, len) }
    }
    pub unsafe fn connect(s: Socket, addr: *const SockAddr, len: SockLen) -> c_int {
        unsafe { ws::connect(s, addr, len) }
    }
    pub unsafe fn listen(s: Socket, backlog: c_int) -> c_int {
        unsafe { ws::listen(s, backlog) }
    }
    pub unsafe fn accept(s: Socket, addr: *mut SockAddr, len: *mut SockLen) -> Socket {
        unsafe { ws::accept(s, addr, len) }
    }
    pub unsafe fn setsockopt(
        s: Socket,
        level: c_int,
        name: c_int,
        val: *const c_void,
        len: SockLen,
    ) -> c_int {
        unsafe { ws::setsockopt(s, level, name, val.cast(), len) }
    }
    pub unsafe fn getsockopt(
        s: Socket,
        level: c_int,
        name: c_int,
        val: *mut c_void,
        len: *mut SockLen,
    ) -> c_int {
        unsafe { ws::getsockopt(s, level, name, val.cast(), len) }
    }
    pub unsafe fn sendto(
        s: Socket,
        buf: *const u8,
        len: usize,
        flags: c_int,
        to: *const SockAddr,
        tolen: SockLen,
    ) -> isize {
        // WinSock takes an `int` length (upstream casts the `size_t` away).
        unsafe { ws::sendto(s, buf, len as c_int, flags, to, tolen) as isize }
    }
    pub unsafe fn recvfrom(
        s: Socket,
        buf: *mut u8,
        len: usize,
        flags: c_int,
        from: *mut SockAddr,
        fromlen: *mut SockLen,
    ) -> isize {
        unsafe { ws::recvfrom(s, buf, len as c_int, flags, from, fromlen) as isize }
    }
    pub unsafe fn getaddrinfo(
        node: *const c_char,
        service: *const c_char,
        hints: *const AddrInfo,
        res: *mut *mut AddrInfo,
    ) -> c_int {
        unsafe { ws::getaddrinfo(node.cast(), service.cast(), hints, res) }
    }
    pub unsafe fn freeaddrinfo(ai: *mut AddrInfo) {
        unsafe { ws::freeaddrinfo(ai) }
    }
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn getnameinfo(
        sa: *const SockAddr,
        salen: SockLen,
        host: *mut c_char,
        hostlen: usize,
        serv: *mut c_char,
        servlen: usize,
        flags: c_int,
    ) -> c_int {
        unsafe {
            ws::getnameinfo(
                sa,
                salen,
                host.cast(),
                hostlen as u32,
                serv.cast(),
                servlen as u32,
                flags,
            )
        }
    }
}

pub use imp::*;

/// `inet_pton(AF_INET6, src, dst)`: the 16 bytes of an IPv6 address
/// string (`libc` doesn't declare `inet_pton()`; the standard library's
/// parser takes the same textual forms).
pub fn inet_pton6(src: &str) -> Option<[u8; 16]> {
    src.parse::<std::net::Ipv6Addr>().ok().map(|a| a.octets())
}

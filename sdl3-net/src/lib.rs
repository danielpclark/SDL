// Rust translation of include/SDL3_net/SDL_net.h from SDL_net.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! # sdl3-net — SDL_net, translated to Rust
//!
//! The pure-Rust translation of [SDL_net](https://github.com/libsdl-org/SDL_net)
//! 3, "a simple library to help with networking": a thin layer over the
//! system's BSD sockets or WinSock that makes them less complicated to use
//! and handles several unexpected corner cases, so the app doesn't have to.
//! It is a separate crate over [`sdl3`], as SDL_net is a separate library
//! over SDL.
//!
//! Some design philosophies of SDL_net:
//!
//! - Nothing is blocking (but you can explicitly wait on things if you want).
//! - Addressing is abstract so you don't have to worry about specific
//!   networks and their specific protocols.
//! - Simple is better than hard, and not necessarily less powerful either.
//!
//! All apps call [`init`] on startup and [`quit`] on shutdown.
//!
//! The cornerstone of the library is the [`Address`]: what it takes to
//! reach another computer on the network, and what network protocol to use
//! to get there. [`resolve_hostname`] turns a hostname (such as
//! `"libsdl.org"`) or an IP address string into one, doing the DNS queries
//! on a background thread.
//!
//! A client connects to a server with [`StreamSocket::create_client`]; once
//! the connection is established (a non-blocking operation), the
//! [`StreamSocket`] sends and receives data with
//! [`write`](StreamSocket::write) and [`read`](StreamSocket::read). A
//! [`Server`] accepts connections from clients
//! ([`Server::accept_client`]), each one a [`StreamSocket`] on the server's
//! end. These are TCP connections, so they can talk to things that don't
//! use SDL_net at all.
//!
//! [`DatagramSocket`]s send and receive UDP packets
//! ([`send`](DatagramSocket::send), [`receive`](DatagramSocket::receive)):
//! small packets that may arrive in any order, or not at all, between any
//! number of peers.
//!
//! The API is non-blocking: every operation returns immediately, with a
//! way to check later if it completed. The functions that do block until
//! an operation completes, each with a timeout in milliseconds (-1 waits
//! indefinitely, 0 just checks):
//!
//! - [`StreamSocket::wait_until_connected`]
//! - [`wait_until_input_available`]
//! - [`Address::wait_until_resolved`]
//! - [`StreamSocket::wait_until_drained`]
//!
//! Finally, network problems can be simulated, to test the
//! always-less-than-ideal conditions in the real world:
//! [`simulate_address_resolution_loss`],
//! [`StreamSocket::simulate_packet_loss`] and
//! [`DatagramSocket::simulate_packet_loss`].
//!
//! As in the [`sdl3`] crate, the implementation is a line-by-line
//! translation and the API is designed for Rust: sockets are owned values
//! whose `Drop` destroys them (`NET_Destroy*`), addresses are reference
//! counted with `Clone` and `Drop` (`NET_RefAddress`/`NET_UnrefAddress`),
//! errors are [`Result`](sdl3::Result)s (`NET_FAILURE` is the `Err` next to
//! a [`Status`]), and every item names the C symbol it translates. The
//! sockets are the system's, through the `libc` and `windows-sys`
//! declarations; no C is compiled and no C SDL is loaded.
//!
//! ```no_run
//! # fn main() -> sdl3::Result<()> {
//! sdl3_net::init()?;
//! let addr = sdl3_net::resolve_hostname("example.com")?;
//! addr.wait_until_resolved(-1)?;
//! let mut sock = sdl3_net::StreamSocket::create_client(&addr, 80, None)?;
//! sock.wait_until_connected(-1)?;
//! sock.write(b"GET / HTTP/1.0\r\nHost: example.com\r\n\r\n")?;
//! let mut buf = [0u8; 1024];
//! loop {
//!     sdl3_net::wait_until_input_available(&mut [(&mut sock).into()], -1)?;
//!     match sock.read(&mut buf) {
//!         Ok(n) => print!("{}", String::from_utf8_lossy(&buf[..n])),
//!         Err(_) => break, // "End of stream"
//!     }
//! }
//! drop(sock);
//! sdl3_net::quit();
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_op_in_unsafe_fn)]
#![warn(missing_debug_implementations)]
// The socket types differ between platforms (`socklen_t` or `int`, `size_t`
// or `int` lengths), so a cast needed on one is a no-op on another.
#![allow(clippy::unnecessary_cast)]

mod net;
mod sys;

pub use net::{
    compare_addresses, get_local_addresses, init, quit, resolve_hostname,
    simulate_address_resolution_loss, version, wait_until_input_available, Address, Datagram,
    DatagramSocket, GenericSocket, Server, Status, StreamSocket,
    PROP_DATAGRAM_SOCKET_ALLOW_BROADCAST_BOOLEAN, PROP_DATAGRAM_SOCKET_REUSEADDR_BOOLEAN,
    PROP_SERVER_REUSEADDR_BOOLEAN,
};

/// The major version of SDL_net this crate translates.
/// Translation of `SDL_NET_MAJOR_VERSION`.
pub const MAJOR_VERSION: u16 = 3;
/// The minor version. Translation of `SDL_NET_MINOR_VERSION`.
pub const MINOR_VERSION: u16 = 2;
/// The micro (patch) version. Translation of `SDL_NET_MICRO_VERSION`.
pub const MICRO_VERSION: u16 = 0;

/// The SDL_net version this crate translates. Translation of
/// `SDL_NET_VERSION` (and, with [`Version::at_least`](sdl3::Version::at_least),
/// of `SDL_NET_VERSION_ATLEAST()`).
pub const VERSION: sdl3::Version = sdl3::Version::new(MAJOR_VERSION, MINOR_VERSION, MICRO_VERSION);

/// The upstream SDL_net revision this translation was made from.
pub const REVISION: &str = "SDL_net-3.2.0-1a84a2a6b9663572f77e2eb5348d42845bac0053";

#[cfg(test)]
mod tests;

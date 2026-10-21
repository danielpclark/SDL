// Rust translation of src/SDL_net_stub_only.c from SDL_net.
// Copyright (C) 1997-2026 Sam Lantinga <slouken@libsdl.org>
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The library where upstream builds the stub (`SDL_net_stub_only.c`, for
//! Emscripten and DOS, which "either don't have networking at all, or have
//! needs we don't currently meet"): "don't do anything. Sorry!"
//!
//! Initializing, resolving and creating sockets fail with "That operation
//! is not supported"; as none can be created, the address and socket types
//! have no values, and their methods can't be called.

use sdl3::properties::Properties;
use sdl3::{Error, Result};

/// `#define NET_PROP_SERVER_REUSEADDR_BOOLEAN "NET.server.reuseaddr"`
pub const PROP_SERVER_REUSEADDR_BOOLEAN: &str = "NET.server.reuseaddr";
/// `#define NET_PROP_DATAGRAM_SOCKET_REUSEADDR_BOOLEAN "NET.datagram_socket.reuseaddr"`
pub const PROP_DATAGRAM_SOCKET_REUSEADDR_BOOLEAN: &str = "NET.datagram_socket.reuseaddr";
/// `#define NET_PROP_DATAGRAM_SOCKET_ALLOW_BROADCAST_BOOLEAN "NET.datagram_socket.allow_broadcast"`
pub const PROP_DATAGRAM_SOCKET_ALLOW_BROADCAST_BOOLEAN: &str =
    "NET.datagram_socket.allow_broadcast";

/// What the stub's types hold: nothing, as none can be created.
#[derive(Clone, Copy, Debug)]
enum Never {}

/// The version of SDL_net this crate translates. Translation of
/// `NET_Version()` (which reports the linked library's version; this is
/// [`VERSION`](crate::VERSION)).
pub fn version() -> sdl3::Version {
    crate::VERSION
}

/// A tri-state for asynchronous operations, less its failure state.
/// Translation of `NET_Status`.
///
/// Still in progress ([`Status::Waiting`], `NET_WAITING`, 0) or complete
/// and successful ([`Status::Success`], `NET_SUCCESS`, 1); complete and
/// failed (`NET_FAILURE`, -1) is the `Err` of the [`Result`] these come in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Status {
    /// Async operation is still in progress, check again later. (`NET_WAITING`)
    Waiting = 0,
    /// Async operation complete, result was success. (`NET_SUCCESS`)
    Success = 1,
}

/// A computer-readable network address. Translation of `NET_Address`.
///
/// The stub resolves nothing, so there are none.
#[derive(Clone, Debug)]
pub struct Address(Never);

/// Initialize the SDL_net library. Translation of `NET_Init()`: the stub
/// fails.
pub fn init() -> Result<()> {
    Err(Error::unsupported())
}

/// Deinitialize the SDL_net library. Translation of `NET_Quit()`: the stub
/// does nothing.
pub fn quit() {}

/// Resolve a human-readable hostname. Translation of
/// `NET_ResolveHostname()`: the stub fails.
pub fn resolve_hostname(_host: &str) -> Result<Address> {
    Err(Error::unsupported())
}

impl Address {
    /// Block until the address is resolved. Translation of
    /// `NET_WaitUntilResolved()`.
    pub fn wait_until_resolved(&self, _timeout: i32) -> Result<Status> {
        match self.0 {}
    }

    /// Check if the address is resolved, without blocking. Translation of
    /// `NET_GetAddressStatus()`.
    pub fn status(&self) -> Result<Status> {
        match self.0 {}
    }

    /// A human-readable string for the resolved address. Translation of
    /// `NET_GetAddressString()`.
    pub fn address_string(&self) -> Result<&str> {
        match self.0 {}
    }

    /// The protocol-level bytes of the resolved address. Translation of
    /// `NET_GetAddressBytes()`.
    pub fn bytes(&self) -> Result<&[u8]> {
        match self.0 {}
    }
}

/// Compare two addresses (`None` is a NULL address), for sorting.
/// Translation of `NET_CompareAddresses()`: the stub's are all equal.
pub fn compare_addresses(_a: Option<&Address>, _b: Option<&Address>) -> std::cmp::Ordering {
    std::cmp::Ordering::Equal
}

impl PartialEq for Address {
    fn eq(&self, _other: &Address) -> bool {
        match self.0 {}
    }
}

impl Eq for Address {}

impl PartialOrd for Address {
    fn partial_cmp(&self, other: &Address) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Address {
    fn cmp(&self, _other: &Address) -> std::cmp::Ordering {
        match self.0 {}
    }
}

/// Enable simulated address resolution failures. Translation of
/// `NET_SimulateAddressResolutionLoss()`: the stub does nothing.
pub fn simulate_address_resolution_loss(_percent_loss: i32) {}

/// The addresses of the system's network interfaces. Translation of
/// `NET_GetLocalAddresses()`: the stub fails.
pub fn get_local_addresses() -> Result<Vec<Address>> {
    Err(Error::unsupported())
}

/// A reliable, stream-oriented connection to another system (TCP).
/// Translation of `NET_StreamSocket`.
///
/// The stub connects nothing, so there are none.
#[derive(Debug)]
pub struct StreamSocket(Never);

impl StreamSocket {
    /// Begin connecting a socket as a client to a remote server.
    /// Translation of `NET_CreateClient()`: the stub fails.
    pub fn create_client(
        _addr: &Address,
        _port: u16,
        _props: Option<&Properties>,
    ) -> Result<StreamSocket> {
        Err(Error::unsupported())
    }

    /// Block until the socket is connected. Translation of
    /// `NET_WaitUntilConnected()`.
    pub fn wait_until_connected(&mut self, _timeout: i32) -> Result<Status> {
        match self.0 {}
    }

    /// Check if the socket is connected, without blocking. Translation of
    /// `NET_GetConnectionStatus()`.
    pub fn connection_status(&mut self) -> Result<Status> {
        match self.0 {}
    }

    /// The remote address of the socket. Translation of
    /// `NET_GetStreamSocketAddress()`.
    pub fn address(&self) -> Address {
        match self.0 {}
    }

    /// Send bytes over the socket. Translation of
    /// `NET_WriteToStreamSocket()`.
    pub fn write(&mut self, _buf: &[u8]) -> Result<()> {
        match self.0 {}
    }

    /// The number of bytes still queued for transmission. Translation of
    /// `NET_GetStreamSocketPendingWrites()`.
    pub fn pending_writes(&mut self) -> Result<usize> {
        match self.0 {}
    }

    /// Block until all queued data is sent. Translation of
    /// `NET_WaitUntilStreamSocketDrained()`.
    pub fn wait_until_drained(&mut self, _timeout: i32) -> Result<usize> {
        match self.0 {}
    }

    /// Receive bytes the remote system sent. Translation of
    /// `NET_ReadFromStreamSocket()`.
    pub fn read(&mut self, _buf: &mut [u8]) -> Result<usize> {
        match self.0 {}
    }

    /// Enable simulated stream failures. Translation of
    /// `NET_SimulateStreamPacketLoss()`.
    pub fn simulate_packet_loss(&mut self, _percent_loss: i32) {
        match self.0 {}
    }
}

/// The receiving end of stream connections. Translation of `NET_Server`.
///
/// The stub listens for nothing, so there are none.
#[derive(Debug)]
pub struct Server(Never);

impl Server {
    /// Create a server, listening for connections to accept. Translation of
    /// `NET_CreateServer()`: the stub fails.
    pub fn create(
        _addr: Option<&Address>,
        _port: u16,
        _props: Option<&Properties>,
    ) -> Result<Server> {
        Err(Error::unsupported())
    }

    /// A stream socket for the next pending client connection.
    /// Translation of `NET_AcceptClient()`.
    pub fn accept_client(&mut self) -> Result<Option<StreamSocket>> {
        match self.0 {}
    }
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

/// An unreliable, packet-oriented socket (UDP). Translation of
/// `NET_DatagramSocket`.
///
/// The stub binds nothing, so there are none.
#[derive(Debug)]
pub struct DatagramSocket(Never);

impl DatagramSocket {
    /// Create and bind a new datagram socket. Translation of
    /// `NET_CreateDatagramSocket()`: the stub fails.
    pub fn create(
        _addr: Option<&Address>,
        _port: u16,
        _props: Option<&Properties>,
    ) -> Result<DatagramSocket> {
        Err(Error::unsupported())
    }

    /// Send a packet to `addr` and `port`. Translation of
    /// `NET_SendDatagram()`.
    pub fn send(&mut self, _addr: Option<&Address>, _port: u16, _buf: &[u8]) -> Result<()> {
        match self.0 {}
    }

    /// The next packet that arrived. Translation of `NET_ReceiveDatagram()`.
    pub fn receive(&mut self) -> Result<Option<Datagram>> {
        match self.0 {}
    }

    /// Enable simulated datagram failures. Translation of
    /// `NET_SimulateDatagramPacketLoss()`.
    pub fn simulate_packet_loss(&mut self, _percent_loss: i32) {
        match self.0 {}
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

/// Block until at least one of `sockets` has new input. Translation of
/// `NET_WaitUntilInputAvailable()`: the stub fails.
pub fn wait_until_input_available(
    _sockets: &mut [GenericSocket<'_>],
    _timeout: i32,
) -> Result<usize> {
    Err(Error::unsupported())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_is_unsupported() {
        let unsupported = Error::unsupported();
        assert_eq!(version(), crate::VERSION);
        assert_eq!(init().unwrap_err().message(), unsupported.message());
        quit();
        assert!(resolve_hostname("libsdl.org").is_err());
        assert!(get_local_addresses().is_err());
        assert!(Server::create(None, 0, None).is_err());
        assert!(DatagramSocket::create(None, 0, None).is_err());
        assert!(wait_until_input_available(&mut [], 0).is_err());
        assert_eq!(compare_addresses(None, None), std::cmp::Ordering::Equal);
        simulate_address_resolution_loss(50);
    }
}

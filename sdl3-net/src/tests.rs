// Tests for the SDL_net translation, over loopback only (127.0.0.1, and
// ::1 where the system has IPv6): no outside network is needed. Where the
// result is the system's (error strings, address formats), the expected
// values are the ones upstream's C (SDL_net 3.2.0 with SDL3) gives on Linux
// for the same calls; elsewhere only their shape is checked.
//
// The examples upstream ships are translated as tests where they are not
// interactive demos: resolve-hostnames.c, get-local-addrs.c, echo-server.c
// (with a client) and datagram.c (server and client).

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use sdl3::properties::Properties;

use crate::*;

/// The tests share the library's global state (init counts, simulated
/// resolution loss), so they run one at a time.
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Initialized SDL_net (and the test lock) for the life of the guard.
struct Net {
    _lock: MutexGuard<'static, ()>,
}

impl Net {
    fn new() -> Net {
        let lock = TEST_LOCK.lock().unwrap_or_else(|e| {
            TEST_LOCK.clear_poison();
            e.into_inner()
        });
        init().unwrap();
        simulate_address_resolution_loss(0);
        Net { _lock: lock }
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        simulate_address_resolution_loss(0);
        quit();
    }
}

/// Report a skip for a missing `capability`, the way the `sdl3` crate's
/// tests do (`SDL3_TEST_REQUIRE` turns it into a failure).
fn skip(capability: &str, reason: impl std::fmt::Display) {
    let list = std::env::var("SDL3_TEST_REQUIRE").unwrap_or_default();
    let required = list
        .split(',')
        .map(str::trim)
        .any(|n| n == "all" || n == capability);
    if required {
        panic!("required capability {capability} unavailable: {reason}");
    }
    eprintln!("note: skipping, {capability} unavailable: {reason}");
    if let Ok(path) = std::env::var("SDL3_TEST_SKIP_LOG") {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let thread = std::thread::current();
            let _ = writeln!(
                file,
                "{capability}\t{}\t{reason}",
                thread.name().unwrap_or("?")
            );
        }
    }
}

fn resolve(host: &str) -> Result<Address, sdl3::Error> {
    let addr = resolve_hostname(host)?;
    addr.wait_until_resolved(-1)?;
    Ok(addr)
}

fn loopback_v4() -> Address {
    resolve("127.0.0.1").unwrap()
}

/// `::1`, or `None` (after noting the skip) without IPv6.
fn loopback_v6() -> Option<Address> {
    match resolve("::1") {
        Ok(addr) => {
            // The system may resolve it but have no IPv6 to bind to.
            match DatagramSocket::create(Some(&addr), 0, None) {
                Ok(_) => Some(addr),
                Err(e) => {
                    skip("ipv6", format!("can't bind to ::1: {e}"));
                    None
                }
            }
        }
        Err(e) => {
            skip("ipv6", format!("can't resolve ::1: {e}"));
            None
        }
    }
}

fn no_reuseaddr() -> Properties {
    let props = Properties::new();
    props.set(PROP_SERVER_REUSEADDR_BOOLEAN, false).unwrap();
    props
        .set(PROP_DATAGRAM_SOCKET_REUSEADDR_BOOLEAN, false)
        .unwrap();
    props
}

/// Ports for this process's tests: a start that differs between
/// processes, then the next one each time.
fn next_port() -> u16 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let base = 20000 + (std::process::id() % 400) * 25;
    (base + n % 10000) as u16
}

/// A server on `addr` at a free port (no address reuse, so a port that is
/// taken fails and the next one is tried).
fn server_on(addr: &Address) -> (Server, u16) {
    let props = no_reuseaddr();
    let mut last = None;
    for _ in 0..50 {
        let port = next_port();
        match Server::create(Some(addr), port, Some(&props)) {
            Ok(server) => return (server, port),
            Err(e) => last = Some(e),
        }
    }
    panic!("no free port for a server: {last:?}");
}

/// A datagram socket on `addr` at a free port.
fn datagram_on(addr: &Address, props: Option<&Properties>) -> (DatagramSocket, u16) {
    let no_reuse = no_reuseaddr();
    let props = props.unwrap_or(&no_reuse);
    let mut last = None;
    for _ in 0..50 {
        let port = next_port();
        match DatagramSocket::create(Some(addr), port, Some(props)) {
            Ok(sock) => return (sock, port),
            Err(e) => last = Some(e),
        }
    }
    panic!("no free port for a datagram socket: {last:?}");
}

/// A connected client and the server's end of it.
fn connected_pair(addr: &Address) -> (Server, StreamSocket, StreamSocket) {
    let (mut server, port) = server_on(addr);
    let mut client = StreamSocket::create_client(addr, port, None).unwrap();
    assert_eq!(client.wait_until_connected(-1).unwrap(), Status::Success);
    assert_eq!(
        wait_until_input_available(&mut [(&mut server).into()], 5000).unwrap(),
        1
    );
    let accepted = server
        .accept_client()
        .unwrap()
        .expect("a pending connection");
    (server, client, accepted)
}

/// Read exactly `len` bytes, waiting for them (and pumping `pump`'s
/// queued output meanwhile).
fn read_exactly(
    sock: &mut StreamSocket,
    len: usize,
    mut pump: Option<&mut StreamSocket>,
) -> Vec<u8> {
    let mut data = Vec::with_capacity(len);
    let mut buf = vec![0u8; 64 * 1024];
    let deadline = Instant::now() + Duration::from_secs(30);
    while data.len() < len {
        assert!(
            Instant::now() < deadline,
            "timed out after {} bytes",
            data.len()
        );
        match &mut pump {
            Some(other) => {
                wait_until_input_available(&mut [(&mut *sock).into(), (&mut **other).into()], 100)
                    .unwrap();
            }
            None => {
                wait_until_input_available(&mut [(&mut *sock).into()], 100).unwrap();
            }
        }
        let want = (len - data.len()).min(buf.len());
        let n = sock.read(&mut buf[..want]).unwrap();
        data.extend_from_slice(&buf[..n]);
    }
    data
}

fn receive_one(sock: &mut DatagramSocket) -> Datagram {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "no datagram arrived");
        wait_until_input_available(&mut [(&mut *sock).into()], 100).unwrap();
        if let Some(dgram) = sock.receive().unwrap() {
            return dgram;
        }
    }
}

#[test]
fn version_and_revision() {
    assert_eq!(version(), VERSION);
    // NET_Version() in upstream's C
    assert_eq!(version().to_number(), 3002000);
    assert!(VERSION.at_least(3, 2, 0));
    assert!(REVISION.starts_with("SDL_net-3.2.0-"));
}

#[test]
fn init_and_quit_nest() {
    let _net = Net::new();
    init().unwrap();
    init().unwrap();
    quit();
    quit();
    // still initialized once (by `_net`)
    resolve("127.0.0.1").unwrap();
    drop(_net);

    // Not initialized: upstream asserts; here it's an error.
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        resolve_hostname("127.0.0.1").unwrap_err().message(),
        "SDL_net is not initialized"
    );
    quit(); // an extra quit is harmless
    init().unwrap();
    resolve("127.0.0.1").unwrap();
    quit();
}

#[test]
fn resolve_numeric_and_localhost() {
    let _net = Net::new();

    let lo = loopback_v4();
    assert_eq!(lo.status().unwrap(), Status::Success);
    assert_eq!(lo.wait_until_resolved(0).unwrap(), Status::Success);
    assert_eq!(lo.address_string().unwrap(), "127.0.0.1");
    // The system's sockaddr_in (BSDs put a length byte first).
    #[cfg(any(target_os = "linux", target_os = "android", windows))]
    assert_eq!(
        lo.bytes().unwrap(),
        [2, 0, 0, 0, 127, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]
    );

    // whitespace around the name is ignored (SDL_net issue 148)
    let lo_ws = resolve("  127.0.0.1\t\n").unwrap();
    assert_eq!(lo_ws.address_string().unwrap(), "127.0.0.1");
    assert_eq!(
        compare_addresses(Some(&lo), Some(&lo_ws)),
        std::cmp::Ordering::Equal
    );
    assert_eq!(lo, lo_ws);

    let localhost = resolve("localhost").unwrap();
    let s = localhost.address_string().unwrap();
    assert!(s == "127.0.0.1" || s == "::1", "localhost is {s}");

    let two = resolve("127.0.0.2").unwrap();
    assert_eq!(two.address_string().unwrap(), "127.0.0.2");
    assert!(lo < two);
    assert_eq!(
        compare_addresses(Some(&two), Some(&lo)),
        std::cmp::Ordering::Greater
    );
    assert_eq!(compare_addresses(Some(&lo), None), std::cmp::Ordering::Less);
    assert_eq!(
        compare_addresses(None, Some(&lo)),
        std::cmp::Ordering::Greater
    );
    assert_eq!(compare_addresses(None, None), std::cmp::Ordering::Equal);

    // a clone is a reference to the same address
    let lo2 = lo.clone();
    drop(lo);
    assert_eq!(lo2.address_string().unwrap(), "127.0.0.1");

    // many at once (more than MIN_RESOLVER_THREADS, so more threads spin up)
    let addrs: Vec<Address> = (1..=20)
        .map(|i| resolve_hostname(&format!("127.0.0.{i}")).unwrap())
        .collect();
    for (i, addr) in addrs.iter().enumerate() {
        assert_eq!(addr.wait_until_resolved(-1).unwrap(), Status::Success);
        assert_eq!(addr.address_string().unwrap(), format!("127.0.0.{}", i + 1));
    }
    let mut sorted = addrs.clone();
    sorted.reverse();
    sorted.sort();
    assert_eq!(sorted, addrs);
}

#[test]
fn resolve_ipv6_loopback() {
    let _net = Net::new();
    let Some(v6) = loopback_v6() else {
        return;
    };
    assert_eq!(v6.address_string().unwrap(), "::1");
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // sockaddr_in6: AF_INET6 (10), port, flow info, the address, scope.
        let mut expected = vec![10, 0, 0, 0, 0, 0, 0, 0];
        expected.extend_from_slice(&[0; 15]);
        expected.push(1);
        expected.extend_from_slice(&[0; 4]);
        assert_eq!(v6.bytes().unwrap(), expected);
    }
    let lo = loopback_v4();
    // AF_INET sorts before AF_INET6
    assert!(lo < v6);
}

#[test]
fn resolve_failures() {
    let _net = Net::new();

    // A reserved name (RFC 2606) that can't resolve; whatever DNS says
    // (or doesn't, without a network), it fails.
    let inv = resolve_hostname("nonexistent.invalid").unwrap();
    let err = inv.wait_until_resolved(-1).unwrap_err();
    #[cfg(target_os = "linux")]
    assert!(
        [
            "Name or service not known",
            "Temporary failure in name resolution"
        ]
        .contains(&err.message()),
        "{err}"
    );
    assert!(!err.message().is_empty());
    // The same error, every time it's asked.
    assert_eq!(inv.status().unwrap_err(), err);
    assert_eq!(inv.address_string().unwrap_err(), err);
    assert_eq!(inv.bytes().unwrap_err(), err);

    // An unresolved address can't be used.
    let port = next_port();
    assert_eq!(
        StreamSocket::create_client(&inv, port, None)
            .unwrap_err()
            .message(),
        "Address is not resolved"
    );
    assert_eq!(
        Server::create(Some(&inv), port, None)
            .unwrap_err()
            .message(),
        "Address is not resolved"
    );
    assert_eq!(
        DatagramSocket::create(Some(&inv), port, None)
            .unwrap_err()
            .message(),
        "Address is not resolved"
    );

    // Failed addresses have nothing to compare.
    let lo = loopback_v4();
    assert_eq!(
        compare_addresses(Some(&lo), Some(&inv)),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        compare_addresses(Some(&inv), Some(&lo)),
        std::cmp::Ordering::Greater
    );

    // An empty name: glibc and the BSDs fail, WinSock lists the local
    // computer's addresses.
    let empty = resolve_hostname("").unwrap();
    let result = empty.wait_until_resolved(-1);
    #[cfg(windows)]
    assert_eq!(result.unwrap(), Status::Success);
    #[cfg(not(windows))]
    {
        let err = result.unwrap_err();
        #[cfg(target_os = "linux")]
        assert_eq!(err.message(), "Name or service not known");
        let _ = err;
        assert_eq!(
            compare_addresses(Some(&inv), Some(&empty)),
            std::cmp::Ordering::Equal
        );
    }

    // A name with a NUL ends there, as the C string would.
    let nul = resolve("127.0.0.1\0garbage").unwrap();
    assert_eq!(nul.address_string().unwrap(), "127.0.0.1");
}

#[test]
fn simulated_resolution_loss() {
    let _net = Net::new();
    simulate_address_resolution_loss(1000); // clamped to 100: everything fails, after a delay
    let start = Instant::now();
    let addr = resolve_hostname("127.0.0.1").unwrap();
    // it takes at least 250 milliseconds to fail
    assert_eq!(addr.status().unwrap(), Status::Waiting);
    assert_eq!(
        addr.address_string().unwrap_err().message(),
        "Address not yet resolved"
    );
    assert_eq!(
        addr.bytes().unwrap_err().message(),
        "Address not yet resolved"
    );
    assert_eq!(addr.wait_until_resolved(0).unwrap(), Status::Waiting);
    assert_eq!(addr.wait_until_resolved(50).unwrap(), Status::Waiting);
    assert!(start.elapsed() >= Duration::from_millis(50));
    let err = addr.wait_until_resolved(-1).unwrap_err();
    assert_eq!(err.message(), "simulated failure");
    assert!(start.elapsed() >= Duration::from_millis(250));
    assert_eq!(
        addr.address_string().unwrap_err().message(),
        "simulated failure"
    );

    simulate_address_resolution_loss(-5); // clamped to 0: off
    assert_eq!(
        resolve_hostname("127.0.0.1")
            .unwrap()
            .wait_until_resolved(-1)
            .unwrap(),
        Status::Success
    );
}

fn tcp_exchange(addr: &Address, expected_peer: &str) {
    let (mut server, client_end, server_end) = connected_pair(addr);
    let (mut client, mut accepted) = (client_end, server_end);

    // Accepting a connection that's there; then nothing more.
    assert!(server.accept_client().unwrap().is_none());

    assert_eq!(client.connection_status().unwrap(), Status::Success);
    assert_eq!(accepted.connection_status().unwrap(), Status::Success);
    assert_eq!(client.address(), *addr);
    assert_eq!(client.address().address_string().unwrap(), expected_peer);
    assert_eq!(accepted.address().address_string().unwrap(), expected_peer);

    // Both ways.
    client.write(b"hello").unwrap();
    assert_eq!(client.pending_writes().unwrap(), 0);
    assert_eq!(read_exactly(&mut accepted, 5, None), b"hello");
    let mut buf = [0u8; 16];
    assert_eq!(accepted.read(&mut buf).unwrap(), 0); // nothing more for now
    assert_eq!(accepted.read(&mut []).unwrap(), 0);
    accepted.write(b"world!").unwrap();
    accepted.write(b"").unwrap();
    assert_eq!(read_exactly(&mut client, 6, None), b"world!");

    // Partial writes: more than the kernel buffers hold, so the rest is
    // queued, then sent as the other side reads.
    let big: Vec<u8> = (0..(16usize << 20))
        .map(|i| (i * 7 + i / 251) as u8)
        .collect();
    client.write(&big).unwrap();
    let queued = client.pending_writes().unwrap();
    // (Windows may take it all into its own buffers.)
    #[cfg(unix)]
    assert!(queued > 0, "16 MB went out at once");
    assert!(queued < big.len());
    if queued > 0 {
        // A short wait times out with data still queued.
        let start = Instant::now();
        let left = client.wait_until_drained(50).unwrap();
        assert!(left > 0);
        assert!(start.elapsed() >= Duration::from_millis(40));
        // More queues behind it, in order.
        client.write(b"tail").unwrap();
        assert!(client.pending_writes().unwrap() >= 4);
    } else {
        client.write(b"tail").unwrap();
    }
    let received = read_exactly(&mut accepted, big.len() + 4, Some(&mut client));
    assert!(
        received[..big.len()] == big[..],
        "the stream arrived out of order"
    );
    assert_eq!(&received[big.len()..], b"tail");
    assert_eq!(client.wait_until_drained(-1).unwrap(), 0);
    assert_eq!(client.pending_writes().unwrap(), 0);

    // Disconnects: the other end reads the end of the stream...
    drop(client);
    assert_eq!(
        wait_until_input_available(&mut [(&mut accepted).into()], 5000).unwrap(),
        1
    );
    assert_eq!(
        accepted.read(&mut buf).unwrap_err().message(),
        "End of stream"
    );
    assert_eq!(
        accepted.read(&mut buf).unwrap_err().message(),
        "End of stream"
    );
    drop(accepted);

    // ...and writing to a closed connection fails, sooner or later.
    let (_server2, mut client, accepted) = connected_pair(addr);
    drop(accepted);
    std::thread::sleep(Duration::from_millis(100));
    let chunk = vec![0u8; 64 * 1024];
    let mut result = Ok(());
    for _ in 0..200 {
        result = client.write(&chunk);
        if result.is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let err = result.expect_err("writes to a closed connection kept working");
    assert!(
        err.message().starts_with("Failed to write to socket: "),
        "{err}"
    );
    #[cfg(target_os = "linux")]
    assert!(
        [
            "Failed to write to socket: Broken pipe",
            "Failed to write to socket: Connection reset by peer"
        ]
        .contains(&err.message()),
        "{err}"
    );
}

#[test]
fn tcp_exchange_ipv4() {
    let _net = Net::new();
    tcp_exchange(&loopback_v4(), "127.0.0.1");
}

#[test]
fn tcp_exchange_ipv6() {
    let _net = Net::new();
    if let Some(v6) = loopback_v6() {
        tcp_exchange(&v6, "::1");
    }
}

#[test]
fn wait_functions_and_timeouts() {
    let _net = Net::new();
    let lo = loopback_v4();
    let (mut server, port) = server_on(&lo);

    assert_eq!(wait_until_input_available(&mut [], -1).unwrap(), 0);

    // Nothing pending: no wait, and a timed wait.
    let start = Instant::now();
    assert_eq!(
        wait_until_input_available(&mut [(&mut server).into()], 0).unwrap(),
        0
    );
    assert!(start.elapsed() < Duration::from_millis(1000));
    let start = Instant::now();
    assert_eq!(
        wait_until_input_available(&mut [(&mut server).into()], 100).unwrap(),
        0
    );
    assert!(start.elapsed() >= Duration::from_millis(90));
    assert!(server.accept_client().unwrap().is_none());

    // A client: the server has input, the client's connection completes.
    let mut client = StreamSocket::create_client(&lo, port, None).unwrap();
    let status = client.wait_until_connected(5000).unwrap();
    assert_eq!(status, Status::Success);
    assert_eq!(client.wait_until_connected(0).unwrap(), Status::Success);
    assert_eq!(client.wait_until_drained(0).unwrap(), 0);
    assert_eq!(client.wait_until_drained(-1).unwrap(), 0);

    let (mut dgram, dport) = datagram_on(&lo, None);
    // Several sockets at once: only the server has input.
    let n = wait_until_input_available(
        &mut [
            (&mut client).into(),
            (&mut server).into(),
            (&mut dgram).into(),
        ],
        5000,
    )
    .unwrap();
    assert_eq!(n, 1);
    let mut accepted = server.accept_client().unwrap().unwrap();
    assert_eq!(
        wait_until_input_available(
            &mut [
                (&mut client).into(),
                (&mut server).into(),
                (&mut dgram).into(),
                (&mut accepted).into(),
            ],
            50,
        )
        .unwrap(),
        0
    );

    // Data for two of them.
    accepted.write(b"x").unwrap();
    let (mut sender, _) = datagram_on(&lo, None);
    sender.send(Some(&lo), dport, b"y").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let n = wait_until_input_available(
            &mut [
                (&mut client).into(),
                (&mut server).into(),
                (&mut dgram).into(),
                (&mut accepted).into(),
            ],
            1000,
        )
        .unwrap();
        if n == 2 {
            break;
        }
        assert!(n == 1 && Instant::now() < deadline, "{n} ready");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn refused_connection() {
    let _net = Net::new();
    let lo = loopback_v4();
    // A port nobody listens on: a server's, after it's gone.
    let (server, port) = server_on(&lo);
    drop(server);

    let mut client = StreamSocket::create_client(&lo, port, None).unwrap();
    let err = client.wait_until_connected(-1).unwrap_err();
    assert!(
        err.message().starts_with("Socket failed to connect: "),
        "{err}"
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        err.message(),
        "Socket failed to connect: Connection refused"
    );
    // (Upstream returns NET_FAILURE again and leaves SDL_GetError() alone;
    // this returns the same error again.)
    assert_eq!(client.connection_status().unwrap_err(), err);
    assert_eq!(client.wait_until_connected(100).unwrap_err(), err);

    let mut buf = [0u8; 16];
    let err = client.read(&mut buf).unwrap_err();
    #[cfg(target_os = "linux")]
    assert!(
        err.message() == "End of stream"
            || err.message().starts_with("Failed to read from socket: "),
        "{err}"
    );
    let _ = err;
    let err = client.write(b"hi").unwrap_err();
    assert!(
        err.message().starts_with("Failed to write to socket: "),
        "{err}"
    );
    #[cfg(target_os = "linux")]
    assert_eq!(err.message(), "Failed to write to socket: Broken pipe");
}

#[test]
fn bind_conflicts() {
    let _net = Net::new();
    let lo = loopback_v4();
    let props = no_reuseaddr();

    let (_server, port) = server_on(&lo);
    let err = Server::create(Some(&lo), port, Some(&props)).unwrap_err();
    assert!(
        err.message().starts_with("Failed to bind listen socket: "),
        "{err}"
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        err.message(),
        "Failed to bind listen socket: Address already in use"
    );

    let (_dgram, port) = datagram_on(&lo, None);
    let err = DatagramSocket::create(Some(&lo), port, Some(&props)).unwrap_err();
    assert!(
        err.message().starts_with("Failed to bind socket: "),
        "{err}"
    );
    #[cfg(target_os = "linux")]
    assert_eq!(
        err.message(),
        "Failed to bind socket: Address already in use"
    );
}

fn udp_exchange(addr: &Address, expected_peer: &str) {
    let (mut d1, port1) = datagram_on(addr, None);
    let mut d2 = DatagramSocket::create(Some(addr), 0, None).unwrap();

    d2.send(Some(addr), port1, b"ping").unwrap();
    let dgram = receive_one(&mut d1);
    assert_eq!(dgram.buf, b"ping");
    assert_eq!(dgram.addr.address_string().unwrap(), expected_peer);
    assert_ne!(dgram.port, 0);
    assert!(d1.receive().unwrap().is_none());

    // Reply to the sender, at the port it came from.
    d1.send(Some(&dgram.addr), dgram.port, b"pong").unwrap();
    let reply = receive_one(&mut d2);
    assert_eq!(reply.buf, b"pong");
    assert_eq!(reply.port, port1);
    assert_eq!(reply.addr, *addr);

    // Several, from the same sender (the receiver caches the address).
    for i in 0..100u8 {
        d2.send(Some(addr), port1, &[i; 3]).unwrap();
    }
    let mut seen = Vec::new();
    for _ in 0..100 {
        let dg = receive_one(&mut d1);
        assert_eq!(dg.addr, dgram.addr);
        assert_eq!(dg.port, dgram.port);
        seen.push(dg.buf[0]);
    }
    seen.sort_unstable();
    assert_eq!(seen, (0..100).collect::<Vec<u8>>());

    // Sizes: empty is a no-op, 64K + 1 is refused.
    d2.send(Some(addr), port1, b"").unwrap();
    let huge = vec![0u8; 64 * 1024 + 1];
    assert_eq!(
        d2.send(Some(addr), port1, &huge).unwrap_err().message(),
        "buffer is too large to send in a single datagram packet"
    );
    let big = vec![7u8; 60000];
    d2.send(Some(addr), port1, &big).unwrap();
    assert_eq!(receive_one(&mut d1).buf, big);

    // No broadcasting without permission.
    assert_eq!(
        d2.send(None, port1, b"x").unwrap_err().message(),
        "Datagram socket was not created with broadcast support"
    );
}

#[test]
fn udp_datagrams_ipv4() {
    let _net = Net::new();
    let lo = loopback_v4();
    udp_exchange(&lo, "127.0.0.1");

    // The largest datagram is still too big for UDP over IPv4.
    #[cfg(target_os = "linux")]
    {
        let (_d1, port1) = datagram_on(&lo, None);
        let mut d2 = DatagramSocket::create(Some(&lo), 0, None).unwrap();
        assert_eq!(
            d2.send(Some(&lo), port1, &vec![0u8; 64 * 1024])
                .unwrap_err()
                .message(),
            "Failed to send from socket: Message too long"
        );
    }
}

#[test]
fn udp_datagrams_ipv6() {
    let _net = Net::new();
    let Some(v6) = loopback_v6() else {
        return;
    };
    udp_exchange(&v6, "::1");

    // A socket for one family can't send to the other.
    let lo = loopback_v4();
    let mut d = DatagramSocket::create(Some(&v6), 0, None).unwrap();
    assert_eq!(
        d.send(Some(&lo), 9, b"x").unwrap_err().message(),
        "Unsupported network family in destination address"
    );
}

#[test]
fn udp_simulated_loss() {
    let _net = Net::new();
    let lo = loopback_v4();
    let (mut d1, port1) = datagram_on(&lo, None);
    let mut d2 = DatagramSocket::create(Some(&lo), 0, None).unwrap();

    // Outgoing loss: the packets "go", and never arrive.
    d2.simulate_packet_loss(100);
    for _ in 0..10 {
        d2.send(Some(&lo), port1, b"lost").unwrap();
    }
    d2.simulate_packet_loss(0);
    d2.send(Some(&lo), port1, b"found").unwrap();
    assert_eq!(receive_one(&mut d1).buf, b"found");
    assert_eq!(
        wait_until_input_available(&mut [(&mut d1).into()], 50).unwrap(),
        0
    );

    // Incoming loss: they arrive, and are thrown away.
    d1.simulate_packet_loss(200); // (clamped to 100)
    d2.send(Some(&lo), port1, b"dropped").unwrap();
    assert_eq!(
        wait_until_input_available(&mut [(&mut d1).into()], 5000).unwrap(),
        1
    );
    assert!(d1.receive().unwrap().is_none());
    d1.simulate_packet_loss(0);
    d2.send(Some(&lo), port1, b"kept").unwrap();
    assert_eq!(receive_one(&mut d1).buf, b"kept");

    // Half: some get through, some don't.
    d2.simulate_packet_loss(50);
    for i in 0..400u16 {
        d2.send(Some(&lo), port1, &i.to_le_bytes()).unwrap();
    }
    d2.simulate_packet_loss(0);
    d2.send(Some(&lo), port1, b"end").unwrap();
    let mut got = 0;
    loop {
        let dg = receive_one(&mut d1);
        if dg.buf == b"end" {
            break;
        }
        got += 1;
    }
    assert!(
        (100..=300).contains(&got),
        "{got} of 400 arrived at 50% loss"
    );
}

#[test]
fn stream_simulated_loss() {
    let _net = Net::new();
    let lo = loopback_v4();
    let (_server, mut client, mut accepted) = connected_pair(&lo);

    // At 100%, data is held back (lag, as a reliable stream would see).
    client.simulate_packet_loss(100);
    client.write(b"abc").unwrap();
    assert_eq!(client.pending_writes().unwrap(), 3);
    accepted.write(b"xyz").unwrap();
    std::thread::sleep(Duration::from_millis(20));
    let mut buf = [0u8; 8];
    // it's there, but "delayed"
    assert_eq!(client.read(&mut buf).unwrap(), 0);

    // Turned off, it all goes through.
    client.simulate_packet_loss(0);
    assert_eq!(client.pending_writes().unwrap(), 0);
    assert_eq!(read_exactly(&mut accepted, 3, None), b"abc");
    assert_eq!(read_exactly(&mut client, 3, None), b"xyz");

    // At a lower rate, everything still arrives, in order.
    client.simulate_packet_loss(5);
    let data: Vec<u8> = (0..20000u32).map(|i| (i % 253) as u8).collect();
    for chunk in data.chunks(1000) {
        client.write(chunk).unwrap();
    }
    let received = read_exactly(&mut accepted, data.len(), Some(&mut client));
    assert_eq!(received, data);
}

#[test]
fn datagram_broadcast_setup() {
    let _net = Net::new();
    let lo = loopback_v4();
    let props = Properties::new();
    props
        .set(PROP_DATAGRAM_SOCKET_ALLOW_BROADCAST_BOOLEAN, true)
        .unwrap();
    // Loopback has no broadcast address.
    #[cfg(target_os = "linux")]
    assert_eq!(
        DatagramSocket::create(Some(&lo), 0, Some(&props))
            .unwrap_err()
            .message(),
        "Failed to determine broadcast address for this interface"
    );
    let _ = lo;
    // (Sending broadcasts would leave the machine, so they aren't sent.)
}

#[test]
fn local_addresses() {
    let _net = Net::new();
    let addrs = match get_local_addresses() {
        Ok(addrs) => addrs,
        #[cfg(windows)]
        Err(e) => {
            // (Wine may not monitor interface changes.)
            eprintln!("note: no local addresses: {e}");
            return;
        }
        #[cfg(not(windows))]
        Err(e) => panic!("{e}"),
    };
    for addr in &addrs {
        assert_eq!(addr.status().unwrap(), Status::Success);
        let s = addr.address_string().unwrap();
        assert!(!s.is_empty());
        assert!(!s.contains('%'), "{s}");
    }
    #[cfg(target_os = "linux")]
    assert!(
        addrs
            .iter()
            .any(|a| a.address_string().unwrap() == "127.0.0.1"),
        "{addrs:?}"
    );
    // A second call gives the same list (no changes in between).
    let again = get_local_addresses().unwrap();
    assert_eq!(again, addrs);
}

#[test]
fn bind_to_all_interfaces() {
    let _net = Net::new();
    let lo = loopback_v4();
    let port = loop {
        let port = next_port();
        match Server::create(None, port, Some(&no_reuseaddr())) {
            Ok(mut server) => {
                // reachable through loopback
                let mut client = StreamSocket::create_client(&lo, port, None).unwrap();
                assert_eq!(client.wait_until_connected(5000).unwrap(), Status::Success);
                assert_eq!(
                    wait_until_input_available(&mut [(&mut server).into()], 5000).unwrap(),
                    1
                );
                assert!(server.accept_client().unwrap().is_some());
                break port;
            }
            // Upstream makes a socket for every family getaddrinfo() lists,
            // IPv6 included, and fails if one can't be made.
            Err(e) if e.message().starts_with("Failed to create listen socket: ") => {
                skip("ipv6", format!("no IPv6 sockets: {e}"));
                return;
            }
            Err(_) => continue,
        }
    };

    let mut d = DatagramSocket::create(None, port, Some(&no_reuseaddr())).unwrap();
    let mut sender = DatagramSocket::create(Some(&lo), 0, None).unwrap();
    sender.send(Some(&lo), port, b"any").unwrap();
    assert_eq!(receive_one(&mut d).buf, b"any");
}

#[test]
fn sockets_outlive_quit() {
    let _net = Net::new();
    let lo = loopback_v4();
    let (_server, mut client, mut accepted) = connected_pair(&lo);
    let (mut d1, port1) = datagram_on(&lo, None);
    let mut d2 = DatagramSocket::create(Some(&lo), 0, None).unwrap();

    // Quit with everything still open (upstream wants them destroyed
    // first): they keep working, and close cleanly later.
    quit();
    client.write(b"after quit").unwrap();
    assert_eq!(read_exactly(&mut accepted, 10, None), b"after quit");
    d2.send(Some(&lo), port1, b"udp").unwrap();
    assert_eq!(receive_one(&mut d1).buf, b"udp");
    assert_eq!(lo.address_string().unwrap(), "127.0.0.1");
    drop((client, accepted, d1, d2));
    init().unwrap(); // (for `_net` to quit)
}

// --- upstream's examples ---

/// examples/resolve-hostnames.c, over names that need no network.
#[test]
fn example_resolve_hostnames() {
    let _net = Net::new();
    let argv = [
        "localhost",
        "127.0.0.1",
        " 10.0.0.1 ",
        "nonexistent.invalid",
    ];
    let addrs: Vec<Address> = argv.iter().map(|h| resolve_hostname(h).unwrap()).collect();
    let mut log = Vec::new();
    for (host, addr) in argv.iter().zip(&addrs) {
        let _ = addr.wait_until_resolved(-1);
        match addr.status() {
            Err(e) => log.push(format!("{host}: [FAILED TO RESOLVE: {e}]")),
            Ok(_) => log.push(format!("{host}: {}", addr.address_string().unwrap())),
        }
    }
    assert!(log[0] == "localhost: 127.0.0.1" || log[0] == "localhost: ::1");
    assert_eq!(log[1], "127.0.0.1: 127.0.0.1");
    assert_eq!(log[2], " 10.0.0.1 : 10.0.0.1");
    assert!(log[3].starts_with("nonexistent.invalid: [FAILED TO RESOLVE: "));
}

/// examples/get-local-addrs.c.
#[test]
fn example_get_local_addrs() {
    let _net = Net::new();
    match get_local_addresses() {
        Ok(addrs) => {
            let mut log = vec![format!("We saw {} local addresses:", addrs.len())];
            for addr in &addrs {
                log.push(format!("  - {}", addr.address_string().unwrap()));
            }
            assert_eq!(log.len(), addrs.len() + 1);
        }
        Err(e) => eprintln!("Failed to determine local addresses: {e}"),
    }
}

/// examples/echo-server.c, with clients (one simulating failure) that
/// check what comes back.
#[test]
fn example_echo_server() {
    let _net = Net::new();
    let lo = loopback_v4();
    let (mut server, server_port) = server_on(&lo);

    let clients: Vec<_> = (0..3)
        .map(|n| {
            let lo = lo.clone();
            std::thread::spawn(move || {
                let mut sock = StreamSocket::create_client(&lo, server_port, None).unwrap();
                assert_eq!(sock.wait_until_connected(-1).unwrap(), Status::Success);
                if n == 2 {
                    sock.simulate_packet_loss(10);
                }
                let text: Vec<u8> = format!("client {n} says hello ").repeat(500).into_bytes();
                sock.write(&text).unwrap();
                let echoed = read_exactly(&mut sock, text.len(), None);
                assert_eq!(echoed, text);
                text.len()
            })
        })
        .collect();

    let mut streams: Vec<StreamSocket> = Vec::new();
    let mut echoed = 0;
    let mut dropped = 0;
    let deadline = Instant::now() + Duration::from_secs(60);
    while dropped < clients.len() {
        assert!(Instant::now() < deadline, "the echo server timed out");
        {
            let mut vsockets: Vec<GenericSocket> = vec![(&mut server).into()];
            vsockets.extend(streams.iter_mut().map(GenericSocket::from));
            wait_until_input_available(&mut vsockets, 100).unwrap();
        }
        while let Some(stream) = server.accept_client().unwrap() {
            // new connection!
            assert_eq!(stream.address().address_string().unwrap(), "127.0.0.1");
            streams.push(stream);
        }

        // see if anything has new stuff.
        let mut buffer = [0u8; 1024];
        let mut i = 0;
        while i < streams.len() {
            let mut kill_socket = false;
            match streams[i].read(&mut buffer) {
                Err(_) => kill_socket = true, // uhoh, socket failed!
                Ok(0) => {}
                Ok(br) => {
                    echoed += br;
                    if streams[i].write(&buffer[..br]).is_err() {
                        kill_socket = true;
                    }
                }
            }
            if kill_socket {
                streams.remove(i);
                dropped += 1;
            } else {
                i += 1;
            }
        }
    }

    let sent: usize = clients.into_iter().map(|c| c.join().unwrap()).sum();
    assert_eq!(echoed, sent);
}

/// examples/datagram.c: a server receiving, a client sending 128 zero bytes.
#[test]
fn example_datagram() {
    let _net = Net::new();
    let props = Properties::new();
    props
        .set(PROP_DATAGRAM_SOCKET_ALLOW_BROADCAST_BOOLEAN, true)
        .unwrap();

    // SERVER: Listening on 127.0.0.1 (broadcast needs a real interface, so
    // the example's property is left out on loopback).
    let socket_address = loopback_v4();
    let (mut server, server_port) = datagram_on(&socket_address, None);

    // CLIENT: Resolving server hostname '127.0.0.1' ...
    let server_addr = resolve("127.0.0.1").unwrap();
    let mut client = DatagramSocket::create(Some(&socket_address), 0, None).unwrap();
    let buf = [0u8; 128];
    client.send(Some(&server_addr), server_port, &buf).unwrap();

    let dgram = receive_one(&mut server);
    let log = format!(
        "SERVER: got {}-byte datagram from {}:{}",
        dgram.buf.len(),
        dgram.addr.address_string().unwrap(),
        dgram.port
    );
    assert!(
        log.starts_with("SERVER: got 128-byte datagram from 127.0.0.1:"),
        "{log}"
    );
    drop(props);
}

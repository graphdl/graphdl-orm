#![feature(uefi_std)]  // std::os::uefi::env — nightly, same as the target itself

// engine/os — AREST 0.9.0 on bare UEFI, the STD shape: rust ships std
// for x86_64-unknown-uefi (stdout wired to the firmware's simple-text
// ConOut, which OVMF mirrors to COM1 — the harness reads the banner
// off the serial capture), so the OS is a plain main() and the engine
// core links exactly as any host does. The pre-0.9.0 kernel's no_std
// entry shape isn't needed until we outgrow boot services.
//
// The server target's arc from here: the verb table over virtio-net —
// the engine surface, headless. mini adds a text console; full
// realizes the canon view trees through Slint on the framebuffer.

// Canon at boot: the store the build staged (the real support store on
// the dev layout; an empty-store fallback elsewhere) rides the image.
const STORE: &str = include_str!(concat!(env!("OUT_DIR"), "/store.json"));

fn main() {
    // versions DERIVE (the crate's from Cargo, the engine's from its
    // own verb) — never hard-coded in banners or asserts
    println!("AREST OS {}", env!("CARGO_PKG_VERSION"));
    println!("engine: {}", arest::worker::arest_version());
    println!("target: {}", TARGET);
    let receipt = arest::worker::arest_load(STORE);
    println!("store: {} bytes; load: {}", STORE.len(),
             &receipt[..receipt.len().min(120)]);
    let verbs = arest::worker::arest_call("verbs", "{}");
    println!("verbs: {}", &verbs[..verbs.len().min(120)]);
    // the NATIVE carrier path (system:entity_view resolves to its
    // canon-named prim — one spine pass); the interpretive verbs
    // (query's ast:FetchPop) cost minutes at store scale and never
    // belong on the boot path
    let got = arest::worker::arest_call(
        "get",
        "{\"noun\":\"Contact Submission\",\"id\":\"ef998c6716463931\"}");
    println!("get: {}", &got[..got.len().min(160)]);
    net_probe();
    println!("boot: complete");

    // server: the verb table over the wire — smoltcp rides the SNP
    // handle directly (no OS network layer exists; smoltcp IS it).
    // QEMU's user netdev has a fixed layout: we are 10.0.2.15/24,
    // the gateway is 10.0.2.2, and the harness hostfwds a host port
    // to :80 so the smoke can curl the engine on bare firmware.
    #[cfg(all(feature = "server", not(feature = "mini")))]
    wire::serve();

    // mini: the console IS the verb table — one line in (<verb>
    // [args-json]), one JSON answer out, the same store_call the
    // Worker and the MCP host dispatch through. uefi std wires stdin
    // to the firmware's Simple Text Input, stdout to ConOut.
    #[cfg(feature = "mini")]
    console_loop();

    // server: the engine parks here until the verb table goes over
    // the wire (virtio-net / SNP + smoltcp — the lane's next
    // milestone). std::thread::sleep has no clock on bare firmware;
    // the harness kills QEMU at its cap.
    #[allow(unreachable_code)]
    loop {
        std::hint::spin_loop();
    }
}

// The wire's first rung: find the firmware's Simple Network Protocol
// handles and report link identity — the smoltcp Device rides this
// handle next. std owns the runtime; the uefi crate only needs the
// system-table pointer std already holds (std::os::uefi::env).
fn net_probe() {
    use uefi::proto::network::snp::SimpleNetwork;
    let st = std::os::uefi::env::system_table();
    let ih = std::os::uefi::env::image_handle();
    unsafe {
        uefi::table::set_system_table(st.as_ptr().cast());
        // the open agent for protocol opens — without it the uefi
        // crate's boot functions panic
        uefi::boot::set_image_handle(
            uefi::Handle::from_ptr(ih.as_ptr()).unwrap());
    }
    match uefi::boot::find_handles::<SimpleNetwork>() {
        Ok(handles) => {
            println!("net: {} SNP handle(s)", handles.len());
            for h in handles {
                if let Ok(snp) =
                    uefi::boot::open_protocol_exclusive::<SimpleNetwork>(h)
                {
                    let mode = snp.mode();
                    let mac = &mode.current_address.0[..6];
                    println!(
                        "net: mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} state {:?}",
                        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
                        mode.state
                    );
                }
            }
        }
        Err(e) => println!("net: no SNP ({e:?})"),
    }
}

// The wire: a smoltcp Device over the firmware's SNP handle, one TCP
// socket listening on :80, and the SAME verb table every other host
// serves. GET /{verb}?args=<urlencoded-json> (or POST body as args)
// -> arest_call -> one JSON answer. No OS network layer exists on
// bare firmware; smoltcp is the network layer, SNP is the PHY.
#[cfg(all(feature = "server", not(feature = "mini")))]
mod wire {
    use smoltcp::iface::{Config, Interface, SocketSet};
    use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
    use smoltcp::socket::tcp;
    use smoltcp::time::Instant;
    use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr};
    use uefi::proto::network::snp::{NetworkState, SimpleNetwork};
    use uefi::boot::ScopedProtocol;

    struct SnpDevice {
        snp: ScopedProtocol<SimpleNetwork>,
    }

    struct SnpRx(Vec<u8>);
    struct SnpTx<'a>(&'a SimpleNetwork);

    impl RxToken for SnpRx {
        fn consume<R, F: FnOnce(&mut [u8]) -> R>(mut self, f: F) -> R {
            f(&mut self.0)
        }
    }

    impl<'a> TxToken for SnpTx<'a> {
        fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
            let mut buf = vec![0u8; len];
            let r = f(&mut buf);
            // SNP transmit is async: fire, then reap the recycle
            // pointer so the firmware's queue never fills
            if self.0.transmit(0, &buf, None, None, None).is_ok() {
                let mut spins = 0u32;
                loop {
                    match self.0.get_recycled_transmit_buffer_status() {
                        Ok(Some(_)) => break,
                        Ok(None) if spins < 100_000 => spins += 1,
                        _ => break,
                    }
                }
            }
            r
        }
    }

    impl Device for SnpDevice {
        type RxToken<'t> = SnpRx where Self: 't;
        type TxToken<'t> = SnpTx<'t> where Self: 't;

        fn receive(&mut self, _ts: Instant)
            -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
            let mut buf = vec![0u8; 1600];
            match self.snp.receive(&mut buf, None, None, None, None) {
                Ok(n) => {
                    buf.truncate(n);
                    Some((SnpRx(buf), SnpTx(&self.snp)))
                }
                Err(_) => None,
            }
        }

        fn transmit(&mut self, _ts: Instant) -> Option<Self::TxToken<'_>> {
            Some(SnpTx(&self.snp))
        }

        fn capabilities(&self) -> DeviceCapabilities {
            let mut caps = DeviceCapabilities::default();
            caps.medium = Medium::Ethernet;
            caps.max_transmission_unit = 1514;
            caps
        }
    }

    pub fn serve() -> ! {
        let handles = uefi::boot::find_handles::<SimpleNetwork>()
            .expect("SNP handles");
        // the raw SNP handle is the one whose state can start; MNP
        // children refuse — probe each until one initializes
        let mut dev = None;
        for h in handles {
            if let Ok(snp) =
                uefi::boot::open_protocol_exclusive::<SimpleNetwork>(h)
            {
                if snp.mode().state == NetworkState::STOPPED
                    && snp.start().is_err()
                {
                    continue;
                }
                if snp.mode().state == NetworkState::STARTED
                    && snp.initialize(0, 0).is_err()
                {
                    continue;
                }
                if snp.mode().state == NetworkState::INITIALIZED {
                    dev = Some(SnpDevice { snp });
                    break;
                }
            }
        }
        let mut dev = match dev {
            Some(d) => d,
            None => {
                println!("wire: no initializable SNP; parking");
                loop {
                    std::hint::spin_loop();
                }
            }
        };
        let mac = dev.snp.mode().current_address.0;
        let hw = EthernetAddress::from_bytes(&mac[..6]);
        println!("wire: SNP initialized, mac {}", hw);

        // no clock on bare firmware: a poll-counter millisecond fake
        // keeps smoltcp's timers ordered (retransmits are coarse; a
        // LAN request/response flow doesn't mind)
        let mut fake_ms: i64 = 0;
        let mut config = Config::new(HardwareAddress::Ethernet(hw));
        config.random_seed = 0xa1e57;
        let mut iface = Interface::new(
            config, &mut dev, Instant::from_millis(fake_ms));
        iface.update_ip_addrs(|addrs| {
            addrs.push(IpCidr::new(
                smoltcp::wire::IpAddress::v4(10, 0, 2, 15), 24)).unwrap();
        });
        iface.routes_mut().add_default_ipv4_route(
            smoltcp::wire::Ipv4Address::new(10, 0, 2, 2)).unwrap();

        let rx = tcp::SocketBuffer::new(vec![0; 16384]);
        let tx = tcp::SocketBuffer::new(vec![0; 65536]);
        let sock = tcp::Socket::new(rx, tx);
        let mut sockets = SocketSet::new(vec![]);
        let h = sockets.add(sock);
        println!("wire: listening on 10.0.2.15:80");

        let mut inbuf: Vec<u8> = Vec::new();
        loop {
            fake_ms += 1;
            let ts = Instant::from_millis(fake_ms);
            iface.poll(ts, &mut dev, &mut sockets);
            let s = sockets.get_mut::<tcp::Socket>(h);
            if !s.is_open() {
                inbuf.clear();
                s.listen(80).ok();
            }
            if s.can_recv() {
                s.recv(|data| {
                    inbuf.extend_from_slice(data);
                    (data.len(), ())
                }).ok();
            }
            // one full HTTP request (headers end) = one dispatch
            if let Some(end) = find_headers_end(&inbuf) {
                let req = String::from_utf8_lossy(&inbuf[..end]).to_string();
                let answer = dispatch(&req);
                let http = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    answer.len(), answer);
                if s.can_send() {
                    s.send_slice(http.as_bytes()).ok();
                    s.close();
                    inbuf.clear();
                }
            }
        }
    }

    fn find_headers_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
    }

    // GET /{verb}?args=<urlencoded-json>: the request line IS the verb
    // dispatch; no router, no framework — the engine is the app
    fn dispatch(req: &str) -> String {
        let line = req.lines().next().unwrap_or("");
        let path = line.split_whitespace().nth(1).unwrap_or("/");
        let (verb, args) = match path.split_once('?') {
            Some((p, q)) => {
                let a = q.strip_prefix("args=").unwrap_or("{}");
                (p.trim_start_matches('/'), urldecode(a))
            }
            None => (path.trim_start_matches('/'), "{}".to_string()),
        };
        if verb.is_empty() || verb == "version" {
            return format!("{{\"version\":\"{}\"}}",
                           crate::arest_version_line());
        }
        arest::worker::arest_call(verb, &args)
    }

    fn urldecode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'%' if i + 2 < b.len() => {
                    let h = |c: u8| (c as char).to_digit(16);
                    if let (Some(x), Some(y)) = (h(b[i + 1]), h(b[i + 2])) {
                        out.push((x * 16 + y) as u8);
                        i += 3;
                        continue;
                    }
                    out.push(b[i]);
                    i += 1;
                }
                b'+' => {
                    out.push(b' ');
                    i += 1;
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).to_string()
    }
}

fn arest_version_line() -> String {
    format!("AREST OS {} / {}", env!("CARGO_PKG_VERSION"),
            arest::worker::arest_version())
}

#[cfg(feature = "mini")]
fn console_loop() -> ! {
    use std::io::{self, BufRead, Write};
    println!("console: <verb> [args-json]   (e.g. get {{\"noun\":\"Contact Submission\",\"id\":\"...\"}})");
    loop {
        print!("arest> ");
        io::stdout().flush().ok();
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).is_err() {
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (verb, args) = match line.split_once(' ') {
            Some((v, a)) => (v, a.trim()),
            None => (line, ""),
        };
        let out = arest::worker::arest_call(
            verb, if args.is_empty() { "{}" } else { args });
        println!("{}", out);
    }
}

#[cfg(feature = "full")]
const TARGET: &str = "full";
#[cfg(all(feature = "mini", not(feature = "full")))]
const TARGET: &str = "mini";
#[cfg(all(feature = "server", not(feature = "mini")))]
const TARGET: &str = "server";

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

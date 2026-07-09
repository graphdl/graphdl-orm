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
    println!("boot: complete");

    // The engine loop replaces this spin: the server target parks on
    // the network poll once virtio-net lands. (std::thread::sleep has
    // no clock on bare firmware; the harness kills QEMU at its cap.)
    loop {
        std::hint::spin_loop();
    }
}

#[cfg(feature = "full")]
const TARGET: &str = "full";
#[cfg(all(feature = "mini", not(feature = "full")))]
const TARGET: &str = "mini";
#[cfg(all(feature = "server", not(feature = "mini")))]
const TARGET: &str = "server";

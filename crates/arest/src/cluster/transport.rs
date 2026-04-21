// crates/arest/src/cluster/transport.rs
//
// Network layer for SWIM gossip. The state machine in mod.rs is
// transport-agnostic — it produces GossipMsgs and expects them to
// arrive, it doesn't care whether the wire is UDP, TCP, or an
// in-process channel.
//
// Two impls live here:
//   - InMemTransport: a pair of channels per peer. Used by unit tests
//     so convergence / failure-detection behavior is verifiable
//     without binding sockets.
//   - UdpTransport:   send_to/recv_from on a std::net::UdpSocket with
//     freeze-bytes on the wire. Used by arest-cli's --cluster boot
//     path and by the two-process integration test.

#![cfg(all(feature = "cluster", not(feature = "no_std")))]

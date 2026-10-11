//! Regression tests for stable identifiers that must survive the AMWAPOS → TILLBH rename.
//!
//! These are compatibility identifiers, not user-visible branding. Protocol v2 was
//! already deployed using the AMWAPOS cryptographic domain. Changing that domain
//! without incrementing the protocol version makes existing paired terminals and
//! hubs unable to authenticate/decrypt each other.

use tillbh_core::channel::{aad, device_keys};

#[test]
fn protocol_v2_preserves_deployed_crypto_domain() {
    let request_aad = aad("POST", "/sync/push", "device-1", 123, "nonce-1");
    assert_eq!(
        request_aad,
        b"amwapos/2\nPOST\n/sync/push\ndevice-1\n123\nnonce-1",
        "protocol v2 AAD is a deployed cryptographic compatibility constant",
    );

    let (terminal_to_hub, hub_to_terminal) = device_keys("legacy-device-key");
    assert_eq!(
        hex::encode(terminal_to_hub),
        "54849c23e6285dc87026b8961997e1d19abf31c4bbe0beeeab3f4bf7a523c16c",
        "protocol v2 terminal→hub key derivation must stay compatible",
    );
    assert_eq!(
        hex::encode(hub_to_terminal),
        "031059aec7e5edaa266b319680795fcc59733fe45db3a76974cbddb0f6f1c6eb",
        "protocol v2 hub→terminal key derivation must stay compatible",
    );
}

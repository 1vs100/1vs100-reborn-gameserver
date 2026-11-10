use base64::{Engine as _, engine::general_purpose::STANDARD};
use rsa::{RsaPrivateKey, pkcs1::DecodeRsaPrivateKey, traits::PublicKeyParts};

const GRID_LAN_MODULUS_1317: &str = "sI/yGKekfXLHDqOiRguTKWZ1XXYF6zphB8iY0dXcXtPGYoxqF7nhrbajRRwT6jMkjNXYF2eYfVePK/heQ7C8D5F2gYTcPQNWP7gFw6VQzdSofbD1kFKbmrpxVE0A5upUpac2kHs/BGB61+BcA8cvEQgdwMwjDNS5kc91tb9L5DE=";

#[test]
fn packaged_grid_lan_key_matches_loader_modulus() {
    let key = RsaPrivateKey::from_pkcs1_pem(include_str!("../keys/grid_lan_private.pem"))
        .expect("packaged GRID LAN key parses as PKCS#1 RSA private key");
    let modulus = STANDARD.encode(key.n().to_bytes_be());

    assert_eq!(modulus.len(), 172);
    assert_eq!(modulus, GRID_LAN_MODULUS_1317);
}

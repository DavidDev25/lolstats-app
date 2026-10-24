//! Signatur der lolstats-Releases mit Ed25519.
//!
//!   lolstats-sign keygen <schlüsseldatei>         neuen privaten Schlüssel anlegen, öffentlichen ausgeben
//!   lolstats-sign pubkey <schlüsseldatei>         öffentlichen Schlüssel ausgeben (für update.rs)
//!   lolstats-sign sign <schlüsseldatei> <datei>   <datei>.sig schreiben (Signatur als Hex)
//!   lolstats-sign verify <pubkey-hex> <datei>     <datei>.sig prüfen
//!
//! Der private Schlüssel gehört NICHT ins Repository (Standard: %USERPROFILE%\.lolstats-signing\release.key).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::path::Path;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    let s = s.trim();
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("kein Hex")).collect()
}

fn load_key(path: &str) -> SigningKey {
    let seed: [u8; 32] = unhex(&std::fs::read_to_string(path).expect("Schlüsseldatei fehlt")).try_into().expect("Schlüssel muss 32 Byte haben");
    SigningKey::from_bytes(&seed)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("keygen") => {
            let path = Path::new(&a[2]);
            assert!(!path.exists(), "Schlüsseldatei existiert schon – nicht überschreiben");
            let mut seed = [0u8; 32];
            getrandom::getrandom(&mut seed).expect("Zufall");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, hex(&seed)).unwrap();
            println!("{}", hex(SigningKey::from_bytes(&seed).verifying_key().as_bytes()));
        }
        Some("pubkey") => println!("{}", hex(load_key(&a[2]).verifying_key().as_bytes())),
        Some("sign") => {
            let key = load_key(&a[2]);
            let data = std::fs::read(&a[3]).expect("Datei fehlt");
            let sig = key.sign(&data);
            std::fs::write(format!("{}.sig", a[3]), hex(&sig.to_bytes()) + "\n").unwrap();
            // Gegenprobe, damit nie eine kaputte Signatur veröffentlicht wird
            key.verifying_key().verify_strict(&data, &sig).expect("Gegenprobe fehlgeschlagen");
            println!("signiert: {}.sig", a[3]);
        }
        Some("verify") => {
            let pk: [u8; 32] = unhex(&a[2]).try_into().expect("öffentlicher Schlüssel: 32 Byte");
            let data = std::fs::read(&a[3]).expect("Datei fehlt");
            let sig: [u8; 64] = unhex(&std::fs::read_to_string(format!("{}.sig", a[3])).expect(".sig fehlt")).try_into().expect("Signatur: 64 Byte");
            match VerifyingKey::from_bytes(&pk).unwrap().verify(&data, &Signature::from_bytes(&sig)) {
                Ok(()) => println!("Signatur gültig"),
                Err(e) => {
                    eprintln!("Signatur UNGÜLTIG: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Aufruf: lolstats-sign keygen|pubkey|sign|verify …");
            std::process::exit(2);
        }
    }
}

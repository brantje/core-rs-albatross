use rand::thread_rng;

use nimiq_bls::*;
use nimiq_utils::key_rng::SecureGenerate;

fn main() {
    let rng = &mut thread_rng();

    let keypair = KeyPair::generate(rng);

    let message = "Who is on first.".to_string();

    let sig = keypair.sign(&message);

    println!("Signature:\n {sig}");
}

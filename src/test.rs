use std::str::from_utf8;
use uuid::Uuid;

mod encryption;

fn main() {
    let party1 = encryption::MessageEncryptor::default();
    let party2 = encryption::MessageEncryptor::default();

    let party1_key = party1.encoded_public_key();
    let party2_key = party2.encoded_public_key();

    let nonce = party1.generate_nonce();
    println!("{}", nonce);
    let nonce = "unique nonce";
    let cipher = party1.encrypt_as_text(&party2_key, &nonce, "hello");
    println!("Cipher: {:?}", cipher);
    let plain_text = party2.decrypt_from_text(&party1_key, &nonce, &cipher);
    println!("Plain Text: {}", plain_text);

    let party2_1 = party2.clone();
    let plain_text2 = party2_1.decrypt_from_text(&party1_key, &nonce, &cipher);
    println!("Plain Text: {}", plain_text2);


}

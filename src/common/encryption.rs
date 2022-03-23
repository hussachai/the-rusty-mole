
use rand_core::OsRng;
use x25519_dalek::{PublicKey, SharedSecret, StaticSecret};
use aes_gcm::{Aes256Gcm, Key, Nonce}; // Or `Aes128Gcm`
use aes_gcm::aead::{Aead, NewAead};
use rand::distributions::{Alphanumeric, DistString};
extern crate base64;
use base64::{encode, decode};

#[derive(Clone)]
pub struct MessageEncryptor {
    private_key: StaticSecret,
    public_key: PublicKey,
}

impl Default for MessageEncryptor {

    fn default() -> Self {
        let private_key = StaticSecret::new(OsRng);
        let public_key = PublicKey::from(&private_key);
        Self { private_key, public_key }
    }
}

impl MessageEncryptor {

    fn generate_nonce(&self) -> String {
        Alphanumeric.sample_string(&mut rand::thread_rng(), 12)
    }

    pub fn encoded_public_key(&self) -> String {
        encode(self.public_key.as_bytes())
    }

    fn derive_shared_secret(&self, encoded_public_key: &str) -> SharedSecret {
        let public_key_vec = &decode(encoded_public_key).unwrap();
        let mut public_key_byes:  [u8; 32] = [0; 32];
        for i in 0..32 {
            public_key_byes[i] = public_key_vec[i];
        }
        let public_key = PublicKey::from(public_key_byes);
        let shared_secret = self.private_key.diffie_hellman(&public_key);
        shared_secret
    }

    pub fn encrypt(&self, encoded_public_key: &str, plain_text: &str) -> Vec<u8> {
        let shared_secret = self.derive_shared_secret(encoded_public_key);
        let secret_key = Key::from_slice(shared_secret.as_bytes());
        let cipher = Aes256Gcm::new(secret_key);
        let nonce_str = self.generate_nonce();
        let nonce_bytes = nonce_str.as_bytes();
        let nonce = Nonce::from_slice(nonce_bytes);
        let cipher_vec = cipher.encrypt(nonce, plain_text.as_bytes().as_ref())
            .expect("encryption failure!"); // NOTE: handle this error to avoid panics!
        [nonce_bytes.to_vec(), cipher_vec].concat()
    }

    pub fn decrypt(&self, encoded_public_key: &str, cipher_vec_with_nonce: Vec<u8>) -> Vec<u8> {
        let shared_secret = self.derive_shared_secret(encoded_public_key);
        let secret_key = Key::from_slice(shared_secret.as_bytes());
        let cipher = Aes256Gcm::new(secret_key);
        let nonce = Nonce::from_slice(&cipher_vec_with_nonce[0..12]);
        let cipher_vec = &cipher_vec_with_nonce[12..];
        let plain_vec = cipher.decrypt(nonce, cipher_vec.as_ref())
            .expect("decryption failure!");
        plain_vec
    }

}

#[test]
fn test_key_exchange() {
    let bob = MessageEncryptor::default();
    let nonce = bob.generate_nonce();
    let alice = MessageEncryptor::default();

    let message = "Hello Alice";
    let cipher_text = bob.encrypt(&alice.encoded_public_key(), message);
    let plain_text = alice.decrypt(&bob.encoded_public_key(), cipher_text);
    assert_eq!(message, String::from_utf8(plain_text).unwrap());
}

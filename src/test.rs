use std::str::{from_utf8, FromStr};
use actix_web::http;
use actix_web::rt::System;
use awc::SendClientRequest;
use uuid::Uuid;
use futures::executor::block_on;
mod encryption;

fn encryption_test() {
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

async fn hello_world() {
    println!("hello, world!");
}

async fn test_awc() {
    let http_client = awc::Client::default();
    let http_method = http::Method::from_str("GET").unwrap();
    println!("2, {:?}", http_method);
    let target_uri = format!("http://localhost:{}/ip", 8888);
    println!("3");
    let http_uri = http::Uri::from_str(&target_uri).unwrap();
    println!("4, {:?}", http_uri);
    let mut http_request = http_client.request(http_method, http_uri);
    println!("5");
    // for (key, value) in &request_data.headers {
    //     http_request = http_request.set_header(key, value.to_string());
    // }
    // println!("6, {:?}", http_request.headers());
    let r = http_request.send().await.unwrap();
    println!("Response status: {}", r.status());
    // let mut http_response = match request_data.body {
    //     Some(body) => http_request.send_body(body),
    //     _ => http_request.send()
    // }.await.expect("It failed!");
}

fn main() {
    System::new("test").block_on(test_awc());
}


use actix_web::{Error, HttpResponse, web};
use crate::common::encryption;

pub async fn handle(message_encryptor: web::Data<encryption::MessageEncryptor>) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok()
        .content_type("text/plain")
        .body(message_encryptor.encoded_public_key()))
}

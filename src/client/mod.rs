
use actix_web::web::Bytes;
use tokio::sync::mpsc::UnboundedSender;
use crate::common::encryption;

pub mod options;
pub mod handler;

pub const MIN_RETRY_WAIT_TIME: u64 = 1;

pub const MAX_RETRY_WAIT_TIME: u64 = 60;

#[derive(Clone)]
pub struct ClientContext {
    pub options: options::ClientOptions,
    pub http_agent: ureq::Agent,
    pub message_encryptor: encryption::MessageEncryptor,
    pub server_public_key: String,
    pub sender: UnboundedSender<Bytes>
}

pub async fn fetch_server_key(http_client: &awc::Client) -> String {
    let mut response = http_client.get("http://localhost:8080/public-key")
        .send()
        .await.unwrap();
    let data = response.body().await.unwrap();
    String::from_utf8(data.to_vec()).unwrap()
}


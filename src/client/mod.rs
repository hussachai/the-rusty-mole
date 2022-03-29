use actix_codec::Framed;
use actix_web::web::Bytes;
use tokio::sync::mpsc::UnboundedSender;
use crate::common::encryption;
use tokio::{select, sync::mpsc};
use awc::{BoxedSocket, ws};
use awc::ws::Codec;
use futures_util::{SinkExt as _, StreamExt as _};
use futures_util::task::SpawnExt;
use futures::executor::ThreadPool;
use tokio_stream::wrappers::UnboundedReceiverStream;

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

pub async fn event_loop(thread_pool: &ThreadPool,
                    client_context: &ClientContext,
                    mut receiver_stream: UnboundedReceiverStream<Bytes>,
                    mut ws: Framed<BoxedSocket, Codec>) {
    loop {
        select! {
            Some(msg) = ws.next() => {
                match msg {
                    Ok(ws::Frame::Binary(data)) => {
                        log::info!("Received: {:?}", data);
                        let new_client_context = client_context.clone();
                        thread_pool.spawn(async move {
                            handler::handle(&new_client_context, data).await;
                        });
                    }
                    Ok(ws::Frame::Text(txt)) => {
                        // log echoed messages from server
                        log::info!("Server Message: {:?}", txt)
                    }

                    Ok(ws::Frame::Ping(_)) => {
                        // respond to ping probes
                        ws.send(ws::Message::Pong(Bytes::new())).await.unwrap();
                    }

                    _ => {}
                }
            }
            Some(element) = receiver_stream.next() => {
                if element.is_empty() {
                    continue;
                }
                log::info!("Yo I got you {:?}", element);
                ws.send(ws::Message::Binary(element.into())).await.unwrap();
            }
            else => break
        }
    }
}

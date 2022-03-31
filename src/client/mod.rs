use actix_codec::Framed;
use actix_web::web::Bytes;
use tokio::sync::mpsc::UnboundedSender;
use crate::common::encryption;
use tokio::select;
use awc::{BoxedSocket, ws};
use awc::ws::Codec;
use futures_util::{SinkExt as _, StreamExt as _};
use futures_util::task::SpawnExt;
use futures::executor::ThreadPool;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio::time;

pub mod options;
pub mod handler;

pub const MAX_RETRY_WAIT_TIME: u64 = 10;
pub const PING_INTERVAL_SECONDS: u64 = 5;

#[derive(Clone)]
pub struct ClientContext {
    pub options: options::ClientOptions,
    pub http_agent: ureq::Agent,
    pub message_encryptor: encryption::MessageEncryptor,
    pub server_public_key: String,
    pub sender: UnboundedSender<Bytes>
}

pub async fn fetch_server_key(http_client: &awc::Client, host_base_uri: &str) -> Option<String> {
    let public_key_endpoint = format!("{}/public-key", host_base_uri);
    let mut response = http_client.get(public_key_endpoint)
        .send()
        .await.ok()?;

    let data = response.body().await.ok()?;
    String::from_utf8(data.to_vec()).ok()
}

pub async fn event_loop(thread_pool: &ThreadPool,
                    client_context: &ClientContext,
                    mut receiver_stream: UnboundedReceiverStream<Bytes>,
                    mut ws: Framed<BoxedSocket, Codec>) {
    let mut interval = time::interval(time::Duration::from_secs(PING_INTERVAL_SECONDS));
    loop {
        select! {
            instant = interval.tick() => {
                let result = ws.send(ws::Message::Ping(Bytes::new())).await;
                match result {
                    Ok(_) => log::debug!("Sent Ping to the server."),
                    Err(e) => {
                        log::error!("Connection failed");
                        return;
                    }
                }
            }
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
                        match String::from_utf8(txt.to_vec()) {
                            Ok(str) => log::info!("Server Message: {:?}", txt.to_vec()),
                            Err(e) => log::error!("Server sent non-utf8 through the byte stream!") // Is this too paranoid?
                        }
                    }

                    Ok(ws::Frame::Ping(_)) => {
                        // respond to ping probes
                        ws.send(ws::Message::Pong(Bytes::new())).await;
                    }
                    Ok(ws::Frame::Pong(_)) => {
                        log::debug!("Received Pong from the server.")
                    }
                    Err(e) => {
                        log::error!("Failed to retrieve the next element.")
                    }
                    unknown => {
                        log::warn!("Received an unknown result.")
                    }
                }
            }
            Some(element) = receiver_stream.next() => {
                if element.is_empty() {
                    continue;
                }
                log::debug!("Sending a message to a server {:?}", element);
                ws.send(ws::Message::Binary(element.into())).await;
            }
            else => {
                log::warn!("The next element from the stream is None. Exiting the event loop.");
                break
            }
        }
    }
}

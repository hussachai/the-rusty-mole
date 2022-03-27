//! Simple websocket client.

use std::{io, thread};
use std::time::Duration;
use actix_web::web::Bytes;
use awc::ws;
use actix_web::HttpMessage;
use awc::http::{Uri, uri};
use futures::executor::ThreadPool;
use futures_util::{SinkExt as _, StreamExt as _};
use futures_util::task::SpawnExt;
use tokio::{select, sync::mpsc};
use tokio_stream::wrappers::UnboundedReceiverStream;
use crate::client::options;
use crate::client::handler;
use crate::common::encryption;

mod common;
mod client;


#[actix_web::main]
async fn main() {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    log::info!("starting echo WebSocket client");

    let options: options::ClientOptions = options::parse_options();

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let mut cmd_rx = UnboundedReceiverStream::new(cmd_rx);

    let input_thread = thread::spawn(move || loop {
        let mut cmd = String::with_capacity(32);
        if io::stdin().read_line(&mut cmd).is_err() {
            log::error!("error reading line");
            return;
        }
    });
    let http_client = awc::Client::default();
    let http_agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(options.timeout_read))
        .timeout_write(Duration::from_secs(options.timeout_write))
        .build();
    let message_encryptor = encryption::MessageEncryptor::default();
    let server_public_key = client::fetch_server_key(&http_client).await;
    println!("Key: {}", server_public_key);
    let sender = cmd_tx;

    let server_host = Uri::try_from(&options.server_host).unwrap();

    let is_secure = server_host.scheme().unwrap() == &uri::Scheme::HTTPS;
    let (ws_protocol, http_protocol) = if is_secure { ("wss", "https") } else { ("ws", "http") };
    let host_and_port = &options.server_host.trim_start_matches(&format!("{}://", http_protocol));
    let ws_uri = format!("{}://{}/subscribe/{}", ws_protocol, host_and_port, options.client_id);
    println!("Connecting to {}", ws_uri);
    let mut ws_client = awc::Client::new()
        .ws(ws_uri)
        .set_header(common::HEADER_PUBLIC_KEY, message_encryptor.encoded_public_key());
    if options.password != "" {
        ws_client = ws_client.set_header(common::HEADER_USERNAME, options.username.clone())
            .set_header(common::HEADER_PASSWORD, options.password.clone());
    }

    let (res, mut ws) = ws_client
        .connect()
        .await
        .unwrap();

    common::print_banner("Minion");
    let hook_uri = if options.password != "" {
        format!("{}://{}:{}@{}/hook/{}", http_protocol, options.username, options.password, host_and_port, options.client_id)
    } else {
        format!("{}://{}/hook/{}", http_protocol, host_and_port, options.client_id)
    };
    println!("The following proxy URL is ready to serve:\n{}\n\n", hook_uri);

    let pool = ThreadPool::new().expect("Failed to build pool");
    let pool_ref = &pool;

    log::debug!("response: {:?}", res);
    log::info!("connected; server will echo messages sent");

    let client_context = client::ClientContext {
        options,
        http_agent,
        message_encryptor,
        server_public_key,
        sender
    };

    loop {
        select! {
            Some(msg) = ws.next() => {
                match msg {
                    Ok(ws::Frame::Binary(data)) => {
                        log::info!("Received: {:?}", data);
                        let new_client_context = client_context.clone();
                        pool_ref.spawn(async move {
                            handler::handle(&new_client_context, data).await;
                        });
                    }
                    Ok(ws::Frame::Text(txt)) => {
                        // log echoed messages from server
                        log::info!("Server: {:?}", txt)
                    }

                    Ok(ws::Frame::Ping(_)) => {
                        // respond to ping probes
                        ws.send(ws::Message::Pong(Bytes::new())).await.unwrap();
                    }

                    _ => {}
                }
            }

            Some(cmd) = cmd_rx.next() => {
                if cmd.is_empty() {
                    continue;
                }
                log::info!("Yo I got you {:?}", cmd);
                ws.send(ws::Message::Binary(cmd.into())).await.unwrap();
            }

            else => break
        }
    }

    input_thread.join().unwrap();
}

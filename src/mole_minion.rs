//! Simple websocket client.

use std::{env, io, thread};
use std::ops::Deref;
use std::time::Duration;
use actix_web::{http, HttpMessage};
use awc::error::WsClientError;
use awc::http::{Uri, uri};
use futures::executor::ThreadPool;
use futures_lite::StreamExt;
use futures_util::{SinkExt as _, StreamExt as _};
use futures_util::task::SpawnExt;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use crate::client::options;
use crate::common::encryption;

mod common;
mod client;


#[actix_web::main]
async fn main() {
    // env_logger::init_from_env(env_logger::Env::new().default_filter_or("debug"));
    std::env::set_var("RUST_LOG", "actix_server=info,actix_web=info,mole_minion=debug");
    env_logger::init();

    log::info!("starting echo WebSocket client");

    let options: options::ClientOptions = options::parse_options();

    let http_client = awc::Client::default();
    let http_agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(options.timeout_read))
        .timeout_write(Duration::from_secs(options.timeout_write))
        .build();
    let message_encryptor = encryption::MessageEncryptor::default();

    let thread_pool = ThreadPool::new().expect("Failed to build pool");

    let server_host = Uri::try_from(&options.server_host).unwrap();

    let is_secure = server_host.scheme().unwrap() == &uri::Scheme::HTTPS;
    let (ws_protocol, http_protocol) = if is_secure { ("wss", "https") } else { ("ws", "http") };
    let host_and_port = &options.server_host.trim_start_matches(&format!("{}://", http_protocol));
    let ws_uri = format!("{}://{}/subscribe/{}", ws_protocol, host_and_port, options.client_id);

    common::print_banner("Minion");

    let hook_uri = if options.password != "" {
        format!("{}://{}:{}@{}/hook/{}", http_protocol, options.username, options.password, host_and_port, options.client_id)
    } else {
        format!("{}://{}/hook/{}", http_protocol, host_and_port, options.client_id)
    };
    println!("The following proxy URL is ready to serve:\n{}\n\n", hook_uri);

    let mut wait_time = 0;

    loop {

        if wait_time >= client::MAX_RETRY_WAIT_TIME {
            wait_time = client::MAX_RETRY_WAIT_TIME;
        }
        if wait_time != 0 {
            log::info!("Wait for {} seconds before re-connecting", wait_time);
            thread::sleep(Duration::from_secs(wait_time));
        }
        let server_public_key = match client::fetch_server_key(&http_client, &options.server_host).await {
            Some(key) => key,
            None => {
                log::error!("Failed to fetch the public key from the server");
                wait_time += 1;
                continue;
            }
        };
        log::debug!("Server public key: {}", server_public_key);

        let mut ws_client = awc::Client::new()
            .ws(&ws_uri)
            .set_header(common::HEADER_PUBLIC_KEY, message_encryptor.encoded_public_key());
        if options.password != "" {
            ws_client = ws_client.set_header(common::HEADER_USERNAME, options.username.clone())
                .set_header(common::HEADER_PASSWORD, options.password.clone());
        }
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut receiver_stream = UnboundedReceiverStream::new(receiver);
        let client_context = client::ClientContext {
            options: options.clone(),
            http_agent: http_agent.clone(),
            message_encryptor: message_encryptor.clone(),
            server_public_key: server_public_key.clone(),
            sender: sender.clone()
        };
        log::info!("Connecting to {}", &ws_uri);
        match ws_client.connect().await {
            Ok((res, mut ws)) => {
                wait_time = 0;
                client::event_loop(&thread_pool, &client_context, receiver_stream, ws).await;
            },
            Err(WsClientError::InvalidResponseStatus(_)) => {
                panic!("Invalid configurations");
            },
            Err(e) => {
                log::error!("Failed to connect the server due to {:?}", e);
                wait_time += 1;
            }
        }
    }

}

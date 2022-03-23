use std::thread;
use std::time::Duration;

use actix::{Actor, ActorContext, AsyncContext, Context, Handler, ResponseFuture, StreamHandler};
use actix::io::SinkWrite;
use awc::Client;
use futures::StreamExt;
use ureq::Agent;

use crate::client;
use crate::client::ws_client;
use crate::common::encryption;
use crate::options;

pub struct WebSocketClientInitializer {
    pub options: options::ClientOptions,
    pub client_id: String,
    pub retry_wait_time: u64 // when connection fails, we do retry after x seconds
}

impl WebSocketClientInitializer {

    fn retry_wait_time(&mut self, secs: u64) {
        self.retry_wait_time = secs;
    }
}

impl Actor for WebSocketClientInitializer {
    type Context = Context<Self>;
}

// To use actor with supervisor actor has to implement `Supervised` trait
impl actix::Supervised for WebSocketClientInitializer {

    fn restarting(&mut self, ctx: &mut Context<Self>) {
        info!("Waiting for {} seconds before attempting to reconnecting...", self.retry_wait_time);
        thread::sleep(Duration::from_secs(self.retry_wait_time));
        info!("Reconnecting...");
        ctx.address().do_send(client::Connect);
    }
}


impl Handler<client::PoisonPill> for WebSocketClientInitializer {
    type Result = ();

    fn handle(&mut self, _: client::PoisonPill, ctx: &mut Context<Self>) {
        let retry_wait_time = self.retry_wait_time + 1;
        if retry_wait_time <= client::MAX_RETRY_WAIT_TIME {
            self.retry_wait_time(retry_wait_time);
        }
        ctx.stop();
    }
}

impl Handler<client::ResetRetryWaitTime> for WebSocketClientInitializer {
    type Result = ();

    fn handle(&mut self, _: client::ResetRetryWaitTime, _: &mut Context<Self>) {
        self.retry_wait_time(client::MIN_RETRY_WAIT_TIME);
    }
}

// TODO: Move to awc once the API is stable
fn fetch_server_public_key(agent: &Agent, server_host: &str) -> String {
    let uri = format!("{}/public-key", server_host);
    let response = agent.request("GET", &uri).call().unwrap();
    response.into_string().unwrap()
}

impl Handler<client::Connect> for WebSocketClientInitializer {
    type Result = ResponseFuture<()>;

    fn handle(&mut self, _: client::Connect, ctx: &mut Context<Self>) -> Self::Result {
        let init_actor = ctx.address().clone();
        let options = self.options.clone();
        let client_id = self.client_id.clone();

        Box::pin(async move {

            let http_agent: Agent = ureq::AgentBuilder::new()
                .timeout_read(Duration::from_secs(options.timeout_read))
                .timeout_write(Duration::from_secs(options.timeout_write))
                .build();

            let message_encryptor = encryption::MessageEncryptor::default();
            let server_public_key = fetch_server_public_key(&http_agent, &options.server_host);

            let ws_connect = Client::builder()
                // Currently only HTTP1.1 supports Websocket until https://datatracker.ietf.org/doc/html/rfc8441 got implemented.
                .max_http_version(awc::http::Version::HTTP_11)
                .finish()
                .ws(format!("{}/subscribe/{}", options.server_host, client_id))
                .header("X-Public-Key", message_encryptor.encoded_public_key())
                .connect()
                .await;

            match ws_connect {
                Ok((response, framed)) => {
                    debug!("{:?}", response);
                    debug!("Client Public Key: {:?}", message_encryptor.encoded_public_key());

                    init_actor.do_send(client::ResetRetryWaitTime);

                    let (sink, stream) = framed.split();
                    ws_client::WebSocketClient::create(|ctx| {
                        ws_client::WebSocketClient::add_stream(stream, ctx);
                        ws_client::WebSocketClient {
                            init_actor,
                            options,
                            http_agent,
                            message_encryptor,
                            server_public_key,
                            writer: SinkWrite::new(sink, ctx),
                        }
                    });
                }
                Err(e) => {
                    warn!("Error occurred: {:?}", e);
                    init_actor.do_send(client::PoisonPill);
                }
            }
        })
    }
}




#[macro_use]
extern crate log;

use std::time::Duration;
use std::collections::HashMap;
use std::io::Read;
use std::str::from_utf8;
use std::{env, thread};

use actix::io::SinkWrite;
use actix::*;
use actix_codec::Framed;
use awc::{error::WsProtocolError, ws::{Codec, Frame, Message}, BoxedSocket, Client};
use bytes::Bytes;
use futures::stream::{SplitSink, StreamExt};
use uuid::Uuid;
use crate::models::{RequestData, ResponseData, SecureEnvelop};
use crate::encryption::MessageEncryptor;
use clap::Parser;
use ureq::{Agent, Error};

mod models;
mod encryption;

const MIN_RETRY_WAIT_TIME: u64 = 1;

const MAX_RETRY_WAIT_TIME: u64 = 60;

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info, runtome=info");
    env_logger::init();


    let mut sys = System::new("websocket-client");
    let mut options: ClientOptions = Parser::parse();

    env::var("RUNTOME_SERVER_HOST").into_iter().for_each(|host| {
        // Override the server host with ENV variable only when the default value is used.
        if options.server_host == "http://localhost:8080" {
            options.server_host = host;
        }
    });

    let client_id = if &options.client_id == "" {
        Uuid::new_v4().to_string().replace("-", "")
    } else {
        options.client_id.clone()
    };

    models::print_banner("Client");
    println!("The following proxy URL is ready to serve:\n{}/hook/{}\n\n", options.server_host, client_id);

    let retry_wait_time = MIN_RETRY_WAIT_TIME;
    let addr = sys.block_on(async move {
        actix::Supervisor::start(move |_| WebSocketClientInitializer{options, client_id, retry_wait_time})
    });
    addr.do_send(Connect);

    sys.run().unwrap();

}

// TODO: support subdomain
/// The open source secure introspectable tunnels to localhost.
#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
struct ClientOptions {
    /// A custom client_id. If omitted, UUID v4 will be used.
    #[clap(short, long, default_value = "")]
    client_id: String,

    /// The port number that the traffic will be routed to.
    #[clap(short = 'p', long)]
    target_port: u16,

    /// The server host
    #[clap(short, long, default_value = "http://localhost:8080")]
    server_host: String,

    /// Timeout for the individual reads of the socket in seconds
    #[clap(long, default_value = "30")]
    timeout_read: u64,

    /// Timeout for the individual writes of the socket in seconds
    #[clap(long, default_value = "30")]
    timeout_write: u64,

    /// Show the request/response in the console
    #[clap(short, long)]
    debug: bool

}

struct WebSocketClientInitializer {
    options: ClientOptions,
    client_id: String,
    retry_wait_time: u64 // when connection fails, we do retry after x seconds
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
        ctx.address().do_send(Connect);
    }
}

impl Handler<PoisonPill> for WebSocketClientInitializer {
    type Result = ();

    fn handle(&mut self, _: PoisonPill, ctx: &mut Context<Self>) {
        let retry_wait_time = self.retry_wait_time + 1;
        if retry_wait_time <= MAX_RETRY_WAIT_TIME {
            self.retry_wait_time(retry_wait_time);
        }
        ctx.stop();
    }
}

impl Handler<ResetRetryWaitTime> for WebSocketClientInitializer {
    type Result = ();

    fn handle(&mut self, _: ResetRetryWaitTime, _: &mut Context<Self>) {
        self.retry_wait_time(MIN_RETRY_WAIT_TIME);
    }
}

impl Handler<Connect> for WebSocketClientInitializer {
    type Result = ResponseFuture<()>;
    fn handle(&mut self, _: Connect, ctx: &mut Context<Self>) -> Self::Result {
        let init_actor = ctx.address().clone();
        let options = self.options.clone();
        let client_id = self.client_id.clone();

        Box::pin(async move {

            let message_encryptor = MessageEncryptor::default();

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

                    init_actor.do_send(ResetRetryWaitTime);

                    let (sink, stream) = framed.split();
                    WebSocketClient::create(|ctx| {
                        WebSocketClient::add_stream(stream, ctx);
                        let http_agent: Agent = ureq::AgentBuilder::new()
                            .timeout_read(Duration::from_secs(options.timeout_read))
                            .timeout_write(Duration::from_secs(options.timeout_write))
                            .build();
                        WebSocketClient {
                            init_actor,
                            options,
                            http_agent,
                            message_encryptor,
                            writer: SinkWrite::new(sink, ctx),
                        }
                    });
                }
                Err(e) => {
                    warn!("Error occurred: {}", e);
                    init_actor.do_send(PoisonPill);
                }
            }
        })
    }
}

struct WebSocketClient {
    init_actor: Addr<WebSocketClientInitializer>,
    options: ClientOptions,
    http_agent: Agent,
    message_encryptor: MessageEncryptor,
    writer: SinkWrite<Message, SplitSink<Framed<BoxedSocket, Codec>, Message>>,
}

#[derive(Message)]
#[rtype(result = "()")]
struct PoisonPill;

#[derive(Message)]
#[rtype(result = "()")]
struct ResetRetryWaitTime;

#[derive(Message)]
#[rtype(result = "()")]
struct Connect;

#[derive(Message)]
#[rtype(result = "()")]
struct BuildRequest {
    data: Bytes
}

struct ServerKey {
    server_public_key: String,
    nonce: String
}

#[derive(Message)]
#[rtype(result = "()")]
struct ExecuteRequest {
    server_key: ServerKey,
    request_data: RequestData
}

#[derive(Message)]
#[rtype(result = "()")]
struct SendResponse {
    server_key: ServerKey,
    request_id: String,
    response_data: ResponseData
}

impl Actor for WebSocketClient {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Context<Self>) {
        // start heartbeats otherwise server will disconnect after 10 seconds
        self.hb(ctx)
    }

    fn stopped(&mut self, ctx: &mut Context<Self>) {
        info!("Disconnected");
        // Stop application on disconnect
        ctx.stop();
        self.init_actor.do_send(Connect)
        // System::current().stop();
    }
}

impl WebSocketClient {
    fn hb(&self, ctx: &mut Context<Self>) {
        ctx.run_later(Duration::new(5, 0), |act, ctx| {
            act.writer.write(Message::Ping(Bytes::from_static(b"")));
            act.hb(ctx);
            // client should also check for a timeout here, similar to the
            // server code
        });
    }

}

impl Handler<BuildRequest> for WebSocketClient {
    type Result = ();

    fn handle(&mut self, msg: BuildRequest, ctx: &mut Context<Self>) {
        let secure_incoming_envelop_json = from_utf8(&msg.data).unwrap();
        debug!("Secure Incoming Envelop: {:?}", secure_incoming_envelop_json);

        let secure_incoming_envelop: SecureEnvelop = serde_json::from_str(secure_incoming_envelop_json).unwrap();
        let server_public_key =  secure_incoming_envelop.encoded_public_key;
        let nonce =  secure_incoming_envelop.nonce;
        let request_data_json = self.message_encryptor.decrypt_from_text(
            &server_public_key,
            &nonce,
            &secure_incoming_envelop.encrypted_payload
        );
        debug!("Request data JSON: {:?}", request_data_json);

        let request_data: RequestData = serde_json::from_str(&request_data_json).unwrap();
        if self.options.debug {
            info!("{}", request_data);
        }
        let server_key = ServerKey {
            server_public_key,
            nonce
        };
        ctx.address().do_send(ExecuteRequest { server_key, request_data });
    }
}

impl Handler<ExecuteRequest> for WebSocketClient {

    type Result = ResponseFuture<()>;

    fn handle(&mut self, msg: ExecuteRequest, ctx: &mut Context<Self>) -> Self::Result {
        let target_port = self.options.target_port;
        let is_debuggable = self.options.debug;
        let http_agent = self.http_agent.clone();
        let self_addr = ctx.address().clone();
        Box::pin(async move {
            let server_key = msg.server_key;
            let request_data = msg.request_data;
            let request_id = request_data.request_id;
            debug!("Path: {}", request_data.path);
            let target_uri = format!("http://localhost:{}/{}", target_port, request_data.path);
            let mut http_request = http_agent.request(&request_data.method, &target_uri);
            for (key, value) in &request_data.headers {
                http_request = http_request.set(key, value);
            }
            let http_response_result = match request_data.body {
                Some(body) => http_request.send_string(&body),
                _ => http_request.call()
            };
            let http_response = match http_response_result {
                Ok(response) => {
                    response
                },
                Err(Error::Status(_, response)) => {
                    /* the server returned an unexpected status
                    code (such as 400, 500 etc) */
                    response
                }
                Err(_) => {
                    ureq::Response::new(503, "Service Unavailable", "").unwrap()
                }
            };
            let res_status = http_response.status();
            let res_content_type = http_response.content_type().to_string();
            let mut res_headers: HashMap<String, String> = HashMap::new();
            for header_name in http_response.headers_names().iter() {
                res_headers.insert(header_name.to_string(), http_response.header(header_name).unwrap().to_string());
            }

            let mut res_body_buf: Vec<u8> = vec![];
            let _ = http_response.into_reader().read_to_end(&mut res_body_buf);
            let res_body = if res_body_buf.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(&res_body_buf).to_string())
            };
            // TODO: support octet stream
            let response_data = ResponseData {
                status: res_status,
                headers: res_headers,
                content_type: res_content_type,
                body: res_body
            };

            if is_debuggable {
                info!("{}", response_data);
            }

            self_addr.do_send(SendResponse { server_key, request_id, response_data })
        })
    }
}

impl Handler<SendResponse> for WebSocketClient {
    type Result = ();

    fn handle(&mut self, msg: SendResponse, _: &mut Context<Self>) {
        let server_public_key = msg.server_key.server_public_key;
        let nonce = msg.server_key.nonce;
        let request_id = msg.request_id;
        let response_data = msg.response_data;
        debug!("Response Data: {:?}", response_data);
        let response_data_json = serde_json::to_string(&response_data).unwrap();
        debug!("Response Data JSON: {:?}", response_data_json);
        let client_public_key = self.message_encryptor.encoded_public_key();
        debug!("Server Public Key: {:?}", server_public_key);
        let encrypted_payload = self.message_encryptor.encrypt_as_text(
            &server_public_key, &nonce, &response_data_json);

        let secure_outgoing_envelop = SecureEnvelop {
            encoded_public_key: client_public_key,
            nonce,
            encrypted_payload
        };

        let secure_outgoing_envelop_json = serde_json::to_string(&secure_outgoing_envelop).unwrap();
        let id_envelop = format!("{}{}", request_id, secure_outgoing_envelop_json);

        self.writer.write(Message::Text(id_envelop));
    }
}
/// Handle server websocket messages
impl StreamHandler<Result<Frame, WsProtocolError>> for WebSocketClient {

    fn handle(&mut self, msg: Result<Frame, WsProtocolError>, ctx: &mut Context<Self>) {
        if let Ok(Frame::Text(data)) = msg {
            debug!("Server: {:?}", data);
            ctx.address().do_send(BuildRequest { data });
        }
    }

    fn started(&mut self, _ctx: &mut Context<Self>) {
        info!("Connected");
    }

    fn finished(&mut self, ctx: &mut Context<Self>) {
        info!("Server disconnected");
        ctx.stop()
    }
}

impl actix::io::WriteHandler<WsProtocolError> for WebSocketClient {}

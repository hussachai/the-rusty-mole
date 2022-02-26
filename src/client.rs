use std::time::Duration;
use std::{io, thread};
use std::collections::HashMap;
use std::env::Args;
use std::io::Read;
use std::str::{from_utf8, FromStr};

use actix::io::SinkWrite;
use actix::*;
use actix_codec::Framed;
use actix_web::{HttpMessage, HttpResponse};
use awc::{error::WsProtocolError, ws::{Codec, Frame, Message}, BoxedSocket, Client, http};
use bytes::Bytes;
use futures::stream::{SplitSink, StreamExt};
use uuid::Uuid;
use crate::common::{RequestData, ResponseData, SecureEnvelop};
use crate::encryption::MessageEncryptor;
use clap::Parser;
use futures::{TryFutureExt, TryStreamExt};
use tracing::log;
use ureq::{Agent, Error};

mod common;
mod encryption;

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();

    let sys = System::new("websocket-client");

    Arbiter::spawn(async {
        let options: RunToMeOptions = Parser::parse();
        println!("ClientID: {}, Port: {}", options.client_id, options.port);
        let message_encryptor = MessageEncryptor::default();
        let client_id = Uuid::new_v4().to_string();
        let (response, framed) = Client::new()
            .ws(format!("http://127.0.0.1:8080/subscribe/{}", client_id))
            .header("X-Public-Key", message_encryptor.encoded_public_key())
            .connect()
            .await
            .map_err(|e| {
                println!("Error: {:?}", e);
            })
            .unwrap();

        println!("{:?}", response);
        println!("Client Public Key: {:?}", message_encryptor.encoded_public_key());

        let (sink, stream) = framed.split();
        let addr = ChatClient::create(|ctx| {
            ChatClient::add_stream(stream, ctx);
            let http_agent: Agent = ureq::AgentBuilder::new()
                .timeout_read(Duration::from_secs(30))
                .timeout_write(Duration::from_secs(30))
                .build();
            ChatClient {
                options,
                http_agent,
                message_encryptor,
                writer: SinkWrite::new(sink, ctx),
            }
        });

        // start console loop
        thread::spawn(move || loop {
            let mut cmd = String::new();
            if io::stdin().read_line(&mut cmd).is_err() {
                println!("error");
                return;
            }
            addr.do_send(ClientCommand(cmd));
        });
    });
    sys.run().unwrap();
}

/// The open source secure introspectable tunnels to localhost.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct RunToMeOptions {
    /// A custom client_id. If omitted, UUID v4 will be used.
    #[clap(short, long)]
    client_id: String,

    /// The port number that the traffic will be routed to.
    #[clap(short, long)]
    port: u16
}

struct ChatClient {
    options: RunToMeOptions,
    http_agent: Agent,
    message_encryptor: MessageEncryptor,
    writer: SinkWrite<Message, SplitSink<Framed<BoxedSocket, Codec>, Message>>,
}

#[derive(Message)]
#[rtype(result = "()")]
struct ClientCommand(String);

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

impl Actor for ChatClient {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Context<Self>) {
        // start heartbeats otherwise server will disconnect after 10 seconds
        self.hb(ctx)
    }

    fn stopped(&mut self, _: &mut Context<Self>) {
        println!("Disconnected");

        // Stop application on disconnect
        System::current().stop();
    }
}

impl ChatClient {
    fn hb(&self, ctx: &mut Context<Self>) {
        ctx.run_later(Duration::new(5, 0), |act, ctx| {
            act.writer.write(Message::Ping(Bytes::from_static(b"")));
            act.hb(ctx);
            // client should also check for a timeout here, similar to the
            // server code
        });
    }

}

/// Handle stdin commands
impl Handler<ClientCommand> for ChatClient {
    type Result = ();

    fn handle(&mut self, msg: ClientCommand, _ctx: &mut Context<Self>) {
        self.writer.write(Message::Text(msg.0));
    }
}

impl Handler<BuildRequest> for ChatClient {
    type Result = ();

    fn handle(&mut self, msg: BuildRequest, ctx: &mut Context<Self>) {
        let secure_incoming_envelop_json = from_utf8(&msg.data).unwrap();
        println!("Secure Incoming Envelop: {:?}", secure_incoming_envelop_json);

        let secure_incoming_envelop: SecureEnvelop = serde_json::from_str(secure_incoming_envelop_json).unwrap();
        let server_public_key =  secure_incoming_envelop.encoded_public_key;
        let nonce =  secure_incoming_envelop.nonce;
        let request_data_json = self.message_encryptor.decrypt_from_text(
            &server_public_key,
            &nonce,
            &secure_incoming_envelop.encrypted_payload
        );
        println!("Request data JSON: {:?}", request_data_json);

        let request_data: RequestData = serde_json::from_str(&request_data_json).unwrap();
        println!("Yes : {:?}", request_data);
        let server_key = ServerKey {
            server_public_key,
            nonce
        };
        ctx.address().do_send(ExecuteRequest { server_key, request_data });
    }
}

impl Handler<ExecuteRequest> for ChatClient {

    type Result = ResponseFuture<()>;

    fn handle(&mut self, msg: ExecuteRequest, ctx: &mut Context<Self>) -> Self::Result {
        let target_port = self.options.port;
        let http_agent = self.http_agent.clone();
        let self_addr = ctx.address().clone();
        Box::pin(async move {
            let server_key = msg.server_key;
            let request_data = msg.request_data;
            let request_id = request_data.request_id;
            println!("Path: {}", request_data.path);
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
                    // TODO: log successful
                    response
                },
                Err(Error::Status(code, response)) => {
                    /* the server returned an unexpected status
                    code (such as 400, 500 etc) */
                    response
                }
                Err(_) => {
                    // TODO: handle it gracefully
                    panic!("Transport error")
                }
            };
            let res_status = http_response.status();
            let res_content_type = http_response.content_type().to_string();
            let mut res_headers: HashMap<String, String> = HashMap::new();
            for header_name in http_response.headers_names().iter() {
                res_headers.insert(header_name.to_string(), http_response.header(header_name).unwrap().to_string());
            }

            let mut res_body_buf: Vec<u8> = vec![];
            http_response.into_reader().read_to_end(&mut res_body_buf);
            let res_body = if res_body_buf.is_empty() {
                None
            } else {
                Some(String::from_utf8_lossy(&res_body_buf).to_string())
            };
            // let res_body = Some(http_response.into_string().unwrap());
            // TODO: support octet stream
            let response_data = ResponseData {
                status: res_status,
                headers: res_headers,
                content_type: res_content_type,
                body: res_body
            };
            // ctx.address().do_send(SendResponse { server_key, request_id, response_data });
            self_addr.do_send(SendResponse { server_key, request_id, response_data })
        })
    }
}

impl Handler<SendResponse> for ChatClient {
    type Result = ();

    fn handle(&mut self, msg: SendResponse, ctx: &mut Context<Self>) {
        let server_public_key = msg.server_key.server_public_key;
        let nonce = msg.server_key.nonce;
        let request_id = msg.request_id;
        let response_data = msg.response_data;
        println!("Response Data: {:?}", response_data);
        let response_data_json = serde_json::to_string(&response_data).unwrap();
        println!("Response Data JSON: {:?}", response_data_json);
        let client_public_key = self.message_encryptor.encoded_public_key();
        println!("Server Public Key: {:?}", server_public_key);
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
impl StreamHandler<Result<Frame, WsProtocolError>> for ChatClient {

    fn handle(&mut self, msg: Result<Frame, WsProtocolError>, ctx: &mut Context<Self>) {
        let self_addr = ctx.address().clone();
        if let Ok(Frame::Text(data)) = msg {
            println!("Server: {:?}", data);
            ctx.address().do_send(BuildRequest { data });
        }
    }

    fn started(&mut self, _ctx: &mut Context<Self>) {
        println!("Connected");
    }

    fn finished(&mut self, ctx: &mut Context<Self>) {
        println!("Server disconnected");
        ctx.stop()
    }
}

impl actix::io::WriteHandler<WsProtocolError> for ChatClient {}

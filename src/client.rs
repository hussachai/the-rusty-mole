use std::time::Duration;
use std::{io, thread};
use std::collections::HashMap;
use std::env::Args;
use std::str::from_utf8;

use actix::io::SinkWrite;
use actix::*;
use actix_codec::Framed;
use awc::{
    error::WsProtocolError,
    ws::{Codec, Frame, Message},
    BoxedSocket, Client,
};
use bytes::Bytes;
use futures::stream::{SplitSink, StreamExt};
use uuid::Uuid;
use crate::common::{RequestData, ResponseData, SecureEnvelop};
use crate::encryption::MessageEncryptor;
use clap::Parser;

mod common;
mod encryption;

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();

    let sys = System::new("websocket-client");

    Arbiter::spawn(async {
        let args: RunToMeOptions = Parser::parse();
        println!("ClientID: {}, Port: {}", args.client_id, args.port);
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
            ChatClient {
                writer: SinkWrite::new(sink, ctx),
                message_encryptor
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
    writer: SinkWrite<Message, SplitSink<Framed<BoxedSocket, Codec>, Message>>,
    message_encryptor: MessageEncryptor,
}

#[derive(Message)]
#[rtype(result = "()")]
struct ClientCommand(String);

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

/// Handle server websocket messages
impl StreamHandler<Result<Frame, WsProtocolError>> for ChatClient {
    fn handle(&mut self, msg: Result<Frame, WsProtocolError>, _: &mut Context<Self>) {

        if let Ok(Frame::Text(txt)) = msg {
            println!("Server: {:?}", txt);

            let secure_incoming_envelop_json = from_utf8(&txt).unwrap();
            println!("Secure Incoming Envelop: {:?}", secure_incoming_envelop_json);

            let secure_incoming_envelop: SecureEnvelop = serde_json::from_str(secure_incoming_envelop_json).unwrap();

            let request_data_json = self.message_encryptor.decrypt_from_text(
                &secure_incoming_envelop.encoded_public_key,
                &secure_incoming_envelop.nonce,
                &secure_incoming_envelop.encrypted_payload
            );
            println!("Request data JSON: {:?}", request_data_json);

            let request_data: RequestData = serde_json::from_str(&request_data_json).unwrap();
            println!("Yes : {:?}", request_data);
            // TODO: call a server

            let response_data: ResponseData = ResponseData {
                status: 200,
                headers: HashMap::default(),
                body: Some("Hello World!!!".to_string())
            };

            let response_data_json = serde_json::to_string(&response_data).unwrap();
            let server_public_key = secure_incoming_envelop.encoded_public_key;
            let client_public_key = self.message_encryptor.encoded_public_key();
            let nonce = secure_incoming_envelop.nonce;
            println!("Server Public Key: {:?}", server_public_key);
            let encrypted_payload = self.message_encryptor.encrypt_as_text(
                &server_public_key, &nonce, &response_data_json);

            let secure_outgoing_envelop = SecureEnvelop {
                data_id: secure_incoming_envelop.data_id,
                encoded_public_key: client_public_key,
                nonce,
                encrypted_payload
            };

            let secure_outgoing_envelop_json = serde_json::to_string(&secure_outgoing_envelop).unwrap();
            self.writer.write(Message::Text(secure_outgoing_envelop_json));

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

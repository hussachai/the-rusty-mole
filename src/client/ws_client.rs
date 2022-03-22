use std::collections::HashMap;
use std::io::Read;
use std::str::from_utf8;
use std::time::Duration;

use actix::{Actor, ActorContext, Addr, AsyncContext, Context, Handler, ResponseFuture};
use actix::io::SinkWrite;
use actix_codec::Framed;
use awc::{BoxedSocket, ws::{Codec, Message}};
use bytes::Bytes;
use futures::stream::SplitSink;
use ureq::{Agent, Error};

use crate::client;
use crate::client::ws_init;
use crate::common;
use crate::common::encryption;
use crate::options;

pub struct WebSocketClient {
    pub init_actor: Addr<ws_init::WebSocketClientInitializer>,
    pub options: options::ClientOptions,
    pub http_agent: Agent,
    pub message_encryptor: encryption::MessageEncryptor,
    pub writer: SinkWrite<Message, SplitSink<Framed<BoxedSocket, Codec>, Message>>,
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
        self.init_actor.do_send(client::Connect)
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

impl Handler<client::BuildRequest> for WebSocketClient {
    type Result = ();

    fn handle(&mut self, msg: client::BuildRequest, ctx: &mut Context<Self>) {
        let secure_incoming_envelop_json = from_utf8(&msg.data).unwrap();
        debug!("Secure Incoming Envelop: {:?}", secure_incoming_envelop_json);

        let secure_incoming_envelop: common::SecureEnvelop = serde_json::from_str(secure_incoming_envelop_json).unwrap();
        let server_public_key =  secure_incoming_envelop.encoded_public_key;
        let nonce =  secure_incoming_envelop.nonce;
        let request_data_json = self.message_encryptor.decrypt_from_text(
            &server_public_key,
            &nonce,
            &secure_incoming_envelop.encrypted_payload
        );
        debug!("Request data JSON: {:?}", request_data_json);

        let request_data: common::RequestData = serde_json::from_str(&request_data_json).unwrap();
        if self.options.debug {
            info!("{}", request_data);
        }
        let server_key = client::ServerKey {
            server_public_key,
            nonce
        };
        ctx.address().do_send(client::ExecuteRequest { server_key, request_data });
    }
}

impl Handler<client::ExecuteRequest> for WebSocketClient {

    type Result = ResponseFuture<()>;

    fn handle(&mut self, msg: client::ExecuteRequest, ctx: &mut Context<Self>) -> Self::Result {
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
            let response_data = common::ResponseData {
                status: res_status,
                headers: res_headers,
                content_type: res_content_type,
                body: res_body
            };

            if is_debuggable {
                info!("{}", response_data);
            }

            self_addr.do_send(client::SendResponse { server_key, request_id, response_data })
        })
    }
}

impl Handler<client::SendResponse> for WebSocketClient {
    type Result = ();

    fn handle(&mut self, msg: client::SendResponse, _: &mut Context<Self>) {
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

        let secure_outgoing_envelop = common::SecureEnvelop {
            encoded_public_key: client_public_key,
            nonce,
            encrypted_payload
        };

        let secure_outgoing_envelop_json = serde_json::to_string(&secure_outgoing_envelop).unwrap();
        let id_envelop = format!("{}{}", request_id, secure_outgoing_envelop_json);

        self.writer.write(Message::Text(id_envelop));
    }
}

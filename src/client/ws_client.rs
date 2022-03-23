
use std::collections::HashMap;
use std::io::Read;
use std::thread;
use std::time::Duration;
use actix::*;
use actix::io::SinkWrite;
use actix_codec::Framed;
use actix_web::client::WsProtocolError;
use actix_web_actors::ws::Frame;
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
    pub server_public_key: String,
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
        let request_data_json = self.message_encryptor.decrypt(&self.server_public_key, msg.data);
        // debug!("Request data JSON bytes: {:?}", request_data_json);
        let request_data: common::RequestData = serde_json::from_slice(request_data_json.as_slice()).unwrap();
        // debug!("Request data: {:?}", request_data);

        if self.options.debug {
            info!("{}", request_data);
        }
        ctx.address().do_send(client::ExecuteRequest { request_data });
    }
}

impl Handler<client::ExecuteRequest> for WebSocketClient {

    type Result = ResponseFuture<()>;

    fn handle(&mut self, msg: client::ExecuteRequest, ctx: &mut Context<Self>) -> Self::Result {
        // let server_public_key = self.server_public_key.clone();
        let target_port = self.options.target_port;
        let is_debuggable = self.options.debug;
        let http_agent = self.http_agent.clone();
        let self_addr = ctx.address().clone();

        Box::pin(async move {
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

            self_addr.do_send(client::SendResponse { request_id, response_data })
        })
    }
}

impl Handler<client::SendResponse> for WebSocketClient {
    type Result = ();

    fn handle(&mut self, msg: client::SendResponse, _: &mut Context<Self>) {
        let request_id = msg.request_id;
        let response_data = msg.response_data;
        // debug!("Response Data: {:?}", response_data);
        let response_data_json = serde_json::to_string(&response_data).unwrap();
        debug!("Response Data JSON: {:?}", response_data_json);
        let encrypted_response = self.message_encryptor.encrypt(
            &self.server_public_key, &response_data_json);
        let result = [request_id.as_bytes().to_vec(), encrypted_response].concat();
        self.writer.write(Message::Binary(Bytes::from(result)));
    }
}

/// Handle server websocket messages
impl StreamHandler<Result<Frame, WsProtocolError>> for WebSocketClient {

    fn handle(&mut self, msg: Result<Frame, WsProtocolError>, ctx: &mut Context<Self>) {
        if let Ok(Frame::Binary(data)) = msg {
            ctx.address().do_send(client::BuildRequest { data: data.to_vec() });
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


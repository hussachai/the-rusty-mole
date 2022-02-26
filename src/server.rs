extern crate qstring;

use std::borrow::Borrow;
use std::collections::HashMap;
use std::str::from_utf8;
use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_files as fs;
use actix_web::{App, Error, HttpMessage, HttpRequest, HttpResponse, HttpServer, middleware, web};
use actix_web::http::StatusCode;
use actix_web_actors::ws;
use awc::ws::Message::Text;
use deadpool_lapin::{Config, Pool, Runtime};
use deadpool_lapin::lapin::{BasicProperties, Channel, Consumer};
use deadpool_lapin::lapin::options::{BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, QueueDeclareOptions, QueueDeleteOptions};
use deadpool_lapin::lapin::types::FieldTable;
use futures_lite::FutureExt;
use futures_lite::stream::StreamExt;
use lapin::{Connection, ConnectionProperties};
use lapin::types::ShortString;
use qstring::QString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::common::{RequestData, ResponseData, SecureEnvelop};
use crate::encryption::MessageEncryptor;

mod common;
mod encryption;

/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

/// do websocket handshake and start `MyWebSocket` actor
async fn ws_index(pool: web::Data<Pool>,
                  message_encryptor: web::Data<MessageEncryptor>,
                  request: HttpRequest,
                  paths: web::Path<(String)>,
                  stream: web::Payload) -> Result<HttpResponse, Error> {
    println!("{:?}", request);
    let client_public_key = request.headers().get("X-Public-Key").unwrap().to_str().unwrap();
    let client_id = paths.0; // Uuid::new_v4().to_string()?;
    let connection = pool.get().await.unwrap();
    let channel = connection.create_channel().await.unwrap();
    let queue_req_name = format!("{}_req", client_id);
    let queue_res_name = format!("{}_res", client_id);

    println!("Client Public Key: {:?}", client_public_key);

    // TODO: obtain public key from the request
    // TODO: WS should send out the server public key out immediately
    channel.queue_declare(
            &queue_req_name,
            QueueDeclareOptions::default(),
            FieldTable::default(),
    ).await;
    channel.queue_declare(
            &queue_res_name,
            QueueDeclareOptions::default(),
            FieldTable::default(),
    ).await;
    let actor = MyWebSocket::new(
        client_id,
        message_encryptor.get_ref().clone(),
        client_public_key.to_string(),
        channel,
        queue_req_name,
        queue_res_name
    );
    let response = ws::start(actor, &request, stream);
    println!("{:?}", response);
    response
}

async fn handle_hook(pool: web::Data<Pool>,
                     message_encryptor: web::Data<MessageEncryptor>,
                     request: HttpRequest,
                     paths: web::Path<(String, String)>,
                     body: web::Bytes) -> Result<HttpResponse, Error> {
    let (client_id, tail_path) = paths.into_inner();
    println!("Remaining path: {}", tail_path);
    println!("Query: {}", request.query_string());
    let connection = pool.get().await.unwrap();
    let channel = connection.create_channel().await.unwrap();
    let queue_req_name = format!("{}_req", client_id);
    let queue_res_name = format!("{}_res", client_id);

    let query_pairs = QString::from(request.query_string()).into_pairs();

    let method = request.method().to_string();
    // Remove `Connection` as per
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Connection#Directives
    let mut headers: HashMap<String, String> = HashMap::new();
    for (header_name, header_value) in request.headers().iter().filter(|(h, _)| *h != "connection") {
        headers.insert(header_name.as_str().to_string(), header_value.to_str().unwrap().to_string());
    }
    let content_type = request.content_type().to_string();
    let body = Some(String::from_utf8(body.to_vec()).unwrap());
    let request_id = Uuid::new_v4().to_string();
    println!("Body: {:?}", body);
    let request_data = RequestData {
        request_id: request_id.clone(),
        path: tail_path,
        query_pairs,
        method,
        headers,
        content_type,
        body
    };
    let serialized_request_data = serde_json::to_vec(&request_data)?;

    let confirm = channel
        .basic_publish(
            "",
            &queue_req_name,
            BasicPublishOptions::default(),
            serialized_request_data,
            BasicProperties::default(),
        ).await.unwrap();

    let mut client_resp = HttpResponse::build(StatusCode::OK);

    let mut consumer = channel
        .basic_consume(
            &queue_res_name,
            "my_consumer",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        ).await.unwrap();

    let delivery_data = loop {
        match consumer.next().await {
            Some(result) => {
                let (_, delivery) = result.unwrap();
                let message_id = &delivery.properties.message_id().clone().unwrap().to_string();
                if message_id == &request_id {
                    delivery.ack(BasicAckOptions::default()).await.expect("ack");
                    break delivery.data
                }
            }
            _ => {}
        }
    };
    let secure_response_envelop_json = from_utf8(&delivery_data).unwrap();
    println!("Secure Response Envelop JSON: {}", secure_response_envelop_json);
    let secure_response_envelop: SecureEnvelop = serde_json::from_str(secure_response_envelop_json).unwrap();
    println!("Secure Response Envelop: {:?}", secure_response_envelop);
    let client_public_key = secure_response_envelop.encoded_public_key;
    println!("Client Public Key: {:?}", client_public_key);
    println!("Server Public Key: {:?}", message_encryptor.encoded_public_key());
    let nonce = secure_response_envelop.nonce;
    println!("Nonce: {:?}", nonce);
    let encrypted_payload = secure_response_envelop.encrypted_payload;
    println!("Encrypted Payload: {:?}", encrypted_payload);

    let response_data_json = message_encryptor.decrypt_from_text(
        &client_public_key, &nonce, &encrypted_payload);
    let response_data: ResponseData = serde_json::from_str(&response_data_json).unwrap();
    client_resp.status(StatusCode::from_u16(response_data.status).unwrap());
    client_resp.content_type(response_data.content_type);
    for (key, value) in response_data.headers {
        client_resp.set_header(key, value);
    }

    match response_data.body {
        Some(body) => Ok(client_resp.body(body)),
        None => Ok(client_resp.finish())
    }

}

/// websocket connection is long running connection, it easier
/// to handle with an actor
struct MyWebSocket {
    /// Client must send ping at least once per 10 seconds (CLIENT_TIMEOUT),
    /// otherwise we drop connection.
    hb: Instant,
    client_id: String,
    message_encryptor: MessageEncryptor,
    client_public_key: String,
    channel: Channel,
    queue_req_name: String,
    queue_res_name: String
}

#[derive(Message)]
#[rtype(result="()")]
struct TextMessage {
    text: String
}

impl Handler<TextMessage> for MyWebSocket {
    type Result = ();

    fn handle(&mut self, msg: TextMessage, ctx: &mut Self::Context) {
        ctx.text(msg.text);
    }
}

impl Actor for MyWebSocket {

    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start. We start the heartbeat process here.
    fn started(&mut self, ctx: &mut Self::Context) {

        let queue_req = self.queue_req_name.clone();
        let channel = self.channel.clone();
        let message_encryptor = self.message_encryptor.clone();
        let client_public_key = self.client_public_key.clone();

        let self_addr = ctx.address().clone();

        async_global_executor::spawn(async move {

            let mut consumer = channel
                .basic_consume(
                    &queue_req,
                    "my_consumer",
                    BasicConsumeOptions::default(),
                    FieldTable::default(),
                ).await.unwrap();
            while let Some(delivery) = consumer.next().await {
                let (_, delivery) = delivery.expect("error in consumer");
                let request_data = from_utf8(&delivery.data).unwrap();
                let nonce = message_encryptor.generate_nonce();
                let encrypted_payload = message_encryptor.encrypt_as_text(&client_public_key, &nonce, request_data);
                let server_public_key = message_encryptor.encoded_public_key();
                let secure_envelop = SecureEnvelop {
                    encoded_public_key: server_public_key,
                    nonce,
                    encrypted_payload,
                };

                let serialized_secure_envelop = serde_json::to_string(&secure_envelop).unwrap();

                self_addr.do_send(TextMessage {text: serialized_secure_envelop });

                println!("Consumed: {:?}", request_data);

                delivery.ack(BasicAckOptions::default()).await.expect("ack");
            }
        }).detach();

        self.hb(ctx);
    }

    // I think it might be ok to do a clean up here even we have a cluster of this,
    // the WS connection between LB and the client should be sticky.
    fn stopped(&mut self, ctx: &mut Self::Context) {
        let queue_req = format!("{}_req", self.client_id);
        let queue_res = format!("{}_res", self.client_id);
        self.channel.queue_delete(&queue_req, QueueDeleteOptions::default());
        self.channel.queue_delete(&queue_res, QueueDeleteOptions::default());
    }
}

/// Handler for `ws::Message`
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for MyWebSocket {

    fn handle(
        &mut self,
        msg: Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {
        let queue_res = self.queue_res_name.clone();
        let channel = self.channel.clone();

        // process websocket messages
        // println!("WS: {:?}", msg);
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.hb = Instant::now();
            }
            Ok(ws::Message::Text(text)) => {
                // substring for the UUID?
                let message_id = &text[0..36];
                let encrypted_response = text[36..].to_string().as_bytes().to_vec();
                let properties = BasicProperties::default().with_message_id(message_id.into());

                channel.basic_publish(
                        "",
                        &queue_res,
                        BasicPublishOptions::default(),
                        encrypted_response,
                        properties,
                );
                println!("Response data: {}", text);
            },
            Ok(ws::Message::Binary(bin)) => ctx.binary(bin),
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => ctx.stop(),
        }
    }
}

impl MyWebSocket {
    fn new(client_id: String,
           message_encryptor: MessageEncryptor,
           client_public_key: String,
           channel: Channel,
           queue_req_name: String,
           queue_res_name: String) -> Self {
        println!("Hello NEW");

        Self { hb: Instant::now(), client_id, message_encryptor, client_public_key, channel, queue_req_name, queue_res_name }
    }

    /// helper method that sends ping to client every second.
    ///
    /// also this method checks heartbeats from client
    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            // check client heartbeats
            if Instant::now().duration_since(act.hb) > CLIENT_TIMEOUT {
                // heartbeat timed out
                println!("Websocket Client heartbeat failed, disconnecting!");

                // stop actor
                ctx.stop();

                // don't try to send a ping
                return;
            }

            ctx.ping(b"");

        });
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_server=info,actix_web=info");
    env_logger::init();

    let mut cfg = Config::default();
    cfg.url = Some("amqp://127.0.0.1:5672/%2f".into());
    let pool = cfg.create_pool(Some(Runtime::AsyncStd1)).unwrap();
    let message_encryptor = MessageEncryptor::default();

    HttpServer::new(move || {
        println!("New Message Encryptor!!");
        App::new()
            // enable logger
            .wrap(middleware::Logger::default())
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(message_encryptor.clone()))
            // websocket route
            .service(web::resource("/subscribe/{client_id}").route(web::get().to(ws_index)))
            .service(web::resource("/hook/{client_id}/{paths:.*}").route(web::to(handle_hook)))
            // static files
            .service(fs::Files::new("/", "static/").index_file("index.html"))
    })
        // start http server on 127.0.0.1:8080
        .bind("127.0.0.1:8080")?
        .run()
        .await
}

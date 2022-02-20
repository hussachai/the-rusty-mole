use std::collections::HashMap;
use std::str::from_utf8;
use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_files as fs;
use actix_web::{middleware, web, App, Error, HttpRequest, HttpResponse, HttpServer};
use actix_web::http::StatusCode;
use actix_web_actors::ws;
use bytes::Buf;
use lru::LruCache;
use uuid::Uuid;
use futures::channel::oneshot;
use futures::StreamExt;
use futures_channel::oneshot::{Receiver, Sender};

/// How often heartbeat pings are sent
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// How long before lack of client response causes a timeout
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

struct RequestPayload {
    headers: HashMap<String, String>,
    body: String
}
struct ResponsePayload {
    headers: HashMap<String, String>,
    body: String
}

/// do websocket handshake and start `MyWebSocket` actor
async fn ws_index(request_queue: web::Data<HashMap<&str, (&Sender<&RequestPayload>, &Receiver<&RequestPayload>)>>,
                  response_queue: web::Data<HashMap<&str, (&Sender<&ResponsePayload>, &Receiver<&ResponsePayload>)>>,
                  r: HttpRequest, paths: web::Path<(String)>,
                  stream: web::Payload) -> Result<HttpResponse, Error> {
    println!("{:?}", r);

    let (sender, receiver) = oneshot::channel::<web::Payload>();

    let client_id = paths.0; // Uuid::new_v4().to_string()?;

    let res = ws::start(MyWebSocket::new(&client_id, request_queue.get_ref(), response_queue.get_ref()), &r, stream);
    println!("{:?}", res);
    res
}

async fn handle_hook(mut request_queue: web::Data<HashMap<&str, (&Sender<&RequestPayload>, &Receiver<&RequestPayload>)>>,
                     response_queue: web::Data<HashMap<&str, (&Sender<&ResponsePayload>, &Receiver<&ResponsePayload>)>>,
                     request: HttpRequest,
                     paths: web::Path<(String)>,
                     body: web::Bytes) -> Result<HttpResponse, Error> {
    let client_id = paths.0;
    let mut client_resp = HttpResponse::build(StatusCode::OK);
    // Remove `Connection` as per
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Connection#Directives
    let mut headers: HashMap<String, String> = HashMap::new();
    for (header_name, header_value) in request.headers().iter().filter(|(h, _)| *h != "connection") {
        headers.insert(header_name.as_str().to_string(), header_value.to_str().unwrap().to_string());
    }

    let body = String::from_utf8(body.to_vec()).unwrap();
    println!("Body: {}", body);
    let simpleRequest = RequestPayload { headers, body };

    let (sender, receiver) = oneshot::channel::<&RequestPayload>();
    request_queue.insert(&client_id, (&sender, &receiver));

    // sender.send(&simplePayload);
    // data.insert(&client_id, (&sender, &receiver));
    // Ok(client_resp.body(body))
    // sender.send();
    // receiver.
    Ok(client_resp.body(simpleRequest.body))
}
/// websocket connection is long running connection, it easier
/// to handle with an actor
struct MyWebSocket {
    /// Client must send ping at least once per 10 seconds (CLIENT_TIMEOUT),
    /// otherwise we drop connection.
    hb: Instant
}

impl Actor for MyWebSocket {

    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start. We start the heartbeat process here.
    fn started(&mut self, ctx: &mut Self::Context) {

        self.hb(ctx);
    }
}

/// Handler for `ws::Message`
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for MyWebSocket {
    fn handle(
        &mut self,
        msg: Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {
        // process websocket messages
        println!("WS: {:?}", msg);
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.hb = Instant::now();
            }
            Ok(ws::Message::Text(text)) => ctx.text(text),
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
    fn new(client_id: &str,
           request_queue: &HashMap<&str, (&Sender<&RequestPayload>, &Receiver<&RequestPayload>)>,
           response_queues: &HashMap<&str, (&Sender<&ResponsePayload>, &Receiver<&ResponsePayload>)>) -> Self {
        println!("Hello NEW");
        Self { hb: Instant::now() }
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

    let request_queue: HashMap<&str, (&Sender<&RequestPayload>, &Receiver<&RequestPayload>)> = HashMap::new();

    let response_queue: HashMap<&str, (&Sender<&ResponsePayload>, &Receiver<&ResponsePayload>)> = HashMap::new();

    HttpServer::new(move || {
        App::new()
            // enable logger
            .wrap(middleware::Logger::default())
            .app_data(web::Data::new(request_queue.clone()))
            .app_data(web::Data::new(response_queue.clone()))
            // websocket route
            .service(web::resource("/subscribe/{client_id}").route(web::get().to(ws_index)))
            .service(web::resource("/hook/{client_id}").route(web::to(handle_hook)))
            // static files
            .service(fs::Files::new("/", "static/").index_file("index.html"))
    })
        // start http server on 127.0.0.1:8080
        .bind("127.0.0.1:8080")?
        .run()
        .await
}

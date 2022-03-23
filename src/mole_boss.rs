#[macro_use]
extern crate log;
extern crate qstring;

use actix_web::{App, HttpServer, middleware, web};
use deadpool_lapin::{Config, Runtime};

use crate::common::encryption::MessageEncryptor;
use crate::server::{handler_hook, handler_public_key, handler_subscribe, options};

mod server;
mod common;


#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_server=info,actix_web=info,mole_boss=debug");
    env_logger::init();

    let options: options::ServerOptions = options::parse_options();

    common::print_banner("Server");

    let mut cfg = Config::default();
    cfg.url = Some(options.amqp_uri.into());
    let pool = cfg.create_pool(Some(Runtime::AsyncStd1)).unwrap();
    let message_encryptor = MessageEncryptor::default();

    HttpServer::new(move || {
        App::new()
            // enable logger
            .wrap(middleware::Logger::default())
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(message_encryptor.clone()))
            // websocket route
            .service(web::resource("/subscribe/{client_id}").route(web::get().to(handler_subscribe::handle)))
            .service(web::resource("/public-key").route(web::get().to(handler_public_key::handle)))
            .service(web::resource("/hook/{client_id}/{paths:.*}").route(web::to(handler_hook::handle)))
            .route("/ping", web::get().to(|| async { "pong" }))
    })
        // start http server on 127.0.0.1:8080
        .bind(format!("{}:{}", options.bind_addr, options.port))?
        .run()
        .await
}

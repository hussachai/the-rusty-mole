#[macro_use]
extern crate log;
extern crate core;

use std::env;

use actix::*;
use uuid::Uuid;

use crate::client::options;
use crate::client::ws_init;

mod common;
mod client;

fn main() {
    ::std::env::set_var("RUST_LOG", "actix_web=info, mole_minion=debug");
    env_logger::init();

    let mut sys = System::new("websocket-client");
    let mut options: options::ClientOptions = options::parse_options();

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

    common::print_banner("Client");
    println!("The following proxy URL is ready to serve:\n{}/hook/{}\n\n", options.server_host, client_id);

    let retry_wait_time = client::MIN_RETRY_WAIT_TIME;
    let addr = sys.block_on(async move {
        actix::Supervisor::start(move |_| ws_init::WebSocketClientInitializer{options, client_id, retry_wait_time})
    });
    addr.do_send(client::Connect);

    sys.run().unwrap();

}



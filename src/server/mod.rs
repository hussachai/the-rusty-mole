use std::time::Duration;

use actix::prelude::*;
use actix_web::web::Bytes;

pub mod options;
pub mod handler_hook;
pub mod handler_subscribe;
pub mod handler_public_key;
pub mod ws_server;

/// How often heartbeat pings are sent
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// How long before lack of client response causes a timeout
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);


#[derive(Message)]
#[rtype(result="()")]
struct Payload {
    data: Bytes
}


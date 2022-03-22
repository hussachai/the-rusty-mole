use actix::prelude::*;
use bytes::Bytes;

use crate::common;

pub mod options;
pub mod ws_client;
pub mod ws_init;

pub const MIN_RETRY_WAIT_TIME: u64 = 1;

pub const MAX_RETRY_WAIT_TIME: u64 = 60;

#[derive(Message)]
#[rtype(result = "()")]
struct PoisonPill;

#[derive(Message)]
#[rtype(result = "()")]
struct ResetRetryWaitTime;

#[derive(Message)]
#[rtype(result = "()")]
pub struct Connect;

#[derive(Message)]
#[rtype(result = "()")]
pub struct BuildRequest {
    pub data: Bytes
}

struct ServerKey {
    server_public_key: String,
    nonce: String
}

#[derive(Message)]
#[rtype(result = "()")]
struct ExecuteRequest {
    server_key: ServerKey,
    request_data: common::RequestData
}

#[derive(Message)]
#[rtype(result = "()")]
struct SendResponse {
    server_key: ServerKey,
    request_id: String,
    response_data: common::ResponseData
}

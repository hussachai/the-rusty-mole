use actix::prelude::*;
use crate::common;

pub mod options;

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
    pub data: Vec<u8>
}

#[derive(Message)]
#[rtype(result = "()")]
struct ExecuteRequest {
    request_data: common::RequestData
}

#[derive(Message)]
#[rtype(result = "()")]
struct SendResponse {
    request_id: String,
    response_data: common::ResponseData
}

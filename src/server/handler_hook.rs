use std::collections::HashMap;
use actix_web::{Error, HttpMessage, HttpRequest, HttpResponse, web};
use actix_web::http::StatusCode;
use deadpool_lapin::lapin::BasicProperties;
use deadpool_lapin::lapin::options::{BasicAckOptions, BasicConsumeOptions, BasicNackOptions, BasicPublishOptions};
use deadpool_lapin::lapin::types::FieldTable;
use deadpool_lapin::Pool;
use futures_lite::stream::StreamExt;
use qstring::QString;
use uuid::Uuid;
use crate::common;

pub async fn handle(pool: web::Data<Pool>,
                    request: HttpRequest,
                    paths: web::Path<(String, String)>,
                    body: web::Bytes) -> Result<HttpResponse, Error> {
    let (client_id, tail_path) = paths.into_inner();
    debug!("Tail path: {}", tail_path);
    debug!("Query: {}", request.query_string());
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

    let request_data = common::RequestData {
        request_id: request_id.clone(),
        path: tail_path,
        query_pairs,
        method,
        headers,
        content_type,
        body,
    };
    let serialized_request_data = serde_json::to_vec(&request_data)?;

    let _ = channel
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
        log::debug!("Fetching a message from the queue: {}", queue_res_name);
        match consumer.next().await {
            Some(result) => {
                let (_, delivery) = result.unwrap();
                let message_id = &delivery.properties.message_id().clone().unwrap().to_string();
                log::debug!("[req:{}] Hook received a message, and it's waiting for the ID: {}", request_id, message_id);
                if message_id == request_id {
                    log::debug!("[req:{}] Message and request ID match. Deleting the message and returning the response.", request_id);
                    delivery.ack(BasicAckOptions::default()).await.expect("ack");
                    break delivery.data;
                } else {
                    log::debug!("[req:{}] Message and request ID do not match. Re-queueing the message.", request_id);
                    delivery.nack(BasicNackOptions { multiple: false, requeue: true}).await.expect("nack")
                }
            }
            _ => {
                log::warn!("[req:{}] Consumed an empty message", request_id)
            }
        }
    };

    let response_data: common::ResponseData = serde_json::from_slice(&delivery_data).unwrap();
    client_resp.status(StatusCode::from_u16(response_data.status).unwrap());
    client_resp.content_type(response_data.content_type);
    for (key, value) in response_data.headers {
        client_resp.insert_header((key, value));
    }

    match response_data.body {
        Some(body) => Ok(client_resp.body(body)),
        None => Ok(client_resp.finish())
    }
}

use std::collections::HashMap;
use actix_web::{Error, error, HttpMessage, HttpRequest, HttpResponse, web};
use actix_web::http::StatusCode;
use deadpool_lapin::lapin::BasicProperties;
use deadpool_lapin::lapin::options::{BasicAckOptions, BasicConsumeOptions, BasicNackOptions, BasicPublishOptions};
use deadpool_lapin::lapin::types::{AMQPValue, FieldTable};
use deadpool_lapin::Pool;
use futures_lite::stream::StreamExt;
use qstring::QString;
use base64;
use uuid::Uuid;

use crate::{common, server};
use crate::options::ServerOptions;

pub async fn handle(options: web::Data<ServerOptions>,
                    pool: web::Data<Pool>,
                    request: HttpRequest,
                    paths: web::Path<(String, String)>,
                    body: web::Bytes) -> Result<HttpResponse, Error> {
    let (client_id, tail_path) = paths.into_inner();

    let credentials = request.headers().get(common::HEADER_AUTHORIZATION).and_then(|auth_value| {
        let basic_auth = auth_value.to_str().ok()?.to_string();
        let basic_auth_value = basic_auth.trim_start_matches("Basic ");
        base64::decode(basic_auth_value).ok().and_then (|v| {
            String::from_utf8(v).ok()
        })
    });

    if credentials.is_none() && options.password_required {
        // We will return 404
        return Err(error::ErrorNotFound(server::MSG_NOT_FOUND));
    }

    let connection = pool.get().await.map_err(|e| {
        error::ErrorInternalServerError(server::MSG_SOMETHING_WRONG)
    })?;
    let channel = connection.create_channel().await.map_err(|e| {
        error::ErrorInternalServerError(server::MSG_SOMETHING_WRONG)
    })?;

    let queue_req_name = format!("{}_req", client_id);
    let queue_res_name = format!("{}_res", client_id);

    let query_pairs = QString::from(request.query_string()).into_pairs();

    let method = request.method().to_string();
    // Remove `Connection` as per
    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Connection#Directives
    let mut req_headers: HashMap<String, String> = HashMap::new();
    for (header_name, header_value) in request.headers().iter().filter(|(h, _)| *h != "connection") {
        let h_name = header_name.as_str().to_string();
        match header_value.to_str() {
            Ok(h_value) => req_headers.insert(h_name, h_value.to_string()),
            Err(e) => {
                log::error!("Unable to decode the header value of {}", h_name);
                None
            }
        };
    }
    let content_type = request.content_type().to_string();
    let body = Some(String::from_utf8(body.to_vec()).map_err(|e| {
        error::ErrorBadRequest(server::MSG_INVALID_STRING)
    })?);
    let request_id = Uuid::new_v4().to_string();

    let request_data = common::RequestData {
        request_id: request_id.clone(),
        path: tail_path,
        query_pairs,
        method,
        headers: req_headers,
        content_type,
        body,
    };
    let serialized_request_data = serde_json::to_vec(&request_data)?;

    let mut queue_headers = FieldTable::default();
    if let Some(user_and_pass) = credentials {
        queue_headers.insert(common::HEADER_AUTHORIZATION.into(), AMQPValue::LongString(user_and_pass.into()));
        queue_headers.insert(common::HEADER_REQUEST_ID.into(), AMQPValue::LongString(request_id.clone().into()));
    }
    let request_ip = request.peer_addr().map_or("0.0.0.0".to_string(), |addr| { addr.to_string()});
    queue_headers.insert(common::HEADER_REQUEST_IP.into(), AMQPValue::LongString(request_ip.into()));

    let _ = channel
        .basic_publish(
            "",
            &queue_req_name,
            BasicPublishOptions::default(),
            serialized_request_data,
            BasicProperties::default().with_headers(queue_headers),
        ).await.map_err(|e| {
        log::error!("Failed to publish to the queue: {} due to: {}", queue_req_name, e.to_string());
        error::ErrorNotFound(server::MSG_NOT_FOUND)
    })?;


    let mut client_resp = HttpResponse::build(StatusCode::OK);

    let mut consumer = channel
        .basic_consume(
            &queue_res_name,
            "my_consumer",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        ).await.map_err(|e| {
        log::error!("Failed to consume to the queue: {} due to: {}", queue_res_name, e.to_string());
        error::ErrorNotFound(server::MSG_NOT_FOUND)
    })?;

    let delivery_data = loop {
        log::debug!("Fetching a message from the queue: {}", queue_res_name);
        match consumer.next().await {
            Some(Ok((_, delivery))) => {
                let message_id = match &delivery.properties.message_id() {
                    Some(value) => value.to_string(),
                    _ => {
                        log::error!("[req:{}] Message ID is empty", request_id);
                        continue;
                    }
                };
                log::debug!("[req:{}] Hook received a message, and it's waiting for the ID: {}", request_id, message_id);
                if message_id == request_id {
                    log::debug!("[req:{}] Message and request ID match. Deleting the message and returning the response.", request_id);
                    delivery.ack(BasicAckOptions::default()).await.map_err(|e| {
                        log::error!("[req:{}] Failed to ack due to {}", request_id, e.to_string())
                    });
                    break delivery.data;
                } else {
                    log::debug!("[req:{}] Message and request ID do not match. Re-queueing the message.", request_id);
                    delivery.nack(BasicNackOptions { multiple: false, requeue: true}).await.map_err(|e| {
                        log::error!("[req:{}] Failed to nack due to {}", request_id, e.to_string())
                    });
                }
            }
            Some(Err(e)) => {
                log::error!("[req: {}] Failed to consume the next element due to {}", request_id, e.to_string());
            }
            _ => {
                log::warn!("[req:{}] Consumed an empty message", request_id)
            }
        }
    };

    let response_data: common::ResponseData = serde_json::from_slice(&delivery_data).map_err(|e| {
        log::error!("[req:{}] Failed to parse the JSON due to {}", request_id, e.to_string());
        error::ErrorInternalServerError("Invalid data")
    })?;
    // Not sure this is the right behavior, but if the status is greater than 1000, I think returning a tea pot is appropriate.
    client_resp.status(StatusCode::from_u16(response_data.status).unwrap_or(StatusCode::IM_A_TEAPOT));
    client_resp.content_type(response_data.content_type);
    for (key, value) in response_data.headers {
        client_resp.insert_header((key, value));
    }

    match response_data.body {
        Some(body) => Ok(client_resp.body(body)),
        None => Ok(client_resp.finish())
    }

}


use std::io::Read;
use std::collections::HashMap;
use actix_web::web::Bytes;
use actix_web::HttpMessage;
use futures_util::{SinkExt as _, StreamExt as _};
use qstring::QString;
use ureq::Error;

use crate::client;
use crate::common;

pub async fn handle(client_context: &client::ClientContext, data: Bytes) {
    let request_data = decrypt_request(client_context, data).await;
    let request_id = request_data.request_id.clone();
    let response_data = execute_request(client_context, request_data).await;
    println!("Response Data {:?}", response_data);
    let encrypted_data = encrypt_response(client_context, request_id, response_data).await;
    client_context.sender.send(encrypted_data).unwrap();
}

async fn decrypt_request(client_context: &client::ClientContext, data: Bytes) -> common::RequestData {
    let request_data_json = client_context.message_encryptor.decrypt(&client_context.server_public_key, data.to_vec());
    // debug!("Request data JSON bytes: {:?}", request_data_json);
    let request_data: common::RequestData = serde_json::from_slice(request_data_json.as_slice()).unwrap();
    log::info!("{}", request_data);
    if client_context.options.debug {
        log::info!("{}", request_data);
    }
    request_data
}

async fn execute_request(client_context: &client::ClientContext, request_data: common::RequestData) -> common::ResponseData {
    // let target_uri = format!("http://localhost:{}/{}", client_context.options.target_port, request_data.path);
    // let http_client = awc::Client::new();
    // let method = Method::from_bytes(request_data.method.as_bytes()).unwrap();
    // let path_and_query = if !request_data.query_pairs.is_empty() {
    //     format!("{}?{}", request_data.path, QString::new(request_data.query_pairs))
    // } else {
    //     request_data.path
    // };
    // let uri = uri::Builder::new().scheme("http").path_and_query(path_and_query).build().unwrap();
    // // TODO: query??
    // // let uri = Uri::try_from(&target_uri).unwrap();
    // let mut request = http_client.request(method, uri);
    // for (key, value) in &request_data.headers {
    //     request = request.insert_header((key.to_string(), value.to_string()));
    // }
    // let mut http_response = match request_data.body {
    //     Some(body) => request.send_body(body),
    //     _ => request.send()
    // }.await.unwrap();
    // let res_status = http_response.status().as_u16();
    // let res_content_type = http_response.content_type().to_string();
    // let mut res_headers: HashMap<String, String> = HashMap::new();
    // for (header_name, header_value) in http_response.headers().iter() {
    //     let value = String::from_utf8(header_value.as_bytes().to_vec()).unwrap();
    //     res_headers.insert(header_name.to_string(), value);
    // }
    // let res_body_bytes = http_response.body().await.unwrap();
    // let res_body = if res_body_bytes.is_empty() {
    //     None
    // } else {
    //     Some(String::from_utf8(res_body_bytes.to_vec()).unwrap())
    // };
    // let response_data = common::ResponseData {
    //     status: res_status,
    //     headers: res_headers,
    //     content_type: res_content_type,
    //     body: res_body
    // };
    // if client_context.options.debug {
    //     log::info!("{}", response_data);
    // }
    // response_data

    let request_id = request_data.request_id;
    log::debug!("Path: {}", request_data.path);

    let target_uri = format!("http://localhost:{}/{}", client_context.options.target_port, request_data.path);

    let target_uri_with_query = if !request_data.query_pairs.is_empty() {
        format!("{}?{}", target_uri, QString::new(request_data.query_pairs))
    } else {
        target_uri
    };
    log::info!("Uri: {}", target_uri_with_query);
    let mut http_request = client_context.http_agent.request(&request_data.method, &target_uri_with_query);
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

    if client_context.options.debug {
        log::info!("{}", response_data);
    }

    response_data
}

async fn encrypt_response(client_context: &client::ClientContext, request_id: String, response_data: common::ResponseData) -> Bytes {
    // debug!("Response Data: {:?}", response_data);
    let response_data_json = serde_json::to_string(&response_data).unwrap();
    log::debug!("Response Data JSON: {:?}", response_data_json);
    let encrypted_response = client_context.message_encryptor.encrypt(
        &client_context.server_public_key, &response_data_json);
    let result = [request_id.as_bytes().to_vec(), encrypted_response].concat();
    Bytes::from(result)
}

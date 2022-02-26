use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SecureEnvelop {
    pub encoded_public_key: String,
    pub nonce: String,
    pub encrypted_payload: String
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestData {
    pub request_id: String,
    pub path: String,
    pub query_pairs: Vec<(String, String)>,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub content_type: String,
    pub body: Option<String>
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseData {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub content_type: String,
    pub body: Option<String>
}

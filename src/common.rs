use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SecureEnvelop {
    pub data_id: String,
    pub encoded_public_key: String,
    pub nonce: String,
    pub encrypted_payload: String
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestData {
    pub url: String,
    pub method: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseData {
    pub status: i16,
    pub headers: HashMap<String, String>,
    pub body: Option<String>
}

use std::collections::HashMap;
use std::fmt;
use chrono;
use qstring::QString;
use serde::{Deserialize, Serialize};

pub fn print_banner(mode: &str) {
    // We don't need this as a constant because it will be shown only once.
    let banner: &str = "\n\n\
\x20  88888888ba                              ad888888b,  88b           d88\n\
\x20  88      '8b                            d8'     '88  888b         d888\n\
\x20  88      ,8P                                    a8P  88`8b       d8'88\n\
\x20  88aaaaaa8P'  88       88  8b,dPPYba,        ,d8P'   88 `8b     d8' 88   ,adPPYba,\n\
\x20  88'''''88'    88       88  88P'   `'8a     a8P'      88  `8b   d8'  88  a8P_____88\n\
\x20  88    `8b    88       88  88       88   a8P'        88   `8b d8'   88  8PP'''''''\n\
\x20  88     `8b   '8a,   ,a88  88       88  d8'          88    `888'    88  `8b,   ,aa\n\
\x20  88      `8b   `'YbbdP'Y8  88       88  88888888888  88     `8'     88   `'Ybbd8'\n\
\x20  ==================================================================================\n";
    println!("{}\x20  Mode: {}\n\n", banner, mode);
}

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

impl fmt::Display for RequestData {

    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "==================== REQUEST ====================")?;
        writeln!(f, "Received at {}", chrono::offset::Local::now())?;
        writeln!(f, "Request ID: {}", self.request_id)?;
        writeln!(f, "URI: {}?{}", self.path, QString::new(self.query_pairs.clone()).to_string())?;
        writeln!(f, "Method: {}", self.method)?;
        writeln!(f, "Headers:")?;
        for (k, v) in self.headers.iter() {
            writeln!(f, " - {} = {}", k, v)?;
        }
        if self.body.is_some() {
            writeln!(f, "Body: \n{:?}", self.body)?
        }
        write!(f, "")
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseData {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub content_type: String,
    pub body: Option<String>
}


impl fmt::Display for ResponseData {

    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "==================== RESPONSE ====================")?;
        writeln!(f, "Responded at {}", chrono::offset::Local::now())?;
        writeln!(f, "Status: {}", self.status)?;
        writeln!(f, "Headers:")?;
        for (k, v) in self.headers.iter() {
            writeln!(f, " - {} = {}", k, v)?;
        }
        if self.body.is_some() {
            writeln!(f, "Body: \n{:?}", self.body)?
        }
        write!(f, "")
    }
}

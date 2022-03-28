
pub mod encryption;

use std::collections::HashMap;
use std::fmt;
use chrono;
use qstring::QString;
use serde::{Deserialize, Serialize};

pub const HEADER_PUBLIC_KEY: &str = "X-Public-Key";

pub const HEADER_USERNAME: &str = "X-Username";

pub const HEADER_PASSWORD: &str = "X-Password";

pub const HEADER_REQUEST_ID: &str = "X-Request-ID";

pub const HEADER_AUTHORIZATION: &str = "Authorization";

pub const RESPONSE_404: &str = "{\"status\": 404, \"headers\": {}, \"content_type\": \"text/plain\"}";

pub fn print_banner(mode: &str) {
    // We don't need this as a constant because it will be shown only once.
    let banner: &str = "\n\n\

\x20                _____
\x20             \\'_   _'/
\x20              |(>)-(<)|
\x20           ../   εO϶   \\..
\x20  -------''(((:-.,_,.-:)))''--------
\x20  ==================================\n";
    println!("{}\x20  Mode: {}\n\n", banner, mode);
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

pub fn get_response_404() -> Vec<u8> {
    RESPONSE_404.as_bytes().to_vec()
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

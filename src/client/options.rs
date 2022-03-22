use clap::Parser;

// TODO: support subdomain
/// The open source secure introspectable tunnels to localhost.
#[derive(Parser, Debug, Clone)]
#[clap(author, version, about, long_about = None)]
pub struct ClientOptions {
    /// A custom client_id. If omitted, UUID v4 will be used.
    #[clap(short, long, default_value = "")]
    pub client_id: String,

    /// The port number that the traffic will be routed to.
    #[clap(short = 'p', long)]
    pub target_port: u16,

    /// The server host
    #[clap(short, long, default_value = "http://localhost:8080")]
    pub server_host: String,

    /// Timeout for the individual reads of the socket in seconds
    #[clap(long, default_value = "30")]
    pub timeout_read: u64,

    /// Timeout for the individual writes of the socket in seconds
    #[clap(long, default_value = "30")]
    pub timeout_write: u64,

    /// Show the request/response in the console
    #[clap(short, long)]
    pub debug: bool

}

pub fn parse_options() -> ClientOptions {
    Parser::parse()
}


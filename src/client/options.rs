use std::env;
use clap::Parser;
use uuid::Uuid;

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

    /// The server host and port
    #[clap(short = 'h', long, default_value = "http://localhost:8080")]
    pub server_host: String,

    /// The username
    #[clap(long, default_value = "secret")]
    pub username: String,

    /// The password
    #[clap(long, default_value = "")]
    pub password: String,

    /// Timeout for the individual reads of the socket in seconds
    #[clap(long, default_value = "30")]
    pub timeout_read: u64,

    /// Timeout for the individual writes of the socket in seconds
    #[clap(long, default_value = "30")]
    pub timeout_write: u64,

    /// Show the request/response in the console
    #[clap(long)]
    pub debug: bool


}

const ENV_SERVER_HOST: &str = "MOLE_SERVER_HOST";

const ENV_CLIENT_ID: &str = "MOLE_CLIENT_ID";

const ENV_USERNAME: &str = "MOLE_USERNAME";

const ENV_PASSWORD: &str = "MOLE_PASSWORD";

pub fn parse_options() -> ClientOptions {

    let mut options: ClientOptions = Parser::parse();

    let args: Vec<String> = env::args().collect();

    env::var(ENV_SERVER_HOST).into_iter().for_each(|host| {
        // Override the server host with ENV variable only when the default value is used.
        if options.server_host == "http://localhost:8080" {
            options.server_host = host;
        }
    });
    env::var(ENV_CLIENT_ID).into_iter().for_each(|client_id| {
        if options.client_id == "" {
            options.client_id = client_id;
        }
    });
    if options.client_id == "" {
        options.client_id = Uuid::new_v4().to_string().replace("-", "")
    }
    env::var(ENV_USERNAME).into_iter().for_each(|username| {
        if options.username == "secret" {
            options.username = username;
        }
    });
    env::var(ENV_PASSWORD).into_iter().for_each(|password| {
        if options.password == "" {
            options.password = password;
        }
    });
    options
}


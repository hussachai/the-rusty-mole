use clap::Parser;

/// The open source secure introspectable tunnels to localhost.
#[derive(Clone, Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct ServerOptions {
    /// A bind address. The default value is 0.0.0.0
    #[clap(short, long, default_value = "0.0.0.0")]
    pub bind_addr: String,
    /// A Rabbit MQ URI. The default value amqp://127.0.0.1:5672/%2f
    #[clap(short, long, default_value = "amqp://127.0.0.1:5672/%2f")]
    pub amqp_uri: String,
    /// The port number that the server will listen to the client. The default value is 8080
    #[clap(short, long, default_value = "8080")]
    pub port: u16,
    /// Provides this flag to enforce every client to have a password set. Otherwise, the connection will be rejected.
    #[clap(long)]
    pub password_required: bool,

}


pub fn parse_options() -> ServerOptions {
    Parser::parse()
}


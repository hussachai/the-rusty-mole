use std::str::from_utf8;
use std::time::Instant;

use actix::prelude::*;
use actix_web_actors::ws;
use deadpool_lapin::lapin::{BasicProperties, Channel};
use deadpool_lapin::lapin::options::{BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, QueueDeleteOptions};
use deadpool_lapin::lapin::types::FieldTable;
use futures_lite::stream::StreamExt;

use crate::common::SecureEnvelop;
use crate::common::encryption::MessageEncryptor;
use crate::server;

/// websocket connection is long running connection, it easier
/// to handle with an actor
pub struct MyWebSocket {
    /// Client must send ping at least once per 10 seconds (CLIENT_TIMEOUT),
    /// otherwise we drop connection.
    pub hb: Instant,
    pub client_id: String,
    pub message_encryptor: MessageEncryptor,
    pub client_public_key: String,
    pub channel: Channel,
    pub queue_req_name: String,
    pub queue_res_name: String
}

impl Handler<server::TextMessage> for MyWebSocket {
    type Result = ();

    fn handle(&mut self, msg: server::TextMessage, ctx: &mut Self::Context) {
        ctx.text(msg.text);
    }
}

impl Actor for MyWebSocket {

    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start. We start the heartbeat process here.
    fn started(&mut self, ctx: &mut Self::Context) {

        let queue_req = self.queue_req_name.clone();
        let channel = self.channel.clone();
        let message_encryptor = self.message_encryptor.clone();
        let client_public_key = self.client_public_key.clone();

        let self_addr = ctx.address().clone();

        async_global_executor::spawn(async move {

            let mut consumer = channel
                .basic_consume(
                    &queue_req,
                    "my_consumer",
                    BasicConsumeOptions::default(),
                    FieldTable::default(),
                ).await.unwrap();
            while let Some(next_result) = consumer.next().await {
                match next_result {
                    Ok((_, delivery)) => {
                        let request_data = from_utf8(&delivery.data).unwrap();
                        let nonce = message_encryptor.generate_nonce();
                        let encrypted_payload = message_encryptor.encrypt_as_text(&client_public_key, &nonce, request_data);
                        let server_public_key = message_encryptor.encoded_public_key();
                        let secure_envelop = SecureEnvelop {
                            encoded_public_key: server_public_key,
                            nonce,
                            encrypted_payload,
                        };

                        let serialized_secure_envelop = serde_json::to_string(&secure_envelop).unwrap();

                        self_addr.do_send(server::TextMessage {text: serialized_secure_envelop });

                        debug!("Consumed: {:?}", request_data);

                        let ack_result = delivery.ack(BasicAckOptions::default()).await;
                        if ack_result.is_err() {
                            error!("Ack failed. The message will be retried again!");
                        }
                    }
                    Err(e) =>
                        error!("Could not consume the message due to {:?}", e)
                }
            }
        }).detach();

        self.hb(ctx);
    }

    // I think it might be ok to do a clean up here even we have a cluster of this,
    // the WS connection between LB and the client should be sticky.
    fn stopped(&mut self, _: &mut Self::Context) {
        let queue_req = format!("{}_req", self.client_id);
        let queue_res = format!("{}_res", self.client_id);
        let _ = self.channel.queue_delete(&queue_req, QueueDeleteOptions::default());
        let _ = self.channel.queue_delete(&queue_res, QueueDeleteOptions::default());
    }
}

/// Handler for `ws::Message`
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for MyWebSocket {

    fn handle(
        &mut self,
        msg: Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {
        let queue_res = self.queue_res_name.clone();
        let channel = self.channel.clone();

        // process websocket messages
        // println!("WS: {:?}", msg);
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.hb = Instant::now();
            }
            Ok(ws::Message::Text(text)) => {
                // substring for the UUID?
                let message_id = &text[0..36];
                let encrypted_response = text[36..].to_string().as_bytes().to_vec();
                let properties = BasicProperties::default().with_message_id(message_id.into());

                let _ = channel.basic_publish(
                    "",
                    &queue_res,
                    BasicPublishOptions::default(),
                    encrypted_response,
                    properties,
                );
                debug!("Response data: {}", text);
            },
            Ok(ws::Message::Binary(bin)) => ctx.binary(bin),
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => ctx.stop(),
        }
    }
}

impl MyWebSocket {

    pub fn new(client_id: String,
           message_encryptor: MessageEncryptor,
           client_public_key: String,
           channel: Channel,
           queue_req_name: String,
           queue_res_name: String) -> Self {

        Self { hb: Instant::now(), client_id, message_encryptor, client_public_key, channel, queue_req_name, queue_res_name }
    }

    /// helper method that sends ping to client every second.
    ///
    /// also this method checks heartbeats from client
    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(server::HEARTBEAT_INTERVAL, |act, ctx| {
            // check client heartbeats
            if Instant::now().duration_since(act.hb) > server::CLIENT_TIMEOUT {
                // heartbeat timed out
                println!("Websocket Client heartbeat failed, disconnecting!");

                // stop actor
                ctx.stop();

                // don't try to send a ping
                return;
            }

            ctx.ping(b"");

        });
    }
}

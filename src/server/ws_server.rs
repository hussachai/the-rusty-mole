use std::ops::Deref;
use std::str::from_utf8;
use std::time::Instant;

use actix::prelude::*;
use actix_web::web::Bytes;
use actix_web_actors::ws;
use deadpool_lapin::lapin::{BasicProperties, Channel, Consumer, PromiseChain};
use deadpool_lapin::lapin::message::Delivery;
use deadpool_lapin::lapin::options::{BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, QueueDeleteOptions};
use deadpool_lapin::lapin::publisher_confirm::PublisherConfirm;
use deadpool_lapin::lapin::types::{AMQPValue, FieldTable};
use futures_lite::stream::StreamExt;

use crate::common::encryption::MessageEncryptor;
use crate::{common, server};

/// websocket connection is long running connection, it easier
/// to handle with an actor
#[derive(Clone)]
pub struct ChannelContext {
    /// Client must send ping at least once per 10 seconds (CLIENT_TIMEOUT),
    /// otherwise we drop connection.
    pub hb: Instant,
    pub client_id: String,
    pub credentials: Option<String>,
    pub message_encryptor: MessageEncryptor,
    pub client_public_key: String,
    pub channel: Channel,
    pub queue_req_name: String,
    pub queue_res_name: String
}

impl Handler<server::Payload> for ChannelContext {
    type Result = ();

    fn handle(&mut self, payload: server::Payload, ctx: &mut Self::Context) {
        ctx.binary(payload.data);
    }
}

impl Handler<server::ServerMessage> for ChannelContext {
    type Result = ();

    fn handle(&mut self, msg: server::ServerMessage, ctx: &mut Self::Context) {
        ctx.text(msg.message);
    }
}

impl Actor for ChannelContext {

    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start. We start the heartbeat process here.
    fn started(&mut self, ctx: &mut Self::Context) {

        let self_addr = ctx.address().clone();

        let mut channel_context = self.deref().clone();

        async_global_executor::spawn(async move {

            let mut consumer = channel_context.consume().await.unwrap();

            while let Some(next_result) = consumer.next().await {
                match next_result {
                    Ok((_, delivery)) => {
                        if let Some(user_and_pass) = channel_context.credentials.as_deref() {
                            let headers = delivery.properties.headers().as_ref().unwrap().inner();
                            let client_ip = if let Some(AMQPValue::LongString(ip_addr)) = headers.get(common::HEADER_REQUEST_IP) {
                                ip_addr.to_string()
                            } else {
                                "Unknown".to_string()
                            };
                            match (headers.get(common::HEADER_AUTHORIZATION), headers.get(common::HEADER_REQUEST_ID)) {
                                (Some(AMQPValue::LongString(provided_user_and_pass)), Some(AMQPValue::LongString(message_id))) => {
                                    if user_and_pass != provided_user_and_pass.to_string() {
                                        let bad_credentials_res = common::get_response_404();
                                        channel_context.publish(message_id.as_str(), bad_credentials_res);
                                        let message = format!("{} is trying to access with the wrong credentials!", client_ip);
                                        self_addr.do_send(server::ServerMessage {message});
                                        channel_context.ack(&delivery).await;
                                        continue;
                                    }
                                },
                                _ => {
                                    let message = format!("{} is trying to access with no credentials!", client_ip);
                                    self_addr.do_send(server::ServerMessage {message});
                                    channel_context.ack(&delivery).await;
                                    continue;
                                },
                            }
                        }

                        let request_data = from_utf8(&delivery.data).unwrap();
                        log::debug!("Consumed: {:?}", request_data);
                        let encrypted_request_data = channel_context.encrypt_data(request_data);

                        self_addr.do_send(server::Payload {data: Bytes::from(encrypted_request_data)});

                        channel_context.ack(&delivery).await;
                    }
                    Err(e) =>
                        log::error!("Could not consume the message due to {:?}", e)
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
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ChannelContext {

    fn handle(
        &mut self,
        msg: Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {

        let mut channel_context = self.deref().clone();

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
                log::debug!("Response data: {}", text);
            },
            Ok(ws::Message::Binary(data)) => {
                // substring for the UUID?
                let message_id = String::from_utf8(data[0..36].to_vec()).unwrap();
                log::debug!("Received message ID: {} and it will be put to queue: {}", message_id, channel_context.queue_res_name);
                let encrypted_response = data[36..].to_vec();
                let response_data_json = channel_context.decrypt_data(encrypted_response);
                channel_context.publish(message_id.as_str(), response_data_json);
            },
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => ctx.stop(),
        }
    }
}

impl ChannelContext {

    pub fn new(client_id: String,
               credentials: Option<String>,
               message_encryptor: MessageEncryptor,
               client_public_key: String,
               channel: Channel,
               queue_req_name: String,
               queue_res_name: String) -> Self {

        Self { hb: Instant::now(), client_id, credentials, message_encryptor, client_public_key, channel, queue_req_name, queue_res_name }
    }

    fn decrypt_data(&mut self, cipher: Vec<u8>) -> Vec<u8> {
        self.message_encryptor.decrypt(&self.client_public_key, cipher)
    }

    fn encrypt_data(&mut self, request_data: &str) -> Vec<u8> {
        self.message_encryptor.encrypt(&self.client_public_key, request_data)
    }

    fn consume(&mut self) -> PromiseChain<Consumer> {
        self.channel
            .basic_consume(
                &self.queue_req_name,
                "my_consumer",
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
    }

    fn publish(&mut self, message_id: &str, payload: Vec<u8>) -> PromiseChain<PublisherConfirm> {
        let properties = BasicProperties::default().with_message_id(message_id.into());
        self.channel.basic_publish(
            "",
            &self.queue_res_name,
            BasicPublishOptions::default(),
            payload,
            properties,
        )
    }

    async fn ack(&mut self, delivery: &Delivery) {
        let ack_result = delivery.ack(BasicAckOptions::default()).await;
        if ack_result.is_err() {
            log::error!("Ack failed. The message will be retried again!");
        }
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

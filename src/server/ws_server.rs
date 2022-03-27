use std::str::from_utf8;
use std::time::Instant;

use actix::prelude::*;
use actix_web::web::Bytes;
use actix_web_actors::ws;
use deadpool_lapin::lapin::{BasicProperties, Channel};
use deadpool_lapin::lapin::options::{BasicAckOptions, BasicConsumeOptions, BasicPublishOptions, QueueDeleteOptions};
use deadpool_lapin::lapin::types::{AMQPValue, FieldTable};
use futures_lite::stream::StreamExt;

use crate::common::encryption::MessageEncryptor;
use crate::{common, server};

/// websocket connection is long running connection, it easier
/// to handle with an actor
pub struct MyWebSocket {
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

impl Handler<server::Payload> for MyWebSocket {
    type Result = ();

    fn handle(&mut self, msg: server::Payload, ctx: &mut Self::Context) {
        ctx.binary(msg.data);
    }
}

impl Actor for MyWebSocket {

    type Context = ws::WebsocketContext<Self>;

    /// Method is called on actor start. We start the heartbeat process here.
    fn started(&mut self, ctx: &mut Self::Context) {

        let queue_req = self.queue_req_name.clone();
        let queue_res = self.queue_res_name.clone();
        let channel = self.channel.clone();
        let message_encryptor = self.message_encryptor.clone();
        let client_public_key = self.client_public_key.clone();
        let credentials = self.credentials.clone();
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
                        if let Some(user_and_pass) = credentials.as_deref() {
                            let headers = delivery.properties.headers().as_ref().unwrap().inner();
                            match (headers.get(common::HEADER_AUTHORIZATION), headers.get(common::HEADER_REQUEST_ID)) {
                                (Some(AMQPValue::LongString(provided_user_and_pass)), Some(AMQPValue::LongString(message_id))) => {
                                    println!(">>>>>> {:?}", provided_user_and_pass.to_string());
                                    println!(">>>##.. {}", user_and_pass);
                                    if user_and_pass != provided_user_and_pass.to_string() {
                                        let properties = BasicProperties::default().with_message_id(message_id.to_string().into());
                                        let bad_credentials_res = "{\"status\": 404, \"headers\": {}, \"content_type\": \"text/plain\"}".as_bytes().to_vec();
                                        let _ = channel.basic_publish(
                                            "",
                                            &queue_res,
                                            BasicPublishOptions::default(),
                                            bad_credentials_res,
                                            properties,
                                        );
                                        println!("Bad credentials!!!");
                                        let ack_result = delivery.ack(BasicAckOptions::default()).await;
                                        if ack_result.is_err() {
                                            error!("Ack failed. The message will be retried again!");
                                        }
                                        continue;
                                    }
                                },
                                _ => println!("ERROR"),
                            }
                        }

                        let request_data = from_utf8(&delivery.data).unwrap();
                        debug!("Consumed: {:?}", request_data);
                        let encrypted_request_data = message_encryptor.encrypt(&client_public_key, request_data);
                        self_addr.do_send(server::Payload {data: Bytes::from(encrypted_request_data)});
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
                debug!("Response data: {}", text);
            },
            Ok(ws::Message::Binary(data)) => {
                // substring for the UUID?
                let message_id = String::from_utf8(data[0..36].to_vec()).unwrap();
                debug!("Received message ID: {} and it will be put to queue: {}", message_id, queue_res);
                let encrypted_response = data[36..].to_vec();

                let properties = BasicProperties::default().with_message_id(message_id.into());

                let response_data_json = self.message_encryptor.decrypt(
                    &self.client_public_key, encrypted_response);

                let _ = channel.basic_publish(
                    "",
                    &queue_res,
                    BasicPublishOptions::default(),
                    response_data_json,
                    properties,
                );
            },
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
               credentials: Option<String>,
               message_encryptor: MessageEncryptor,
               client_public_key: String,
               channel: Channel,
               queue_req_name: String,
               queue_res_name: String) -> Self {

        Self { hb: Instant::now(), client_id, credentials, message_encryptor, client_public_key, channel, queue_req_name, queue_res_name }
    }

    /// helper method that sends ping to client every second.
    ///
    /// also this method checks heartbeats from client
    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(server::HEARTBEAT_INTERVAL, |act, ctx| {
            // check client heartbeats
            // if Instant::now().duration_since(act.hb) > server::CLIENT_TIMEOUT {
            //     // heartbeat timed out
            //     println!("Websocket Client heartbeat failed, disconnecting!");
            //
            //     // stop actor
            //     ctx.stop();
            //
            //     // don't try to send a ping
            //     return;
            // }

            ctx.ping(b"");

        });
    }
}

use actix_web::{guard, web, middleware, HttpResponse, App, HttpServer, HttpRequest};
use std::net::TcpListener;
use crate::message_relayer::common::web_request::{Message, Messages};
use tokio::sync::mpsc::UnboundedSender;

const HEADER_TOKEN: &str = "X-Token";

async fn relay_messages(
    request: HttpRequest,
    messages: web::Json<Messages>,
    secret: web::Data<String>,
    channel: web::Data<UnboundedSender<Message>>,
) -> HttpResponse {
    if !request.headers().get(HEADER_TOKEN)
        .and_then(|h| h.to_str().ok())
        .map(|t| t == secret.get_ref())
        .unwrap_or(false)
    {
        return HttpResponse::Unauthorized().finish();
    }

    let messages = messages.into_inner().messages;
    let len = messages.len();
    let mut to_process = len;

    for message in messages {
        match channel.send(message.clone()) {
            Ok(_) => to_process -= 1,

            Err(e) => {
                log::error!(r#"Unable to send message "{message:?}": {e:?}"#);
            }
        }
    }

    if to_process == 0 {
        HttpResponse::Ok().finish()
    } else if to_process == len {
        HttpResponse::InternalServerError().finish()
    } else {
        HttpResponse::Accepted().finish()
    }
}

pub fn create(
    tcp_listener: TcpListener,
    secret: String,
    channel: UnboundedSender<Message>,
) -> std::io::Result<actix_web::dev::Server> {
    let channel = web::Data::new(channel);
    let secret = web::Data::new(secret);

    let server = HttpServer::new(move || {
        App::new()
            .app_data(secret.clone())
            .app_data(channel.clone())
            // enable logger
            .wrap(middleware::Logger::default())
            .app_data(
                // 128 KiB
                web::JsonConfig::default().limit(131_072)
            )
            .service(
                web::resource("/relay_messages")
                    .route(web::post().to(relay_messages))
                    .route(
                        web::route()
                            .guard(guard::Any(guard::Get()).or(guard::Post()))
                            .to(|| HttpResponse::Unauthorized()),
                    ),

            )
    });

    let server = server.listen(tcp_listener);

    Ok(server?.disable_signals().run())
}

#[cfg(test)]
mod tests {
    use reqwest::{ClientBuilder, header::{CONTENT_TYPE, HeaderValue}};
    use std::{time::Duration, str::FromStr};
    use ethereum_common::U256;
    use tokio::{sync::mpsc, task};
    use super::*;

    #[tokio::test]
    async fn test_server() {
        let _ = pretty_env_logger::formatted_timed_builder()
            .filter_level(log::LevelFilter::Debug)
            .format_target(false)
            .format_timestamp_secs()
            .parse_default_env()
            .try_init();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        const SECRET: &str = "SECRET123";
        let (channel, mut receiver) = mpsc::unbounded_channel();
        let server = super::create(listener, SECRET.to_string(), channel).unwrap();

        task::spawn(server);

        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();

        let url = format!("http://127.0.0.1:{port}/relay_messages");
        let response = client.post(&url).json(&Messages::default()).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

        assert!(receiver.try_recv().is_err());

        let response = client.post(&url).json(&Messages::default())
            .header(HEADER_TOKEN, SECRET)
            .send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        assert!(receiver.try_recv().is_err());

        //
        let nonce_string = "0x0123";
        let block = 2_111_333;
        let body = format!(r#"{{"messages":[{{"block":{block},"nonce":"{nonce_string}"}}]}}"#);
        let response = client.post(&url).body(body)
            .header(HEADER_TOKEN, SECRET)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let message_received = receiver.recv().await.unwrap();
        assert_eq!(block, message_received.block);
        assert_eq!(U256::from_str(nonce_string).unwrap(), message_received.nonce);

        assert!(receiver.try_recv().is_err());

        let message = Message { block: 1_222_333, nonce: 123.into(), };
        let messages = {
            let mut messages = Messages::default();
            messages.messages.push(message.clone());

            messages
        };

        let response = client.post(&url).json(&messages)
            .header(HEADER_TOKEN, SECRET)
            .send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let message_received = receiver.recv().await.unwrap();
        assert_eq!(message.block, message_received.block);
        assert_eq!(message.nonce, message_received.nonce);

        assert!(receiver.try_recv().is_err());

        receiver.close();

        let response = client.post(&url).json(&messages)
            .header(HEADER_TOKEN, SECRET)
            .send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);
    }
}

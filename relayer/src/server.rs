use crate::message_relayer::common::web_request::{
    MerkleRootBlocks, MerkleRootsRequest, Message, Messages,
};
use actix_web::{guard, middleware, web, App, HttpRequest, HttpResponse, HttpServer};
use futures::{stream::FuturesUnordered, StreamExt};
use std::{collections::HashSet, net::TcpListener};
use tokio::sync::mpsc::UnboundedSender;

const HEADER_TOKEN: &str = "X-Token";

struct Secret(String);

struct LogContext(String);

async fn relay_messages(
    request: HttpRequest,
    messages: web::Json<Messages>,
    secret: web::Data<Secret>,
    log_context: web::Data<LogContext>,
    channel: web::Data<UnboundedSender<Message>>,
) -> HttpResponse {
    if !request
        .headers()
        .get(HEADER_TOKEN)
        .and_then(|h| h.to_str().ok())
        .map(|t| t == secret.0.as_str())
        .unwrap_or(false)
    {
        return HttpResponse::Unauthorized().finish();
    }

    let messages = messages.into_inner().messages;
    let len = messages.len();
    let mut to_process = len;

    // Avoid enqueuing duplicates within the same request.
    let mut seen = HashSet::with_capacity(len);

    for message in messages {
        if !seen.insert((message.block, message.nonce)) {
            // Duplicate within this batch; treat as successfully handled.
            to_process -= 1;
            continue;
        }
        match channel.send(message.clone()) {
            Ok(_) => to_process -= 1,

            Err(e) => {
                log::error!(
                    r#"[{}] Unable to send message "{message:?}": {e:?}"#,
                    log_context.0.as_str()
                );
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

async fn get_merkle_root_proof(
    request: HttpRequest,
    blocks: web::Json<MerkleRootBlocks>,
    secret: web::Data<Secret>,
    log_context: web::Data<LogContext>,
    channel: web::Data<UnboundedSender<MerkleRootsRequest>>,
) -> HttpResponse {
    if !request
        .headers()
        .get(HEADER_TOKEN)
        .and_then(|h| h.to_str().ok())
        .map(|t| t == secret.0.as_str())
        .unwrap_or(false)
    {
        return HttpResponse::Unauthorized().finish();
    }

    let blocks = blocks.into_inner().blocks;
    let len = blocks.len();

    let mut futures = FuturesUnordered::new();
    let mut failed_sends = 0;

    for block in blocks {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let request = MerkleRootsRequest::GetMerkleRootProof {
            block_number: block,
            response: sender,
        };

        if channel.send(request).is_err() {
            log::error!(
                "[{}] Unable to send merkle root proof request for block {block}",
                log_context.0.as_str()
            );
            failed_sends += 1;
            continue;
        }

        futures.push(receiver);
    }
    let mut to_process = failed_sends + futures.len();

    let mut merkle_roots = Vec::new();

    while let Some(result) = futures.next().await {
        match result {
            Ok(response) => {
                merkle_roots.push(response);
                to_process -= 1;
            }
            Err(e) => {
                log::error!(
                    "[{}] Unable to receive merkle root proof response: {e:?}",
                    log_context.0.as_str()
                );
            }
        }
    }

    if to_process == 0 {
        HttpResponse::Ok().json(merkle_roots)
    } else if to_process == len {
        HttpResponse::InternalServerError().finish()
    } else {
        HttpResponse::Accepted().json(merkle_roots)
    }
}

pub fn create(
    tcp_listener: TcpListener,
    secret: String,
    log_context: String,
    messages_channel: Option<UnboundedSender<Message>>,
    merkle_roots_channel: Option<UnboundedSender<MerkleRootsRequest>>,
) -> std::io::Result<actix_web::dev::Server> {
    let messages_channel = messages_channel.map(web::Data::new);
    let merkle_roots_channel = merkle_roots_channel.map(web::Data::new);

    let secret = web::Data::new(Secret(secret));
    let log_context = web::Data::new(LogContext(log_context));

    let server = HttpServer::new(move || {
        let mut app = App::new()
            .app_data(secret.clone())
            .app_data(log_context.clone())
            // enable logger
            .wrap(middleware::Logger::default())
            .app_data(
                // 128 KiB
                web::JsonConfig::default().limit(131_072),
            );
        app = if let Some(channel) = messages_channel.as_ref() {
            app.app_data(channel.clone()).service(
                web::resource("/relay_messages")
                    .route(web::post().to(relay_messages))
                    .route(
                        web::route()
                            .guard(guard::Any(guard::Get()).or(guard::Post()))
                            .to(HttpResponse::Unauthorized),
                    ),
            )
        } else {
            app
        };

        app = if let Some(channel) = merkle_roots_channel.as_ref() {
            app.app_data(channel.clone()).service(
                web::resource("/get_merkle_root_proof")
                    .route(web::post().to(get_merkle_root_proof))
                    .route(
                        web::route()
                            .guard(guard::Any(guard::Get()).or(guard::Post()))
                            .to(HttpResponse::Unauthorized),
                    ),
            )
        } else {
            app
        };

        app
    });

    let server = server.listen(tcp_listener);

    Ok(server?.disable_signals().run())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message_relayer::common::web_request::MerkleRootsResponse;
    use ethereum_common::U256;
    use reqwest::{
        header::{HeaderValue, CONTENT_TYPE},
        ClientBuilder,
    };
    use std::{str::FromStr, time::Duration};
    use tokio::{sync::mpsc, task};

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

        let server = super::create(
            listener,
            SECRET.to_string(),
            "test".to_string(),
            Some(channel),
            None,
        )
        .unwrap();

        task::spawn(server);

        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();

        let url = format!("http://127.0.0.1:{port}/relay_messages");
        let response = client
            .post(&url)
            .json(&Messages::default())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

        assert!(receiver.try_recv().is_err());

        let response = client
            .post(&url)
            .json(&Messages::default())
            .header(HEADER_TOKEN, SECRET)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        assert!(receiver.try_recv().is_err());

        //
        let nonce_string = "0x0123";
        let block = 2_111_333;
        let body = format!(r#"{{"messages":[{{"block":{block},"nonce":"{nonce_string}"}}]}}"#);
        let response = client
            .post(&url)
            .body(body)
            .header(HEADER_TOKEN, SECRET)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let message_received = receiver.recv().await.unwrap();
        assert_eq!(block, message_received.block);
        assert_eq!(
            U256::from_str(nonce_string).unwrap().0,
            message_received.nonce.0
        );

        assert!(receiver.try_recv().is_err());

        let message = Message {
            block: 1_222_333,
            nonce: 123.into(),
        };
        let messages = {
            let mut messages = Messages::default();
            messages.messages.push(message.clone());

            messages
        };

        let response = client
            .post(&url)
            .json(&messages)
            .header(HEADER_TOKEN, SECRET)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let message_received = receiver.recv().await.unwrap();
        assert_eq!(message.block, message_received.block);
        assert_eq!(message.nonce, message_received.nonce);

        assert!(receiver.try_recv().is_err());

        receiver.close();

        let response = client
            .post(&url)
            .json(&messages)
            .header(HEADER_TOKEN, SECRET)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_merkle_root_proof_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        const SECRET: &str = "SECRET123";
        let (channel, mut receiver) = mpsc::unbounded_channel();

        let server = super::create(
            listener,
            SECRET.to_string(),
            "test".to_string(),
            None,
            Some(channel),
        )
        .unwrap();

        task::spawn(server);

        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();

        let url = format!("http://127.0.0.1:{port}/get_merkle_root_proof");
        let body = MerkleRootBlocks { blocks: vec![123] };

        let response = client.post(&url).json(&body).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

        let client_for_request = client.clone();
        let url_for_request = url.clone();
        let request = task::spawn(async move {
            client_for_request
                .post(&url_for_request)
                .json(&body)
                .header(HEADER_TOKEN, SECRET)
                .send()
                .await
                .unwrap()
        });

        let request_received = receiver.recv().await.unwrap();
        match request_received {
            MerkleRootsRequest::GetMerkleRootProof {
                block_number,
                response,
            } => {
                assert_eq!(block_number, 123);
                response
                    .send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number })
                    .unwrap();
            }
        }

        let response = request.await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body = response.json::<Vec<MerkleRootsResponse>>().await.unwrap();
        assert_eq!(
            body,
            vec![MerkleRootsResponse::NoMerkleRootOnBlock { block_number: 123 }]
        );
    }

    #[tokio::test]
    async fn test_merkle_root_proof_separate_listeners_route_independently() {
        const SECRET_A: &str = "SECRET_A";
        const SECRET_B: &str = "SECRET_B";

        let (url_a, mut receiver_a) = spawn_merkle_root_server(SECRET_A, "relayer-a");
        let (url_b, mut receiver_b) = spawn_merkle_root_server(SECRET_B, "relayer-b");

        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();

        let request_a = {
            let client = client.clone();
            let url = url_a.clone();
            task::spawn(async move {
                client
                    .post(&url)
                    .json(&MerkleRootBlocks { blocks: vec![111] })
                    .header(HEADER_TOKEN, SECRET_A)
                    .send()
                    .await
                    .unwrap()
            })
        };

        let request_received = receiver_a.recv().await.unwrap();
        match request_received {
            MerkleRootsRequest::GetMerkleRootProof {
                block_number,
                response,
            } => {
                assert_eq!(block_number, 111);
                response
                    .send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number })
                    .unwrap();
            }
        }
        assert!(receiver_b.try_recv().is_err());
        assert_eq!(request_a.await.unwrap().status(), reqwest::StatusCode::OK);

        let request_b = {
            let client = client.clone();
            let url = url_b.clone();
            task::spawn(async move {
                client
                    .post(&url)
                    .json(&MerkleRootBlocks { blocks: vec![222] })
                    .header(HEADER_TOKEN, SECRET_B)
                    .send()
                    .await
                    .unwrap()
            })
        };

        let request_received = receiver_b.recv().await.unwrap();
        match request_received {
            MerkleRootsRequest::GetMerkleRootProof {
                block_number,
                response,
            } => {
                assert_eq!(block_number, 222);
                response
                    .send(MerkleRootsResponse::NoMerkleRootOnBlock { block_number })
                    .unwrap();
            }
        }
        assert!(receiver_a.try_recv().is_err());
        assert_eq!(request_b.await.unwrap().status(), reqwest::StatusCode::OK);
    }

    #[tokio::test]
    async fn test_merkle_root_proof_closed_channel_is_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        const SECRET: &str = "SECRET123";
        let (channel, receiver) = mpsc::unbounded_channel();
        drop(receiver);

        let server = super::create(
            listener,
            SECRET.to_string(),
            "test".to_string(),
            None,
            Some(channel),
        )
        .unwrap();

        task::spawn(server);

        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();

        let url = format!("http://127.0.0.1:{port}/get_merkle_root_proof");
        let response = client
            .post(&url)
            .json(&MerkleRootBlocks { blocks: vec![123] })
            .header(HEADER_TOKEN, SECRET)
            .send()
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn test_merkle_root_proof_dropped_response_is_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        const SECRET: &str = "SECRET123";
        let (channel, mut receiver) = mpsc::unbounded_channel();

        let server = super::create(
            listener,
            SECRET.to_string(),
            "test".to_string(),
            None,
            Some(channel),
        )
        .unwrap();

        task::spawn(server);

        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();

        let url = format!("http://127.0.0.1:{port}/get_merkle_root_proof");
        let body = MerkleRootBlocks { blocks: vec![123] };

        let request = task::spawn(async move {
            client
                .post(&url)
                .json(&body)
                .header(HEADER_TOKEN, SECRET)
                .send()
                .await
                .unwrap()
        });

        let request_received = receiver.recv().await.unwrap();
        match request_received {
            MerkleRootsRequest::GetMerkleRootProof {
                block_number,
                response,
            } => {
                assert_eq!(block_number, 123);
                drop(response);
            }
        }

        let response = request.await.unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    fn spawn_merkle_root_server(
        secret: &str,
        log_context: &str,
    ) -> (String, mpsc::UnboundedReceiver<MerkleRootsRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (channel, receiver) = mpsc::unbounded_channel();

        let server = super::create(
            listener,
            secret.to_string(),
            log_context.to_string(),
            None,
            Some(channel),
        )
        .unwrap();
        task::spawn(server);

        (
            format!("http://127.0.0.1:{port}/get_merkle_root_proof"),
            receiver,
        )
    }
}

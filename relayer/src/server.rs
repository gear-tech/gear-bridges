use crate::message_relayer::common::web_request::{
    EthTransaction, EthTransactions, Message, Messages,
};
use actix_web::{guard, middleware, web, App, HttpRequest, HttpResponse, HttpServer};
use std::{collections::HashSet, net::TcpListener};
use tokio::sync::mpsc::{error::TrySendError, Sender, UnboundedSender};

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

async fn relay_transactions(
    request: HttpRequest,
    transactions: web::Json<EthTransactions>,
    secret: web::Data<Secret>,
    log_context: web::Data<LogContext>,
    channel: web::Data<Sender<EthTransaction>>,
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

    let transactions = transactions.into_inner().transactions;
    let mut seen = HashSet::with_capacity(transactions.len());
    let transactions: Vec<_> = transactions
        .into_iter()
        .filter(|transaction| seen.insert(transaction.tx_hash.clone()))
        .collect();
    if transactions.is_empty() {
        return HttpResponse::Ok().finish();
    }
    // Reserve the entire batch atomically: overload never partially accepts a request.
    let permits = match channel.try_reserve_many(transactions.len()) {
        Ok(permits) => permits,
        Err(TrySendError::Full(_)) => {
            return HttpResponse::TooManyRequests()
                .insert_header(("Retry-After", "1"))
                .finish();
        }
        Err(TrySendError::Closed(_)) => {
            log::error!("[{}] Transaction receiver unavailable", log_context.0);
            return HttpResponse::InternalServerError().finish();
        }
    };
    for (permit, transaction) in permits.zip(transactions) {
        permit.send(transaction);
    }
    HttpResponse::Ok().finish()
}

pub fn create(
    tcp_listener: TcpListener,
    secret: String,
    log_context: String,
    messages_channel: Option<UnboundedSender<Message>>,
    transactions_channel: Option<Sender<EthTransaction>>,
) -> std::io::Result<actix_web::dev::Server> {
    let messages_channel = messages_channel.map(web::Data::new);
    let transactions_channel = transactions_channel.map(web::Data::new);

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

        app = if let Some(channel) = transactions_channel.as_ref() {
            app.app_data(channel.clone()).service(
                web::resource("/relay_transactions")
                    .route(web::post().to(relay_transactions))
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

        let response = client
            .post(&url)
            .json(&Messages {
                messages: vec![message.clone(), message.clone()],
            })
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
    async fn test_transaction_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        const SECRET: &str = "SECRET123";
        let (channel, mut receiver) = mpsc::channel(128);
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
        let url = format!("http://127.0.0.1:{port}/relay_transactions");
        let transaction = EthTransaction {
            tx_hash: format!("0x{}", "ab".repeat(32)),
        };
        let body = EthTransactions {
            transactions: vec![transaction.clone(), transaction.clone()],
        };

        let response = client.post(&url).json(&body).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert!(receiver.try_recv().is_err());

        let response = client
            .post(&url)
            .json(&body)
            .header(HEADER_TOKEN, "wrong-token")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert!(receiver.try_recv().is_err());

        let response = client
            .post(&url)
            .json(&EthTransactions::default())
            .header(HEADER_TOKEN, SECRET)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(receiver.try_recv().is_err());

        let response = client
            .post(&url)
            .json(&body)
            .header(HEADER_TOKEN, SECRET)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(response.bytes().await.unwrap().is_empty());
        assert_eq!(receiver.recv().await.unwrap().tx_hash, transaction.tx_hash);
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_messages_separate_listeners_route_independently() {
        const SECRET_A: &str = "SECRET_A";
        const SECRET_B: &str = "SECRET_B";

        let (url_a, mut receiver_a) = spawn_message_server(SECRET_A, "relayer-a");
        let (url_b, mut receiver_b) = spawn_message_server(SECRET_B, "relayer-b");
        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let body_a = Messages {
            messages: vec![Message {
                block: 111,
                nonce: 1.into(),
            }],
        };
        let body_b = Messages {
            messages: vec![Message {
                block: 222,
                nonce: 2.into(),
            }],
        };

        for (url, secret) in [(&url_a, SECRET_B), (&url_b, SECRET_A)] {
            let response = client
                .post(url)
                .json(&body_a)
                .header(HEADER_TOKEN, secret)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        }
        assert!(receiver_a.try_recv().is_err());
        assert!(receiver_b.try_recv().is_err());

        let response = client
            .post(&url_a)
            .json(&body_a)
            .header(HEADER_TOKEN, SECRET_A)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let message = receiver_a.recv().await.unwrap();
        assert_eq!(message.block, 111);
        assert_eq!(message.nonce, 1.into());
        assert!(receiver_b.try_recv().is_err());

        let response = client
            .post(&url_b)
            .json(&body_b)
            .header(HEADER_TOKEN, SECRET_B)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let message = receiver_b.recv().await.unwrap();
        assert_eq!(message.block, 222);
        assert_eq!(message.nonce, 2.into());
        assert!(receiver_a.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_transactions_closed_channel_is_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        const SECRET: &str = "SECRET123";
        let (channel, receiver) = mpsc::channel(128);
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
        let response = client
            .post(format!("http://127.0.0.1:{port}/relay_transactions"))
            .json(&EthTransactions {
                transactions: vec![EthTransaction {
                    tx_hash: format!("0x{}", "ab".repeat(32)),
                }],
            })
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
    async fn test_messages_dropped_receiver_is_error() {
        const SECRET: &str = "SECRET123";
        let (url, receiver) = spawn_message_server(SECRET, "test");
        drop(receiver);
        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let response = client
            .post(&url)
            .json(&Messages {
                messages: vec![Message {
                    block: 123,
                    nonce: 1.into(),
                }],
            })
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
    async fn test_retained_routes_reject_invalid_and_oversized_payloads() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        const SECRET: &str = "SECRET123";
        let (messages, mut message_receiver) = mpsc::unbounded_channel();
        let (transactions, mut transaction_receiver) = mpsc::channel(128);
        let server = super::create(
            listener,
            SECRET.to_string(),
            "test".to_string(),
            Some(messages),
            Some(transactions),
        )
        .unwrap();
        task::spawn(server);
        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();

        for (route, body) in [
            ("relay_messages", r#"{"messages":[]}"#),
            ("relay_transactions", r#"{"transactions":[]}"#),
        ] {
            let url = format!("http://127.0.0.1:{port}/{route}");
            for (payload, status) in [
                ("{".to_string(), reqwest::StatusCode::BAD_REQUEST),
                (
                    format!("{body}{}", " ".repeat(131_072)),
                    reqwest::StatusCode::PAYLOAD_TOO_LARGE,
                ),
            ] {
                // Rejection cases are independent; oversized requests may leave unread bodies.
                let response = client
                    .post(&url)
                    .body(payload)
                    .header(HEADER_TOKEN, SECRET)
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .header(reqwest::header::CONNECTION, "close")
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), status);
                response.bytes().await.unwrap();
            }
        }
        assert!(message_receiver.try_recv().is_err());
        assert!(transaction_receiver.try_recv().is_err());
    }

    // Reconstructed from supplied PR860 HTML, C4; not the unavailable original PoC.
    #[actix_web::test]
    async fn reconstructed_transaction_overload_is_atomic_and_recovers() {
        let (channel, mut receiver) = mpsc::channel(128);
        let transaction = EthTransaction {
            tx_hash: format!("0x{}", "ab".repeat(32)),
        };
        for _ in 0..127 {
            channel.try_send(transaction.clone()).unwrap();
        }
        let app = actix_web::test::init_service(
            App::new()
                .app_data(web::Data::new(Secret("secret".into())))
                .app_data(web::Data::new(LogContext("overload".into())))
                .app_data(web::Data::new(channel.clone()))
                .route("/relay_transactions", web::post().to(relay_transactions)),
        )
        .await;
        let batch = EthTransactions {
            transactions: vec![
                transaction.clone(),
                EthTransaction {
                    tx_hash: format!("0x{}", "cd".repeat(32)),
                },
            ],
        };
        let request = actix_web::test::TestRequest::post()
            .uri("/relay_transactions")
            .insert_header((HEADER_TOKEN, "secret"))
            .set_json(&batch)
            .to_request();
        let response = actix_web::test::call_service(&app, request).await;
        assert_eq!(
            response.status(),
            actix_web::http::StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(response.headers().get("Retry-After").unwrap(), "1");
        assert_eq!(receiver.len(), 127, "overload must not partially enqueue");
        receiver.recv().await.unwrap();
        let request = actix_web::test::TestRequest::post()
            .uri("/relay_transactions")
            .insert_header((HEADER_TOKEN, "secret"))
            .set_json(&batch)
            .to_request();
        let response = actix_web::test::call_service(&app, request).await;
        assert_eq!(response.status(), actix_web::http::StatusCode::OK);
        assert_eq!(receiver.len(), 128);
        let request = actix_web::test::TestRequest::post()
            .uri("/relay_transactions")
            .insert_header((HEADER_TOKEN, "secret"))
            .set_json(&batch)
            .to_request();
        assert_eq!(
            actix_web::test::call_service(&app, request).await.status(),
            actix_web::http::StatusCode::TOO_MANY_REQUESTS
        );
    }

    fn spawn_message_server(
        secret: &str,
        log_context: &str,
    ) -> (String, mpsc::UnboundedReceiver<Message>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (channel, receiver) = mpsc::unbounded_channel();
        let server = super::create(
            listener,
            secret.to_string(),
            log_context.to_string(),
            Some(channel),
            None,
        )
        .unwrap();
        task::spawn(server);

        (format!("http://127.0.0.1:{port}/relay_messages"), receiver)
    }
}

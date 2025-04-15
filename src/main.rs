use std::net::SocketAddr;

use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::{
    body::{Bytes, Incoming},
    service::service_fn,
    upgrade::Upgraded,
    Method, Request, Response,
};

use hyper_util::rt::TokioIo;
use tokio::{
    io::{AsyncReadExt, BufReader},
    net::{TcpListener, TcpStream},
};

type ClientBuilder = hyper::client::conn::http1::Builder;
type ServerBuilder = hyper::server::conn::http1::Builder;

/**
// Pseudo-code steps (some libraries not shown here)
if CONNECT request {
    // Step 1: Respond "200 OK" to tell browser tunnel is ready
    respond_ok_to_browser()

    // Step 2: Perform TLS handshake with browser using mitm cert
    let client_tls = rustls_acceptor.accept(upgraded_stream).await?;

    // Step 3: Parse HTTP request from TLS client stream
    let browser_request = parse_http_request(client_tls).await?;

    // Step 4: Connect to the real server over TLS
    let server_tls = rustls_connector.connect(domain, tcp_stream_to_real_server).await?;

    // Step 5: Send request to real server
    server_tls.write_all(browser_request).await?;

    // Step 6: Read response from server
    let mut response = read_http_response(server_tls).await?;

    // Step 7: Modify or log response if needed
    log_or_modify_response(&mut response);

    // Step 8: Send response back to browser
    client_tls.write_all(response).await?;
}

*/
async fn proxy_handler(
    req: Request<Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    if Method::CONNECT == req.method() {
        if let Some(addr) = req.uri().authority().map(|auth| auth.to_string()) {
            tokio::task::spawn(async move {
                match hyper::upgrade::on(req).await {
                    Ok(upgrade) => {
                        if let Err(e) = tunnel(upgrade, addr).await {
                            eprintln!("server io error {}", e);
                        }
                    }
                    Err(e) => eprintln!("upgrade error : {}", e),
                }
            });
            Ok(Response::new(empty()))
        } else {
            eprintln!("CONNECT host is not socker addr: {:?}", req.uri());
            let mut resp = Response::new(
                Full::new("CONNECT must be to a socker address".into())
                    .map_err(|never| match never {})
                    .boxed(),
            );
            *resp.status_mut() = http::StatusCode::BAD_REQUEST;

            Ok(resp)
        }
    } else {
        let host = req.uri().host().expect("uri has no host");
        let port = req.uri().port_u16().unwrap_or(80);

        let stream = TcpStream::connect((host, port)).await.unwrap();
        let io = TokioIo::new(stream);

        let (mut sender, conn) = ClientBuilder::new()
            .preserve_header_case(true)
            .title_case_headers(true)
            .handshake::<_, hyper::body::Incoming>(io)
            .await?;

        tokio::task::spawn(async move {
            if let Err(e) = conn.await {
                println!("connection failed : {:?}", e);
            }
        });

        let mut resp = sender.send_request(req).await?;
        let body_bytes = resp.body_mut().collect().await?.to_bytes();

        if let Ok(text) = std::str::from_utf8(&body_bytes) {
            println!("html response : \n {}", text);
        } else {
            println!("Response body is not valid utf 8 ");
        }

        Ok(resp.map(|b| b.boxed()))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 6969));

    let listener = TcpListener::bind(addr).await?;
    println!("Proxy running at http://{}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let service = service_fn(proxy_handler);

        tokio::task::spawn(async move {
            if let Err(err) = ServerBuilder::new()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                eprintln!("x connection error : {}", err);
            }
        });
    }
}

fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new().map_err(|n| match n {}).boxed()
}
async fn tunnel(upgrade: Upgraded, addr: String) -> std::io::Result<()> {
    let mut server = TcpStream::connect(addr).await?;
    let mut upgraded = TokioIo::new(upgrade);

    println!("Running in tunnel , start copy bidirectional");
    let (from_client, from_server) =
        tokio::io::copy_bidirectional(&mut upgraded, &mut server).await?;

    println!("Running in tunnel , end copy bidirectional");

    println!(
        "client wrote {} btyes and revived {} bytes",
        from_client, from_server
    );

    let mut reader = BufReader::new(&mut server);
    let mut response_buffer = Vec::new();
    reader.read_to_end(&mut response_buffer).await?;

    if let Ok(html_response) = std::str::from_utf8(&response_buffer) {
        println!("Server respone body (html) : \n {}", html_response);
    } else {
        println!("Response body is not valid UTF-8 or binary data");
    }

    Ok(())
}

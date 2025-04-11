use std::net::SocketAddr;

use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::{
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
    Request, Response,
};

use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};

type ClientBuilder = hyper::client::conn::http1::Builder;

async fn proxy_handler(
    req: Request<Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

    let listener = TcpListener::bind(addr).await?;
    println!("Proxy running at http://{}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let service = service_fn(proxy_handler);

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                eprintln!("x connection error : {}", err);
            }
        });
    }
}

fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new().map_err(|n| match n {}).boxed()
}

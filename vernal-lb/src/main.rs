use tokio::io;
use tokio::net::{TcpListener, TcpStream};

use std::env;

struct Api {
    urls: Vec<String>,
    current_index: usize,
}

impl Api {
    fn new(urls: Vec<String>) -> Self {
        Self {
            urls,
            current_index: 0,
        }
    }

    fn next_url(&mut self) -> String {
        let url = self.urls[self.current_index].clone();
        self.current_index = (self.current_index + 1) % self.urls.len();
        url
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    let port = env::var("LISTEN_PORT")
        .expect("LISTEN_PORT must be set")
        .parse::<u16>()
        .expect("LISTEN_PORT must be a valid port number");

    let listener = TcpListener::bind(("0.0.0.0", port)).await.unwrap();

    let addrs = env::var("VERNAL_LB_ADDRS")
        .expect("VERNAL_LB_ADDRS must be set")
        .split(',')
        .map(|s| s.to_string())
        .collect();

    let mut api = Api::new(addrs);

    println!("Listening on port {}", port);

    while let Ok((mut downstream, _)) = listener.accept().await {
        downstream
            .set_nodelay(true)
            .expect("set_nodelay call failed");

        let addr = api.next_url();

        tokio::spawn(async move {
            let mut upstream = TcpStream::connect(addr).await.unwrap();
            upstream.set_nodelay(true).expect("set_nodelay call failed");

            io::copy_bidirectional(&mut downstream, &mut upstream)
                .await
                .unwrap();
        });
    }

    Ok(())
}

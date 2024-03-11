use tokio::io;
use tokio::net::{TcpListener, UnixStream};

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

    let addrs: Vec<String> = env::var("VERNAL_LB_SOCKETS")
        .expect("VERNAL_LB_SOCKETS must be set")
        .split(',')
        .map(|s| s.to_string())
        .collect();

    let mut api = Api::new(addrs.clone());

    println!("Listening on port {}", port);
    println!("Proxying to {:?}", addrs);

    while let Ok((mut downstream, _)) = listener.accept().await {
        downstream
            .set_nodelay(true)
            .expect("set_nodelay call failed");

        let addr = api.next_url();

        tokio::spawn(async move {
            let mut upstream = UnixStream::connect(addr).await.unwrap();

            io::copy_bidirectional(&mut downstream, &mut upstream)
                .await
                .unwrap();
        });
    }

    Ok(())
}

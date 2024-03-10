FROM rust:1.76.0-slim-buster as builder

WORKDIR /usr/src/app

COPY Cargo.toml .
COPY src/main.rs ./src/

RUN apt update && apt install -y musl-tools
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo install --target x86_64-unknown-linux-musl --path .

CMD ["vernal-lb"]

FROM scratch

COPY --from=builder /usr/src/app/target/x86_64-unknown-linux-musl/release/vernal-lb /usr/local/bin/

CMD ["vernal-lb"]

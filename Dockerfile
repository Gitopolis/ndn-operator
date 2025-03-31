FROM rust:1.85.0-slim-bullseye AS builder

WORKDIR /usr/src/app

COPY src src
COPY Cargo.toml Cargo.toml

RUN cargo build --release

FROM debian:bullseye-slim

COPY --from=builder /usr/src/app/target/release/ndn-operator /
COPY --from=builder /usr/src/app/target/release/pod_init /

CMD ["/ndn-operator"]
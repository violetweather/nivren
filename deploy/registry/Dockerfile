FROM rust:1.88-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --locked --release --bin niv

FROM debian:bookworm-slim
RUN groupadd --gid 10001 nivren && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin nivren
COPY --from=builder /src/target/release/niv /usr/local/bin/niv
USER 10001:10001
VOLUME ["/registry"]
EXPOSE 8080
ENTRYPOINT ["niv", "registry", "serve", "/registry", "0.0.0.0:8080", "1"]

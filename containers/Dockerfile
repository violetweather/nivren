FROM rust:1.88-bookworm AS builder

WORKDIR /source
COPY . .
RUN cargo build --release --locked --bin niv \
    && strip target/release/niv

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 nivren \
    && useradd --system --uid 10001 --gid nivren --home-dir /workspace nivren \
    && mkdir /workspace \
    && chown nivren:nivren /workspace

COPY --from=builder /source/target/release/niv /usr/local/bin/niv
COPY LICENSE README.md SECURITY.md /usr/share/doc/nivren/

USER 10001:10001
WORKDIR /workspace
ENTRYPOINT ["niv"]
CMD ["version"]

FROM rust:1-slim-bookworm AS builder
WORKDIR /app/offchain
COPY offchain/Cargo.toml offchain/Cargo.lock ./
COPY offchain/src ./src
RUN cargo build --release

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app/offchain
COPY --from=builder /app/offchain/target/release/milestone_escrow_api ./milestone_escrow_api
COPY onchain/deploy /app/onchain/deploy

ENV BIND_ADDR=0.0.0.0:3000
EXPOSE 3000
CMD ["./milestone_escrow_api"]

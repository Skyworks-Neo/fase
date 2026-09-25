FROM rust:1.98-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY fase-api ./fase-api
COPY fase-controller ./fase-controller
RUN cargo build --locked --release -p fase-controller

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/fase-controller /usr/local/bin/fase-controller
LABEL org.opencontainers.image.source="https://github.com/Skyworks-Neo/fase"
ENTRYPOINT ["/usr/local/bin/fase-controller"]
USER 65532:65532

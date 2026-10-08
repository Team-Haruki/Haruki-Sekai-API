FROM rust:1.98-alpine AS chef
RUN apk add --no-cache musl-dev \
    && cargo install cargo-chef --version 0.1.78 --locked
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json ./recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json \
    --bin haruki-sekai-api --bin master_registry --bin master_ingest --bin run_ingest
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked \
    --bin haruki-sekai-api --bin master_registry --bin master_ingest --bin run_ingest

FROM alpine:3.24
RUN apk --no-cache add \
    ca-certificates=20260909-r0 \
    tzdata=2026d-r0 \
    git=2.54.0-r0 \
    gnupg=2.4.9-r1 \
    openssh-keygen=10.3_p1-r1 \
    && addgroup -S haruki \
    && adduser -S -G haruki haruki
WORKDIR /app
COPY --chown=haruki:haruki --from=builder /app/target/release/haruki-sekai-api .
COPY --chown=haruki:haruki --from=builder /app/target/release/run_ingest .
COPY --chown=haruki:haruki --from=builder /app/target/release/master_registry .
COPY --chown=haruki:haruki --from=builder /app/target/release/master_ingest .
COPY --chown=haruki:haruki schema_info.json ./schema_info.json
COPY --chown=haruki:haruki Data/structures ./Data/structures
RUN mkdir -p logs && chown haruki:haruki logs
EXPOSE 9999 9998 9997
ENV TZ=Asia/Shanghai
ENV RUST_LOG=info
# mimalloc (the binaries' global allocator) eagerly commits its first arena,
# which shows up as ~12 MB of anonymous RSS at idle on this image; committing
# on demand keeps the idle footprint within ~2 MB of the system allocator.
ENV MIMALLOC_ARENA_EAGER_COMMIT=0
ARG VERSION=dev
LABEL org.opencontainers.image.version="${VERSION}"
USER haruki
CMD ["./haruki-sekai-api"]

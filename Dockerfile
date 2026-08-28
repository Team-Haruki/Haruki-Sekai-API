FROM rust:1.98-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
ARG VERSION=dev
RUN if [ "$VERSION" != "dev" ]; then \
    CLEAN_VERSION=$(echo "$VERSION" | sed 's/^v//'); \
    sed -i "s/^version = \".*\"/version = \"${CLEAN_VERSION}\"/" Cargo.toml; \
    echo "Building version: ${CLEAN_VERSION}"; \
    fi
RUN cargo build --release --locked

FROM alpine:3.24
RUN apk --no-cache add \
    ca-certificates=20260611-r0 \
    tzdata=2026c-r0 \
    git=2.54.0-r0 \
    gnupg=2.4.9-r1 \
    openssh-keygen=10.3_p1-r0 \
    && addgroup -S haruki \
    && adduser -S -G haruki haruki
WORKDIR /app
COPY --chown=haruki:haruki --from=builder /app/target/release/haruki-sekai-api .
COPY --chown=haruki:haruki --from=builder /app/target/release/run_ingest .
COPY --chown=haruki:haruki Data ./Data
RUN mkdir -p logs && chown haruki:haruki logs
EXPOSE 9999
ENV TZ=Asia/Shanghai
ENV RUST_LOG=info
ARG VERSION=dev
LABEL org.opencontainers.image.version="${VERSION}"
USER haruki
CMD ["./haruki-sekai-api"]

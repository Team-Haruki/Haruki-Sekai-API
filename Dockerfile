FROM rust:1.95-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
ARG VERSION=dev
RUN if [ "$VERSION" != "dev" ]; then \
    CLEAN_VERSION=$(echo "$VERSION" | sed 's/^v//'); \
    sed -i "s/^version = \".*\"/version = \"${CLEAN_VERSION}\"/" Cargo.toml; \
    echo "Building version: ${CLEAN_VERSION}"; \
    fi
RUN cargo build --release

FROM alpine:3.23
RUN apk --no-cache add ca-certificates tzdata git gnupg openssh-keygen \
    && addgroup -S -g 65532 haruki \
    && adduser -S -u 65532 -G haruki -h /app -s /sbin/nologin haruki
WORKDIR /app
COPY --from=builder /app/target/release/haruki-sekai-api .
COPY --from=builder /app/target/release/haruki-sekai-updater .
COPY Data ./Data
RUN mkdir -p logs && chown -R haruki:haruki /app
USER 65532:65532
EXPOSE 9999
ENV TZ=Asia/Shanghai
ENV RUST_LOG=info
ARG VERSION=dev
LABEL org.opencontainers.image.version="${VERSION}"
LABEL org.opencontainers.image.source="https://github.com/Team-Haruki/Haruki-Sekai-API"
# Default entrypoint runs the API server. Override CMD with
# `["./haruki-sekai-updater"]` (or `args: ["./haruki-sekai-updater"]` in K8s)
# to run the cron-only worker from the same image.
CMD ["./haruki-sekai-api"]

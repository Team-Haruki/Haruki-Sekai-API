FROM rust:1.96-alpine AS builder
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

FROM alpine:3.24
RUN apk --no-cache add ca-certificates tzdata git gnupg openssh-keygen
WORKDIR /app
COPY --from=builder /app/target/release/haruki-sekai-api .
COPY Data ./Data
RUN mkdir -p logs
EXPOSE 9999
ENV TZ=Asia/Shanghai
ENV RUST_LOG=info
ARG VERSION=dev
LABEL org.opencontainers.image.version="${VERSION}"
CMD ["./haruki-sekai-api"]

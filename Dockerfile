# Build only the CLI. The final image has no shell, package manager or GUI assets.
FROM rust:1.98.1-alpine3.24@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f AS build
RUN apk add --no-cache build-base
WORKDIR /src
COPY Cargo.toml Cargo.lock config.example.toml LICENSE ./
COPY crates/ crates/
COPY apps/ apps/
COPY eval/ eval/
ARG TARGETARCH
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=chat-tldr-release-${TARGETARCH} \
    cargo build --release --locked -p chat-tldr && \
    mkdir -p /out /runtime/data /runtime/tmp /runtime/etc && \
    cp target/release/chat-tldr /out/chat-tldr && \
    chmod 1777 /runtime/tmp && chown 10001:10001 /runtime/data && \
    printf 'chat-tldr:x:10001:10001:chat-tldr:/data:/sbin/nologin\n' > /runtime/etc/passwd && \
    printf 'chat-tldr:x:10001:\n' > /runtime/etc/group && \
    if readelf -l /out/chat-tldr | grep -q INTERP; then echo 'CLI must be statically linked'; exit 1; fi

# Optional: docker build --target binary --output type=local,dest=dist/linux .
FROM scratch AS binary
COPY --from=build /out/chat-tldr /chat-tldr

FROM scratch AS runtime
LABEL org.opencontainers.image.title="chat-tldr" \
      org.opencontainers.image.description="Evidence-backed chat analysis CLI" \
      org.opencontainers.image.source="https://github.com/Develata/chat-tldr" \
      org.opencontainers.image.licenses="MIT"
COPY --from=build /runtime/ /
COPY --from=build /out/chat-tldr /chat-tldr
COPY LICENSE /LICENSE
# rustls uses compiled-in web PKI roots; SQLite is bundled into the static CLI.
ENV XDG_DATA_HOME=/data
WORKDIR /data
USER 10001:10001
ENTRYPOINT ["/chat-tldr"]
CMD ["version"]

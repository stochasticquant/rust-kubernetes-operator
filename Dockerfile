# syntax=docker/dockerfile:1
# Ferrogate container image.
# - Build: pinned Rust toolchain, locked dependencies, release profile.
# - Runtime: distroless (no shell, no package manager), non-root user 65532.
# Base images are pinned by digest; Dependabot proposes updates.

FROM rust:1.99-slim-bookworm@sha256:86234e857a413004348b471d11a4ebe1aa92d2eae2b6b143cac68fd1ac3c48be AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p ferrogate \
 && install -D -m 0755 target/release/ferrogate /out/ferrogate

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f
LABEL org.opencontainers.image.title="ferrogate" \
      org.opencontainers.image.description="Rust-native Kubernetes policy operator and admission webhook" \
      org.opencontainers.image.source="https://github.com/stochasticquant/rust-kubernetes-operator" \
      org.opencontainers.image.licenses="Apache-2.0"
COPY --from=build /out/ferrogate /usr/local/bin/ferrogate
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/ferrogate"]
CMD ["--help"]

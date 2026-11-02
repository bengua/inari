# Brique 1 — image du PoC : inari-core compilé en musl statique, de quoi faire
# tourner le pont openfox-acp (python3) et la preuve de capacité sandbox (bwrap).
# Build : docker build -t inari-poc-brique-1:local -f poc/brique-1/Dockerfile .
FROM rust:1.98-alpine AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# sqlite-vec.c emploie les types BSD u_int*_t, absents de musl sans _GNU_SOURCE
RUN CFLAGS="-D_GNU_SOURCE -include sys/types.h" cargo build --release -p inari-core

FROM alpine:3.21
RUN apk add --no-cache bubblewrap python3 ca-certificates curl
COPY --from=build /src/target/release/inari-core-rs /usr/local/bin/inari-core-rs
# openfox-acp est monté en lecture seule par run.sh (son absorption dans le
# repo est la brique 2 — pas de copie ici pour ne pas dupliquer la dette).
ENTRYPOINT ["inari-core-rs"]

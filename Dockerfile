FROM rust:alpine as build-env
RUN apk update && apk --no-cache --update add build-base openssl openssl-dev perl
WORKDIR /app
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl

FROM gcr.io/distroless/static
COPY --from=build-env /app/target/x86_64-unknown-linux-musl/release/calconv /bin/calconv
ENTRYPOINT [ "/bin/calconv" ]
FROM rust:slim as build-env
RUN apt-get update && apt-get install -y build-essential
WORKDIR /app
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-gnu

FROM gcr.io/distroless/cc
COPY --from=build-env /app/target/x86_64-unknown-linux-gnu/release/calconv /bin/calconv
ENTRYPOINT [ "/bin/calconv" ]
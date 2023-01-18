FROM rust:slim-buster
COPY . /app
WORKDIR /app
RUN cargo build -r

FROM debian:buster-slim
WORKDIR /app
COPY --from=0 /app/target/release/discoveryd .
ENV ROCKET_ADDRESS=0.0.0.0
EXPOSE 8000
CMD ["./discoveryd"]
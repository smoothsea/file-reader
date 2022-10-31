#######################BUILD IAMGE################
FROM rust:1.48.0 as build
RUN mkdir /app && cd /app
ADD ./ /app/file-reader
WORKDIR /app/file-reader
RUN rustup default nightly-2022-03-15
RUN cargo build --release

#######################RUNTIME IMAGE##############
FROM debian:buster-slim
RUN apt-get update && apt-get install -y \
            --no-install-recommends \
            openssl \
            ca-certificates
ENV TZ=Asia/Shanghai
RUN ln -snf /usr/share/zoneinfo/$TZ /etc/localtime && echo $TZ > /etc/timezone 
COPY --from=build app/file-reader/target/release/file_reader .
COPY --from=build app/file-reader/templates ./templates/
EXPOSE 8080
WORKDIR /
CMD ["/file_reader", "-d", "/logs"]

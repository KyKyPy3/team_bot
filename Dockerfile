FROM rust:latest

RUN apt update && apt upgrade -y
RUN apt install -y libssl-dev pkg-config musl-tools gcc-x86-64-linux-gnu build-essential

COPY ./ ./app

WORKDIR /app

RUN rustup target add x86_64-unknown-linux-gnu

ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=/usr/bin/x86_64-linux-gnu-gcc

CMD ["cargo", "build", "--target=x86_64-unknown-linux-gnu", "--release"]

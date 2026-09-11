# SPDX-License-Identifier: Apache-2.0
ARG BASE=ubuntu@sha256:224a1869083a311ef3f13648a154ba79832fbef6364d31493642ca03082da254
FROM ${BASE}
ENV DEBIAN_FRONTEND=noninteractive CARGO_HOME=/opt/cargo RUSTUP_HOME=/opt/rustup PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
ARG UBUNTU_MIRROR=
RUN if [ -n "$UBUNTU_MIRROR" ]; then sed -i -e "s|http://archive.ubuntu.com/ubuntu|$UBUNTU_MIRROR|g" -e "s|http://security.ubuntu.com/ubuntu|$UBUNTU_MIRROR|g" /etc/apt/sources.list.d/ubuntu.sources; fi
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl python3 git build-essential qemu-system-x86=1:8.2.2+ds-0ubuntu1.18 ovmf=2024.02-2ubuntu0.9 mtools=4.0.43-1build1 dosfstools=4.2-1.1build1 xorriso=1:1.5.6-1.1ubuntu3 && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://static.rust-lang.org/rustup/archive/1.29.1/x86_64-unknown-linux-gnu/rustup-init -o /tmp/rustup-init && echo "dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71 /tmp/rustup-init" | sha256sum -c - && chmod +x /tmp/rustup-init && /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain 1.98.1 --component rustfmt --component clippy --target x86_64-unknown-none && rm /tmp/rustup-init
COPY reference /opt/reference
RUN cd /opt/reference && cargo fetch --locked && python3 tools/environment.py verify && python3 tools/environment.py fetch-bootloader
COPY controller /opt/controller
RUN chmod -R a+rX /opt && mkdir -p /work
USER 1000:1000
WORKDIR /work
CMD ["sleep", "infinity"]

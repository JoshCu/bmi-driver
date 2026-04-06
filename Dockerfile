FROM awiciroh/ngiab AS ngiab_build
WORKDIR /build
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | bash -s -- -y
RUN dnf install -y python3.11-devel g++ cmake
COPY . .
RUN cargo build --release --features static

FROM awiciroh/ngiab AS ngiab
COPY --from=ngiab_build /build/target/release/bmi-driver /usr/local/bin/bmi-driver
VOLUME /data
RUN ln -s /ngen/ngen/data /data
ENV OMP_NUM_THREADS=1
ENTRYPOINT [ "/usr/local/bin/bmi-driver" ]
CMD ["/data"]

FROM rockylinux:9.1 AS final
RUN dnf install -y python3.11 libgfortran attr
COPY --from=ngiab_build /build/target/release/bmi-driver /usr/local/bin/bmi-driver
COPY --from=awiciroh/ngiab /dmod/datasets /dmod/datasets
COPY --from=awiciroh/ngiab /dmod/shared_libs /dmod/shared_libs
RUN echo "/dmod/shared_libs/" >> /etc/ld.so.conf.d/ngen.conf && ldconfig -v

VOLUME /data
RUN mkdir -p /ngen/ngen/data && ln -s /ngen/ngen/data /data
WORKDIR /data
ENV OMP_NUM_THREADS=1
ENTRYPOINT [ "/usr/local/bin/bmi-driver" ]
CMD ["/data"]

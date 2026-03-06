FROM awiciroh/ngiab AS ngiab_build
WORKDIR /build
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | bash -s -- -y
RUN dnf install -y python3.11-devel netcdf-devel hdf5-devel sqlite-devel g++
COPY . .
RUN cargo build --release

FROM awiciroh/ngiab AS final
COPY --from=ngiab_build /build/target/release/bmi-driver /usr/local/bin/bmi-driver
VOLUME /data
RUN ln -s /ngen/ngen/data /data
ENTRYPOINT [ "/usr/local/bin/bmi-driver" ]
CMD ["/data"]

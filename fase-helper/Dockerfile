FROM ghcr.io/skyworks-neo/fase:latest
USER root
RUN groupadd --gid 1000 fase \
    && useradd --uid 1000 --gid fase --no-create-home fase
USER 1000:1000

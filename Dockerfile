FROM alpine:3.20
ARG TARGETARCH
RUN apk add --no-cache ca-certificates git
# The release workflow stages one static binary per platform at
# {amd64,arm64}/lh; buildx sets TARGETARCH to pick the matching one.
# Actions artifacts strip the execute bit — restore it at copy time
COPY --chmod=755 ${TARGETARCH}/lh /usr/local/bin/lh
EXPOSE 3030
CMD ["lh", "serve"]

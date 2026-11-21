FROM alpine:3.24
ARG TARGETARCH
# `apk upgrade` picks up security fixes released since the alpine base
# image was last published; the weekly scheduled rebuild re-runs it.
RUN apk upgrade --no-cache && apk add --no-cache ca-certificates git
# The release workflow stages one static binary per platform at
# {amd64,arm64}/lh; buildx sets TARGETARCH to pick the matching one.
# Actions artifacts strip the execute bit — restore it at copy time
COPY --chmod=755 ${TARGETARCH}/lh /usr/local/bin/lh
EXPOSE 3030
CMD ["lh", "serve"]

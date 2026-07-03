FROM alpine:3.20
RUN apk add --no-cache ca-certificates git
# Actions artifacts strip the execute bit — restore it at copy time
COPY --chmod=755 lh /usr/local/bin/lh
EXPOSE 3030
CMD ["lh", "serve"]

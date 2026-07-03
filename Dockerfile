FROM alpine:3.20
RUN apk add --no-cache ca-certificates git
COPY lh /usr/local/bin/lh
EXPOSE 3030
CMD ["lh", "serve"]

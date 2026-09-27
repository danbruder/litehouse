# litehouse landing page

A single static `index.html` (no build step, no external assets) served by
busybox httpd on port 8080. Deploys like any other litehouse app.

Messaging source: `docs/landing/messaging.md`.

## Preview locally

    python3 -m http.server -d site 8080
    # or: docker build -t lh-site site && docker run --rm -p 8080:8080 lh-site

## Deploy

The generated workflow builds from the repo root, so push this directory to
its own repo and register it:

    lh create site --repo you/litehouse-site
    git push
    lh deploys site --wait

Or build and deploy an image directly:

    docker build -t ghcr.io/you/litehouse-site:latest site
    docker push ghcr.io/you/litehouse-site:latest
    lh deploy site --image ghcr.io/you/litehouse-site:latest

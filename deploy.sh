#!/bin/bash
cd assets
elm make src/Main.elm --optimize --output=dist/app.js
cp public/index.html dist/index.html
cd ..

# compile rust stuff
TARGET_CC=x86_64-linux-musl-gcc cargo build --release --target x86_64-unknown-linux-musl

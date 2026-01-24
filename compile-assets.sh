#!/bin/bash
cd assets
npm run build:css
elm make src/Main.elm --optimize --output=dist/app.js
cp public/sse-manager.js dist/sse-manager.js
cp public/index.html dist/index.html
cd ..

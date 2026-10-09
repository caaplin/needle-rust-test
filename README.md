# needle-rust-test

This repository contains the Ubuntu notebook server. The server is written in Rust. The server runs notebook commands inside an Ubuntu Docker container. The server sends the command output to the browser.

See [ubuntu-notebook-server/README.md](ubuntu-notebook-server/README.md) for the build and run instructions.

## Quick Start

Run this command in the `ubuntu-notebook-server` directory:

```
docker compose up --build
```

Open a browser. Go to `http://localhost:8080`. Sign in with the username `admin` and the password `notebook`.

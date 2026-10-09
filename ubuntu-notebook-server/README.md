# Ubuntu Notebook Server

This project is a Rust server for the Ubuntu notebook. The server runs notebook commands inside an Ubuntu Docker container. The server sends the command output to the browser. The browser shows a notebook web page.

## Architecture

The system has three parts:

1. **Ubuntu Docker container.** This container runs the shell.
2. **Rust server.** This server receives commands from the browser. It runs the commands in the container. It sends the output back.
3. **Frontend web page.** This page shows the notebook interface.

The Rust server connects to the Docker daemon. It uses the Docker API to run commands in the container. The browser connects to the Rust server through a web address.

The server uses these tools:

| Part | Tool | Purpose |
|---|---|---|
| Web framework | Axum | Handles HTTP requests. |
| Async runtime | Tokio | Runs asynchronous tasks. |
| Docker client | Bollard | Connects to the Docker daemon. |
| Command execution | Bollard exec | Runs commands inside a running container. |
| JSON handling | Serde | Serializes and deserializes data. |
| Authentication | tower-http | Adds basic authentication. |
| HTTPS | axum-server with rustls | Serves HTTPS when certificate files are set. |

## Requirements

- Docker
- Rust (for `cargo run` and `cargo build` outside Docker)

## Quick Start

Use Docker Compose. Run this command in this directory:

```
docker compose up --build
```

Open a browser. Go to `http://localhost:8080`.

Type the username `admin` and the password `notebook`.

Change the credentials in `docker-compose.yml` before you expose the server.

## Build and Run Without Compose

### Step 1. Build the Docker Container

Build the Ubuntu image:

```
docker build -t ubuntu-notebook .
```

Run the container:

```
docker run -d --name notebook-container ubuntu-notebook
```

Do not publish a container port. The Rust server connects to the container through the Docker daemon. The container does not need a public port.

### Step 2. Run the Rust Server

Run this command in this directory:

```
cargo run
```

The server listens on port 8080. Open a browser. Go to `http://localhost:8080`.

The browser asks for a username and a password. The default username is `admin`. The default password is `notebook`.

Run the server from this directory. The server serves the frontend files from the `public/` directory.

## Test the Server

1. Start the Rust server.
2. Open a browser. Go to `http://localhost:8080`.
3. Type the username and password.
4. Add a command cell. Type a command (for example, `ls -la`). Click the run button.
5. Check the output. The output must show the result of the command.
6. Add a comment cell. Type some text. Check that the text shows in the cell.

## API

The server has these endpoints:

| Endpoint | Method | Purpose |
|---|---|---|
| `/api/execute` | POST | Run a command in the container. |
| `/api/cells` | GET | Get the list of cells. |
| `/api/cells` | POST | Add a new cell. |
| `/api/cells/{id}` | PUT | Update a cell. |
| `/api/cells/{id}` | DELETE | Delete a cell. |

All endpoints require basic authentication.

### POST /api/execute

Request body:

```json
{
  "command": "ls -la",
  "cell_id": "cell-1737849600000000000-0"
}
```

The `cell_id` field is optional. When you set it, the server stores the output on the cell.

Response body:

```json
{
  "output": "total 0\n",
  "exit_code": 0,
  "cell_id": "cell-1737849600000000000-0"
}
```

### POST /api/cells

Request body:

```json
{
  "cell_type": "command",
  "content": "ls -la"
}
```

The `cell_type` value is `command` or `comment`. The response is the new cell. The status code is 201.

### GET /api/cells

The response is a JSON array of cells:

```json
[
  {
    "id": "cell-1737849600000000000-0",
    "cell_type": "command",
    "content": "ls -la",
    "output": "total 0\n"
  }
]
```

### PUT /api/cells/{id}

Request body. All fields are optional:

```json
{
  "cell_type": "command",
  "content": "ls -la /tmp",
  "output": "total 0\n"
}
```

The response is the updated cell.

### DELETE /api/cells/{id}

The response has no body. The status code is 204.

## Configuration

The server reads these environment variables:

| Variable | Default | Purpose |
|---|---|---|
| `NOTEBOOK_PORT` | `8080` | TCP port for the server. |
| `NOTEBOOK_USERNAME` | `admin` | Basic auth username. |
| `NOTEBOOK_PASSWORD` | `notebook` | Basic auth password. |
| `NOTEBOOK_CONTAINER` | `notebook-container` | Name of the Docker container. |
| `NOTEBOOK_ALLOWED_COMMANDS` | built-in list | Comma-separated allow list of commands. |
| `NOTEBOOK_EXEC_TIMEOUT_SECS` | `30` | Maximum run time for one command. |
| `NOTEBOOK_TLS_CERT` | unset | Path to a TLS certificate file (PEM). |
| `NOTEBOOK_TLS_KEY` | unset | Path to a TLS private key file (PEM). |
| `DOCKER_HOST` | unset | Docker daemon address. Bollard reads this variable. |

When `NOTEBOOK_TLS_CERT` and `NOTEBOOK_TLS_KEY` are set, the server uses HTTPS. It uses axum-server with rustls. Otherwise the server uses HTTP.

Example:

```
NOTEBOOK_PORT=8080 NOTEBOOK_USERNAME=admin NOTEBOOK_PASSWORD=secret cargo run
```

Bollard uses the `DOCKER_HOST` environment variable. When this variable is not set, Bollard uses the local Docker socket (`/var/run/docker.sock` on Linux). Set `DOCKER_HOST` to a `unix://` address for a socket. Set it to a `tcp://`, `http://` or `https://` address for a remote daemon. Without `DOCKER_HOST`, the HTTP connection uses `localhost:2375`.

## Security

The server applies these protections:

1. **Basic authentication.** The server uses `tower-http` basic authentication. All endpoints and the frontend require a username and a password.
2. **HTTPS.** Set `NOTEBOOK_TLS_CERT` and `NOTEBOOK_TLS_KEY` to serve HTTPS. Use axum-server with rustls. You can also put the server behind a reverse proxy.
3. **Command allow list.** The server checks every command against a whitelist of allowed commands. The server does not allow dangerous commands. The first word of every command segment must be in the allow list. The server also rejects a block list of dangerous patterns (for example, `rm -rf /` and writes to block devices).
4. **Non-root user.** The Dockerfile creates a normal user. The container runs the shell as that user. Notebook commands cannot run as root.

The command check is not a sandbox. Always run the notebook container as a disposable container. Do not mount host directories into the notebook container.

## Cells

The server stores cells in memory. The server uses `Arc<Mutex<Vec<Cell>>>`. A cell has these fields:

```rust
struct Cell {
    id: String,
    cell_type: String, // "command" or "comment"
    content: String,
    output: Option<String>,
}
```

The in-memory store loses data when the server restarts. For production, use a database.

## Deploy the Server

1. Build the release binary:

```
cargo build --release
```

2. Put the binary on a server. The server must have a public IP address.
3. Run the binary. Use a process manager (for example, `systemd`).
4. Give the server address to the user.

To deploy with Docker, build the server image:

```
docker build -f Dockerfile.server -t ubuntu-notebook-server .
```

Run the server image. Mount the Docker socket:

```
docker run -d \
  --name notebook-server \
  -p 8080:8080 \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -e NOTEBOOK_USERNAME=admin \
  -e NOTEBOOK_PASSWORD=secret \
  ubuntu-notebook-server
```

## Project Structure

```
ubuntu-notebook-server/
├── Cargo.toml            Dependencies
├── Dockerfile            Ubuntu container image (ubuntu-notebook)
├── Dockerfile.server     Rust server image
├── docker-compose.yml    Notebook container + server
├── public/
│   ├── index.html        Notebook web page
│   ├── app.js            Frontend logic
│   └── style.css         Frontend styles
└── src/
    └── main.rs           Rust server
```

## Important Notes

- Bollard needs access to the Docker socket. Mount `/var/run/docker.sock` into the Rust server container. Or run the Rust server on the host machine.
- The `command` variable must be sanitized. Do not pass user input directly to the shell. The server checks every command against the allow list. The server runs the command as `bash -c <command>` in the container as a non-root user.
- Use `tokio::process::Command` as an alternative to Bollard. This method runs commands directly on the host. But this method does not use the Docker container.
- Bollard supports both Docker and Podman. Use the `DOCKER_HOST` environment variable to set the connection location.
- For streaming output, use WebSockets. Axum supports WebSockets. This server returns the full output when the command finishes.
- The ASD-STE100 standard requires short sentences. Each sentence has one instruction. This document uses the active voice.

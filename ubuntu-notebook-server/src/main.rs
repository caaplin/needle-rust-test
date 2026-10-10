//! Ubuntu Notebook Server.
//!
//! This server runs notebook commands inside an Ubuntu Docker container. It
//! serves a notebook web page. The browser sends commands to this server. The
//! server runs the commands in the container through the Docker API. The
//! server sends the output back to the browser.
//!
//! The server uses these tools:
//! - Axum. Handles the HTTP requests.
//! - Tokio. Runs asynchronous tasks.
//! - Bollard. Connects to the Docker daemon.
//! - Serde. Serializes and deserializes JSON data.
//! - tower-http. Adds basic authentication and CORS.
//! - axum-server with rustls. Serves HTTPS when certificate files are set.

use std::collections::HashSet;
use std::env;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::{API_DEFAULT_VERSION, Docker};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::validate_request::ValidateRequestHeaderLayer;

/// Name of the Docker container that runs the notebook commands.
const DEFAULT_CONTAINER: &str = "notebook-container";

/// TCP port that the server listens on.
const DEFAULT_PORT: u16 = 8080;

/// Default basic auth credentials. Set NOTEBOOK_USERNAME and NOTEBOOK_PASSWORD
/// to change them.
const DEFAULT_USERNAME: &str = "admin";
const DEFAULT_PASSWORD: &str = "notebook";

/// Default maximum run time for one command, in seconds. Package installs
/// with apt can take several minutes.
const DEFAULT_EXEC_TIMEOUT_SECS: u64 = 600;

/// Commands that the notebook may run by default. This is a whitelist. The
/// first word of every command segment must be in this list. Set the
/// NOTEBOOK_ALLOWED_COMMANDS environment variable to use a custom list.
const DEFAULT_ALLOWED_COMMANDS: &[&str] = &[
    "awk", "base64", "basename", "bc", "bunzip2", "bzcat", "cat", "cd", "chgrp", "chmod", "cmp",
    "cp", "curl", "cut", "date", "df", "diff", "dirname", "du", "echo", "egrep", "env", "expr",
    "false", "file", "find", "fgrep", "git", "grep", "gunzip", "gzip", "head", "help", "history",
    "hostname", "id", "jq", "less", "ln", "ls", "man", "md5sum", "mkdir", "more", "mv", "node",
    "npm", "npx", "ping", "printenv", "printf", "ps", "pwd", "python3", "python", "readlink",
    "realpath", "sed", "seq", "sha1sum", "sha256sum", "sleep", "sort", "ss", "stat", "tail", "tar",
    "tee", "test", "top", "touch", "tr", "tree", "true", "type", "uname", "uniq", "uptime", "wc",
    "wget", "which", "whoami", "xargs", "yes", "zip", "unzip", "zcat",
    // Package management. The notebook user runs these with sudo.
    "sudo", "apt", "apt-get",
];

/// Command patterns that are always rejected, even when the first word of the
/// command is in the allow list. The patterns are matched against the
/// lowercased command. This list is defense in depth. It is not a sandbox.
const BLOCKED_PATTERNS: &[&str] = &[
    ":(){", // fork bomb
    "rm -rf /",
    "rm -rf /*",
    "rm -fr /",
    "rm -rf ~",
    "rm -rf $home",
    "mkfs",
    "fdisk",
    "parted",
    "wipefs",
    "mkswap",
    "dd if=",
    "> /dev/sd",
    "> /dev/nvme",
    "> /dev/vd",
    "of=/dev/sd",
    "of=/dev/nvme",
    "of=/dev/vd",
    "shutdown",
    "reboot",
    "poweroff",
    "halt",
    "init 0",
    "init 6",
    "kill -9 1",
    "kill -9 -1",
    "chmod -r 777 /",
    "chmod 777 /",
    "chown -r",
    "passwd",
    "useradd",
    "userdel",
    "usermod",
    "visudo",
    "| bash",
    "|bash",
    "| sh",
    "|sh",
    "$(sudo",
    "$(sh",
    "$(bash",
    "`sudo",
    "`sh",
    "`bash",
];

/// A notebook cell. A cell is a command cell or a comment cell.
#[derive(Serialize, Deserialize, Clone)]
struct Cell {
    id: String,
    cell_type: String, // "command" or "comment"
    content: String,
    output: Option<String>,
}

/// Request body for POST /api/cells.
#[derive(Deserialize)]
struct CreateCellRequest {
    cell_type: String,
    content: String,
    /// Optional. Insert the new cell after the cell with this id. The new cell
    /// goes to the end of the notebook when this is not set.
    after_id: Option<String>,
}

/// Request body for PUT /api/cells/{id}. All fields are optional.
#[derive(Deserialize)]
struct UpdateCellRequest {
    cell_type: Option<String>,
    content: Option<String>,
    output: Option<String>,
}

/// Request body for POST /api/execute.
#[derive(Deserialize)]
struct ExecuteRequest {
    command: String,
    cell_id: Option<String>,
}

/// Response body for POST /api/execute.
#[derive(Serialize)]
struct ExecuteResponse {
    output: String,
    exit_code: Option<i64>,
    cell_id: Option<String>,
}

/// Response body for errors.
#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

/// Application error. Converts into an HTTP response with a JSON error body.
enum AppError {
    BadRequest(String),
    NotFound(String),
    Forbidden(String),
    Docker(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            AppError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            AppError::Forbidden(message) => (StatusCode::FORBIDDEN, message),
            AppError::Docker(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        (status, Json(ErrorResponse { error: message })).into_response()
    }
}

/// Shared server state.
#[derive(Clone)]
struct AppState {
    docker: Docker,
    container: String,
    cells: Arc<Mutex<Vec<Cell>>>,
    allowed_commands: Arc<HashSet<String>>,
    exec_timeout: Duration,
}

/// The result of one command run in the container.
struct ExecOutcome {
    output: String,
    exit_code: Option<i64>,
}

/// Connect to the Docker daemon.
///
/// Bollard reads the DOCKER_HOST environment variable. This function uses the
/// local Docker socket when DOCKER_HOST is not set or points to a Unix socket.
/// It uses HTTP when DOCKER_HOST points to a tcp://, http:// or https://
/// address. When DOCKER_HOST is not set, Bollard uses the local Docker socket
/// (/var/run/docker.sock on Linux).
fn connect_docker() -> Result<Docker, String> {
    let host = env::var("DOCKER_HOST").unwrap_or_default();
    let result = if host.starts_with("tcp://")
        || host.starts_with("http://")
        || host.starts_with("https://")
    {
        Docker::connect_with_http(&host, 120, API_DEFAULT_VERSION)
    } else {
        Docker::connect_with_local_defaults()
    };
    result.map_err(|e| format!("failed to connect to the Docker daemon: {e}"))
}

/// Load the command allow list from NOTEBOOK_ALLOWED_COMMANDS, or use the
/// default list when the variable is not set.
fn load_allowed_commands() -> HashSet<String> {
    match env::var("NOTEBOOK_ALLOWED_COMMANDS") {
        Ok(value) => value
            .split(',')
            .map(|word| word.trim().to_lowercase())
            .filter(|word| !word.is_empty())
            .collect(),
        Err(_) => DEFAULT_ALLOWED_COMMANDS
            .iter()
            .map(|word| word.to_string())
            .collect(),
    }
}

/// Generate a unique cell id.
fn new_cell_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("cell-{nanos}-{count}")
}

/// Validate a command against the allow list and the block list.
///
/// The command is split into segments on `&&`, `||`, `;`, `|`, newlines,
/// parentheses and backticks. The first word of every segment must be in the
/// allow list. This rule stops `sh` and `bash` from being used to bypass the
/// allow list. The block list is checked against the whole command. It catches
/// dangerous commands inside command substitutions. `sudo` is in the allow
/// list, so the block list is the only check on what runs under sudo.
fn validate_command(command: &str, allowed: &HashSet<String>) -> Result<(), String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("the command is empty".to_string());
    }

    let lowered = trimmed.to_lowercase();
    for pattern in BLOCKED_PATTERNS {
        if lowered.contains(pattern) {
            return Err(format!(
                "the command contains the blocked pattern {pattern:?} and was rejected"
            ));
        }
    }

    // A newline starts a new command for bash. Parentheses and backticks start
    // command substitutions or subshells.
    for segment in trimmed.split(|c: char| matches!(c, '&' | '|' | ';' | '\n' | '(' | ')' | '`')) {
        let mut words = segment.split_whitespace().peekable();
        // Skip leading environment variable assignments, for example
        // `FOO=bar ls`.
        while let Some(word) = words.peek() {
            if word.contains('=') {
                words.next();
            } else {
                break;
            }
        }
        let Some(first) = words.next() else {
            // The segment is empty or only sets variables.
            continue;
        };
        // Allow paths, for example `/bin/ls`.
        let base = first.rsplit('/').next().unwrap_or(first).to_lowercase();
        if !allowed.contains(&base) {
            return Err(format!("the command {first:?} is not in the allow list"));
        }
    }
    Ok(())
}

/// Run one command in the container. Return the combined output and the exit
/// code. The output of stdout and stderr is merged in arrival order.
async fn run_in_container(
    docker: &Docker,
    container: &str,
    command: &str,
    timeout: Duration,
) -> Result<ExecOutcome, AppError> {
    // Step 1. Create the exec instance.
    let config = CreateExecOptions {
        cmd: Some(vec!["bash", "-c", command]),
        attach_stdout: Some(true),
        attach_stderr: Some(true),
        ..Default::default()
    };
    let exec = docker
        .create_exec(container, config)
        .await
        .map_err(|e| {
            AppError::Docker(format!(
                "failed to create an exec instance in container {container:?}: {e}. \
                 Check that the container exists and is running."
            ))
        })?;

    // Step 2. Start the exec instance. This returns a stream of output.
    let started = docker
        .start_exec(&exec.id, None::<StartExecOptions>)
        .await
        .map_err(|e| AppError::Docker(format!("failed to start the exec instance: {e}")))?;
    let StartExecResults::Attached { mut output, .. } = started else {
        return Err(AppError::Docker(
            "the Docker daemon detached the exec session".to_string(),
        ));
    };

    // Step 3. Read the output stream. Stop after the timeout.
    let collect = async {
        let mut text = String::new();
        while let Some(chunk) = output.next().await {
            match chunk {
                Ok(log) => text.push_str(&log.to_string()),
                Err(e) => {
                    return Err(AppError::Docker(format!(
                        "error while reading the command output: {e}"
                    )))
                }
            }
        }
        Ok(text)
    };
    let text = match tokio::time::timeout(timeout, collect).await {
        Ok(result) => result?,
        Err(_) => {
            return Err(AppError::Docker(format!(
                "the command timed out after {} seconds",
                timeout.as_secs()
            )));
        }
    };

    // Step 4. Read the exit code.
    let inspect = docker
        .inspect_exec(&exec.id)
        .await
        .map_err(|e| AppError::Docker(format!("failed to inspect the exec instance: {e}")))?;

    Ok(ExecOutcome {
        output: text,
        exit_code: inspect.exit_code,
    })
}

/// GET /api/cells. Return the list of cells.
async fn list_cells(State(state): State<AppState>) -> Json<Vec<Cell>> {
    let cells = state.cells.lock().unwrap().clone();
    Json(cells)
}

/// POST /api/cells. Add a new cell.
async fn add_cell(
    State(state): State<AppState>,
    Json(request): Json<CreateCellRequest>,
) -> Result<(StatusCode, Json<Cell>), AppError> {
    let cell_type = normalize_cell_type(&request.cell_type)?;
    let cell = Cell {
        id: new_cell_id(),
        cell_type,
        content: request.content,
        output: None,
    };
    let mut cells = state.cells.lock().unwrap();
    let index = request
        .after_id
        .as_ref()
        .and_then(|after_id| cells.iter().position(|item| &item.id == after_id))
        .map(|position| position + 1)
        .unwrap_or(cells.len());
    cells.insert(index, cell.clone());
    Ok((StatusCode::CREATED, Json(cell)))
}

/// PUT /api/cells/{id}. Update a cell.
async fn update_cell(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateCellRequest>,
) -> Result<Json<Cell>, AppError> {
    let mut cells = state.cells.lock().unwrap();
    let cell = cells
        .iter_mut()
        .find(|cell| cell.id == id)
        .ok_or_else(|| AppError::NotFound(format!("cell {id:?} was not found")))?;
    if let Some(cell_type) = request.cell_type {
        cell.cell_type = normalize_cell_type(&cell_type)?;
    }
    if let Some(content) = request.content {
        cell.content = content;
    }
    if let Some(output) = request.output {
        cell.output = Some(output);
    }
    Ok(Json(cell.clone()))
}

/// DELETE /api/cells/{id}. Delete a cell.
async fn delete_cell(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let mut cells = state.cells.lock().unwrap();
    let before = cells.len();
    cells.retain(|cell| cell.id != id);
    if cells.len() == before {
        return Err(AppError::NotFound(format!("cell {id:?} was not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/execute. Run a command in the container.
async fn execute(
    State(state): State<AppState>,
    Json(request): Json<ExecuteRequest>,
) -> Result<Json<ExecuteResponse>, AppError> {
    // Check the command against the allow list before it runs.
    validate_command(&request.command, &state.allowed_commands)
        .map_err(AppError::Forbidden)?;

    // Check that the cell exists before the command runs.
    if let Some(cell_id) = &request.cell_id {
        let cells = state.cells.lock().unwrap();
        if !cells.iter().any(|cell| &cell.id == cell_id) {
            return Err(AppError::NotFound(format!("cell {cell_id:?} was not found")));
        }
    }

    let outcome = run_in_container(
        &state.docker,
        &state.container,
        &request.command,
        state.exec_timeout,
    )
    .await?;

    // Store the output on the cell.
    if let Some(cell_id) = &request.cell_id {
        let mut cells = state.cells.lock().unwrap();
        if let Some(cell) = cells.iter_mut().find(|cell| &cell.id == cell_id) {
            cell.output = Some(outcome.output.clone());
        }
    }

    Ok(Json(ExecuteResponse {
        output: outcome.output,
        exit_code: outcome.exit_code,
        cell_id: request.cell_id,
    }))
}

/// Check a cell type value. Return the normalized value.
fn normalize_cell_type(cell_type: &str) -> Result<String, AppError> {
    let normalized = cell_type.trim().to_lowercase();
    if normalized == "command" || normalized == "comment" {
        Ok(normalized)
    } else {
        Err(AppError::BadRequest(
            "cell_type must be \"command\" or \"comment\"".to_string(),
        ))
    }
}

#[tokio::main]
async fn main() {
    // Read the configuration from environment variables.
    let container =
        env::var("NOTEBOOK_CONTAINER").unwrap_or_else(|_| DEFAULT_CONTAINER.to_string());
    let port: u16 = env::var("NOTEBOOK_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let username = env::var("NOTEBOOK_USERNAME").unwrap_or_else(|_| DEFAULT_USERNAME.to_string());
    let password = env::var("NOTEBOOK_PASSWORD").unwrap_or_else(|_| DEFAULT_PASSWORD.to_string());
    let allowed_commands = load_allowed_commands();
    let exec_timeout = Duration::from_secs(
        env::var("NOTEBOOK_EXEC_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_EXEC_TIMEOUT_SECS),
    );
    let tls_cert = env::var("NOTEBOOK_TLS_CERT").ok();
    let tls_key = env::var("NOTEBOOK_TLS_KEY").ok();

    if env::var("NOTEBOOK_PASSWORD").is_err() {
        eprintln!(
            "WARNING: NOTEBOOK_PASSWORD is not set. The server uses the default password \
             {DEFAULT_PASSWORD:?}. Set NOTEBOOK_USERNAME and NOTEBOOK_PASSWORD before you \
             expose this server."
        );
    }

    // Connect to the Docker daemon.
    let docker = match connect_docker() {
        Ok(docker) => docker,
        Err(message) => {
            eprintln!("{message}");
            eprintln!(
                "Bollard reads the DOCKER_HOST environment variable. Without DOCKER_HOST it \
                 uses the local Docker socket (/var/run/docker.sock on Linux)."
            );
            std::process::exit(1);
        }
    };
    match docker.ping().await {
        Ok(_) => println!("Connected to the Docker daemon."),
        Err(e) => eprintln!("WARNING: could not ping the Docker daemon: {e}"),
    }
    match docker.inspect_container(&container, None).await {
        Ok(info) => {
            let running = info.state.and_then(|state| state.running).unwrap_or(false);
            if running {
                println!("The notebook container {container:?} is running.");
            } else {
                eprintln!(
                    "WARNING: the container {container:?} exists but is not running. \
                     Start it with: docker start {container}"
                );
            }
        }
        Err(_) => eprintln!(
            "WARNING: the container {container:?} was not found. Create it with: \
             docker run -d --name {container} ubuntu-notebook"
        ),
    }

    let state = AppState {
        docker,
        container,
        cells: Arc::new(Mutex::new(Vec::new())),
        allowed_commands: Arc::new(allowed_commands),
        exec_timeout,
    };

    // Build the API routes.
    let api = Router::new()
        .route("/execute", post(execute))
        .route("/cells", get(list_cells).post(add_cell))
        .route("/cells/{id}", put(update_cell).delete(delete_cell));

    // Build the app. Add the routes and the fallback first. Add the
    // middleware afterwards. The last layer is the outermost layer. CORS runs
    // first so that preflight requests do not need authentication.
    let app = Router::new()
        .nest("/api", api)
        .fallback_service(
            ServeDir::new("public").not_found_service(ServeFile::new("public/index.html")),
        )
        .layer(ValidateRequestHeaderLayer::basic(&username, &password))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // Serve HTTPS when certificate files are configured. Serve HTTP otherwise.
    match (tls_cert, tls_key) {
        (Some(cert), Some(key)) => {
            println!("Starting the HTTPS server on https://{addr}");
            let config = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key)
                .await
                .expect("failed to load the TLS certificate or key");
            axum_server::tls_rustls::bind_rustls(addr, config)
                .serve(app.into_make_service())
                .await
                .expect("the HTTPS server failed");
        }
        _ => {
            println!("Starting the HTTP server on http://{addr}");
            println!("Open the notebook in a browser. Sign in with the basic auth credentials.");
            let listener = TcpListener::bind(addr)
                .await
                .expect("failed to bind the server address");
            axum::serve(listener, app)
                .await
                .expect("the HTTP server failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed() -> HashSet<String> {
        DEFAULT_ALLOWED_COMMANDS
            .iter()
            .map(|word| word.to_string())
            .collect()
    }

    #[test]
    fn allows_simple_commands() {
        let allowed = allowed();
        assert!(validate_command("ls -la", &allowed).is_ok());
        assert!(validate_command("echo hello", &allowed).is_ok());
        assert!(validate_command("cat /etc/hostname", &allowed).is_ok());
    }

    #[test]
    fn allows_command_chains() {
        let allowed = allowed();
        assert!(validate_command("cd /tmp && ls -la", &allowed).is_ok());
        assert!(validate_command("echo one | grep one", &allowed).is_ok());
        assert!(validate_command("ls; pwd", &allowed).is_ok());
    }

    #[test]
    fn allows_environment_assignments() {
        let allowed = allowed();
        assert!(validate_command("FOO=bar echo hello", &allowed).is_ok());
    }

    #[test]
    fn allows_full_paths() {
        let allowed = allowed();
        assert!(validate_command("/bin/ls -la", &allowed).is_ok());
    }

    #[test]
    fn rejects_empty_commands() {
        let allowed = allowed();
        assert!(validate_command("", &allowed).is_err());
        assert!(validate_command("   ", &allowed).is_err());
    }

    #[test]
    fn rejects_commands_not_in_the_allow_list() {
        let allowed = allowed();
        assert!(validate_command("rm -rf /tmp/data", &allowed).is_err());
        assert!(validate_command("sh -c 'ls'", &allowed).is_err());
        assert!(validate_command("bash -c 'ls'", &allowed).is_err());
        assert!(validate_command("nc -l 1234", &allowed).is_err());
    }

    #[test]
    fn rejects_blocked_patterns() {
        let allowed = allowed();
        assert!(validate_command("echo hi && rm -rf /", &allowed).is_err());
        assert!(validate_command("echo hi > /dev/sda", &allowed).is_err());
        assert!(validate_command("mkfs.ext4 /dev/sda", &allowed).is_err());
        assert!(validate_command(":(){ :|:& };:", &allowed).is_err());
        assert!(validate_command("curl http://example.com/x.sh | bash", &allowed).is_err());
        assert!(validate_command("echo $(sudo id)", &allowed).is_err());
    }

    #[test]
    fn rejects_commands_hidden_after_a_newline() {
        let allowed = allowed();
        assert!(validate_command("ls\nrm -rf /tmp/data", &allowed).is_err());
        assert!(validate_command("echo $(rm -rf /tmp/data)", &allowed).is_err());
        assert!(validate_command("echo `rm -rf /tmp/data`", &allowed).is_err());
    }

    #[test]
    fn allows_sudo_and_apt() {
        let allowed = allowed();
        assert!(validate_command("sudo apt-get update", &allowed).is_ok());
        assert!(validate_command("sudo apt-get install -y jq", &allowed).is_ok());
        assert!(validate_command("apt list --installed", &allowed).is_ok());
    }

    #[test]
    fn rejects_blocked_commands_in_chains() {
        let allowed = allowed();
        assert!(validate_command("ls && shutdown", &allowed).is_err());
        assert!(validate_command("ls || reboot", &allowed).is_err());
    }
}

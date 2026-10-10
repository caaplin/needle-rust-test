# Ubuntu image for the notebook sandbox.
#
# The Rust server runs notebook commands in this container. Commands run as
# `bash -c <command>`. The Ubuntu base image includes the shell program
# (/bin/bash).
FROM ubuntu:24.04

# Install sudo. The notebook user uses sudo to run apt and other system
# commands.
RUN apt-get update \
    && apt-get install -y --no-install-recommends sudo \
    && rm -rf /var/lib/apt/lists/*

# Create a normal user. The container runs the shell as this user. The user
# has passwordless sudo. Keep DEBIAN_FRONTEND so that apt does not ask
# questions during an install.
RUN useradd --create-home --shell /bin/bash notebook \
    && printf '%s\n' \
        'Defaults env_keep += "DEBIAN_FRONTEND"' \
        'notebook ALL=(ALL) NOPASSWD:ALL' \
        > /etc/sudoers.d/notebook \
    && chmod 0440 /etc/sudoers.d/notebook \
    && visudo -cf /etc/sudoers.d/notebook
USER notebook
WORKDIR /home/notebook

# Keep the container running so that the server can run commands in it.
# Do not publish a port. The Rust server connects to this container through
# the Docker daemon.
CMD ["sleep", "infinity"]

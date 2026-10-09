# Ubuntu image for the notebook sandbox.
#
# The Rust server runs notebook commands in this container. Commands run as
# `bash -c <command>`. The Ubuntu base image includes the shell program
# (/bin/bash).
FROM ubuntu:24.04

# Create a normal user. The container runs the shell as this user. Notebook
# commands cannot run as root.
RUN useradd --create-home --shell /bin/bash notebook
USER notebook
WORKDIR /home/notebook

# Keep the container running so that the server can run commands in it.
# Do not publish a port. The Rust server connects to this container through
# the Docker daemon.
CMD ["sleep", "infinity"]

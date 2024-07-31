FROM rust:1.70.0
WORKDIR /app
COPY . .
# Clone your Rust project from a Git repository
# Build the project using Cargo
RUN cargo build --release

# Define the command to run your Rust binary
CMD ["/app/target/release/operator"]

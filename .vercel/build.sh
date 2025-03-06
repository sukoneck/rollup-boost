#!/bin/bash
set -e

# Set up directories and PATH
mkdir -p $HOME/bin
export PATH=$HOME/bin:$PATH
REPO_ROOT=$(pwd)

# Install Mold linker
echo "Installing Mold linker..."
MOLD_VERSION="2.4.0"
curl -sSL https://github.com/rui314/mold/releases/download/v${MOLD_VERSION}/mold-${MOLD_VERSION}-x86_64-linux.tar.gz | tar -xz
cp mold-${MOLD_VERSION}-x86_64-linux/bin/mold $HOME/bin/
rm -rf mold-${MOLD_VERSION}-x86_64-linux

# Install Rust
echo "Installing Rust..."
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
rustup toolchain install nightly
rustup default nightly

# Configure Rust to use Mold
mkdir -p ~/.cargo
echo '[target.x86_64-unknown-linux-gnu]' >> ~/.cargo/config.toml
echo 'linker = "mold"' >> ~/.cargo/config.toml

# Install mdbook
echo "Installing mdbook..."
curl -sSL https://github.com/rust-lang/mdBook/releases/download/v0.4.14/mdbook-v0.4.14-x86_64-unknown-linux-gnu.tar.gz | tar -xz
chmod +x ./mdbook
cp ./mdbook $HOME/bin/

# Install mdbook-template
echo "Installing mdbook-template..."
curl -sSL https://github.com/sgoudham/mdbook-template/releases/latest/download/mdbook-template-x86_64-unknown-linux-gnu.tar.gz | tar -xz
chmod +x ./mdbook-template
cp ./mdbook-template $HOME/bin/

# Verify installations
echo "Verifying installations..."
which mold
which mdbook
which mdbook-template

# Navigate to book directory and build the book
echo "Building the book..."
cd "${REPO_ROOT}/book"
mdbook build

# Build the Rust docs
echo "Building Rust documentation..."
cd "${REPO_ROOT}"
export RUSTDOCFLAGS="--cfg docsrs --show-type-layout --generate-link-to-definition --enable-index-page -Zunstable-options"
cargo doc --all-features --no-deps

# Move docs to the book folder
echo "Organizing documentation..."
mkdir -p "${REPO_ROOT}/book/book/api"
cp -r target/doc/* "${REPO_ROOT}/book/book/api/"

# Create a final output directory for Vercel
mkdir -p "${REPO_ROOT}/vercel-output"
cp -r "${REPO_ROOT}/book/book"/* "${REPO_ROOT}/vercel-output/"

echo "Build completed successfully!"

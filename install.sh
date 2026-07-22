#!/usr/bin/env bash
# Install script for macOS/Linux.
# Checks required toolchains, then installs project dependencies.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

missing=0

check() {
  local name="$1" cmd="$2" url="$3"
  if command -v "$cmd" >/dev/null 2>&1; then
    printf "${GREEN}ok${NC}    %s (%s)\n" "$name" "$(command -v "$cmd")"
  else
    printf "${RED}missing${NC} %s -- install: %s\n" "$name" "$url"
    missing=1
  fi
}

docker_ok=0

check_docker() {
  if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    printf "${GREEN}ok${NC}    Docker + Docker Compose (%s)\n" "$(command -v docker)"
    docker_ok=1
  else
    printf "${YELLOW}skip${NC}  Docker + Docker Compose -- optional, install: https://docs.docker.com/get-docker/\n"
  fi
}

echo "Checking toolchains..."
check "Node.js/npm" npm "https://nodejs.org"
check "Rust/cargo"  cargo "https://rustup.rs"
check "Aiken"       aiken "https://aiken-lang.org/installation-instructions"
check_docker

if [ "$missing" -ne 0 ]; then
  printf "\n${YELLOW}One or more required tools are missing. Install them, then re-run this script.${NC}\n"
  exit 1
fi

echo
echo "Installing frontend dependencies..."
(cd frontend && npm install)

echo
echo "Building offchain (cargo)..."
(cd offchain && cargo build)

echo
echo "Building onchain validator (aiken)..."
(cd onchain && aiken build)

echo
if [ ! -f offchain/.env ]; then
  printf "${YELLOW}note:${NC} offchain/.env not found. Copy offchain/.env.example to offchain/.env\n"
  printf "      and set BLOCKFROST_PROJECT_ID (or decrypt via: just decrypt-env offchain/.env)\n"
fi

printf "${GREEN}Setup complete.${NC} Run with: just up  (or see README.md)\n"
if [ "$docker_ok" -eq 1 ]; then
  printf "Docker alternative: docker compose up --build\n"
fi

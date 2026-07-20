set shell := ["bash", "-euc"]

offchain_log := "offchain/.offchain.log"
offchain_bin := "target/debug/milestone_escrow_api"

default:
    just --list

# --- onchain -----------------------------------------------------------

onchain-build:
    cd onchain && aiken build

onchain-check:
    cd onchain && aiken check

# --- offchain ------------------------------------------------------------

offchain-build:
    cd offchain && cargo build

offchain-test:
    cd offchain && cargo test

offchain-run:
    cd offchain && cargo run

offchain-up:
    cd offchain && (RUST_LOG=info nohup cargo run > .offchain.log 2>&1 &)
    sleep 2
    just offchain-status

offchain-down:
    pkill -f "^{{offchain_bin}}$" || true

offchain-logs:
    tail -f {{offchain_log}}

offchain-status:
    @curl -s -o /dev/null -w "offchain: %{http_code}\n" http://localhost:3000/grants || true

# --- frontend ------------------------------------------------------------

frontend-install:
    cd frontend && npm install

frontend-up:
    cd frontend && npx astro dev --background

frontend-down:
    cd frontend && npx astro dev stop

frontend-status:
    cd frontend && npx astro dev status

# --- combined dev loop -----------------------------------------------------

setup: frontend-install onchain-build

up: offchain-up frontend-up

# onchain-build compiles the validator (no long-running onchain process to
# start); frontend-up backgrounds itself; offchain-run runs last and stays
# in the foreground so its logs stream here -- Ctrl-C stops it, then
# `just frontend-down` (or `just down`) to stop the rest.
all-up: onchain-build frontend-up
    just offchain-run

down: offchain-down frontend-down

status: offchain-status frontend-status

# --- secrets (sops + age, see README.md) ----------------------------------

decrypt-env file:
    sops --input-type dotenv --output-type dotenv -d {{file}}.enc > {{file}}

encrypt-env file:
    sops --input-type dotenv --output-type dotenv -e {{file}} > {{file}}.enc

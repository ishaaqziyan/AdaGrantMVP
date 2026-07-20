FROM node:22-slim

WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm install
COPY . .

EXPOSE 4321

# .astro/dev.json records the dev-server's PID from its previous run. It can
# outlive the container that wrote it (bind mount, or Compose reusing the
# anonymous volume across recreates) -- and since a fresh container's PIDs
# start low and sequential, the new process can land on that exact same PID.
# Astro then believes a server is "already running" and either refuses to
# start, or (with --force) kills that PID to take over -- which is itself,
# instant self-SIGTERM. Clearing the lock before every start avoids both.
CMD ["sh", "-c", "rm -f .astro/dev.json && exec npx astro dev --host 0.0.0.0"]

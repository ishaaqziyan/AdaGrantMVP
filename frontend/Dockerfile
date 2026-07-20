FROM node:22-slim

WORKDIR /app
COPY package.json package-lock.json* ./
RUN npm install
COPY . .

EXPOSE 4321
CMD ["npx", "astro", "dev", "--host", "0.0.0.0"]

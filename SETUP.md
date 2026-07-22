# Setup (Windows Users)

Three steps.

## 1. Install Docker Desktop

Download from: https://www.docker.com/products/docker-desktop/

Run the installer, accept the defaults, restart your computer if it asks
you to. 
Then open **Docker Desktop** from the Start menu and wait until it
says "Docker Desktop is running" (whale icon in the bottom-left, or in the
system tray).

## 2. Get a free Blockfrost API key

This app reads Cardano blockchain data through a service called Blockfrost.

1. Go to https://blockfrost.io and sign up (free).
2. Create a new project, network **Preview**.
3. Copy the Project ID it gives you (starts with `preview...`).

## 3. Run it

1. Double-click **`start.bat`** in this folder.
2. First run: it creates `offchain\.env` and stops, asking you to edit it.
   - Open `offchain\.env` with Notepad.
   - Replace `previewXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX` with the Project ID
     you copied in step 2.
   - Save and close.
3. Double-click **`start.bat`** again. It will download and build
   everything (first time only, a few minutes), then start the app.
4. Open your browser to **http://localhost:4321**

To stop the app, go back to the black window `start.bat` opened and press
`Ctrl+C`. To start it again later, just double-click `start.bat`.

## If something goes wrong

- **"Docker is not installed"** — reinstall Docker Desktop from step 1.
- **"Docker Desktop is not running"** — open it from the Start menu and
  wait for the whale icon to stop animating.
- Anything else — copy the red text from the black window and send it to
  whoever set this up for you.

// @ts-check
import { defineConfig } from 'astro/config';

// https://astro.build/config
export default defineConfig({
	vite: {
		server: {
			// Docker bind mounts don't reliably deliver inotify events into the
			// container (chokidar's default watcher misses edits made on the
			// host) -- poll for changes instead so `astro dev` picks them up.
			watch: {
				usePolling: true,
			},
		},
	},
});

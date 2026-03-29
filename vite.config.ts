import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import devtools from "solid-devtools/vite";
import { defineConfig, type PluginOption } from "vite";
import solidPlugin from "vite-plugin-solid";

const GuobaHookPlugin = () =>
	({
		name: "guoba-html-hook",
		transformIndexHtml(html: string) {
			return html.replace(
				"</head>",
				`
        <script type="module" src="/@fs/${path.resolve(__dirname, "./src-ui/index.tsx")}"></script>
        </head>
        `,
			);
		},
		configureServer(server) {
			server.middlewares.use((req, res, next) => {
				if (
					req.url?.endsWith("sw.js") ||
					req.url?.endsWith("service-worker.js")
				) {
					res.setHeader("Content-Type", "application/javascript");
					res.end(`
              self.addEventListener('install', () => self.skipWaiting());
              self.addEventListener('activate', () => {
                self.registration.unregister()
                  .then(() => self.clients.matchAll())
                  .then(clients => { clients.forEach(client => client.navigate(client.url)) });
              });
            `);
					return;
				}
				next();
			});
		},
	}) satisfies PluginOption;

export default defineConfig(({ command }) => ({
	root:
		command === "build"
			? undefined
			: path.resolve(__dirname, "./data/panel/Zashboard/v2.4.1"),
	base: command === "build" ? "/guoba" : "/",
	resolve: {
		alias: {
			"@": path.resolve(__dirname, "./src-ui"),
		},
	},
	server: {
		host: true,
	},
	plugins: [GuobaHookPlugin(), devtools(), solidPlugin(), tailwindcss()],
	build: {
		target: "esnext",
		lib: {
			entry: "src-ui/index.tsx",
			formats: ["iife"],
			name: "guoba_ui",
			fileName: () => "__guoba__.js",
		},
	},
	optimizeDeps: {
		include: ["path-browserify"],
	},
}));

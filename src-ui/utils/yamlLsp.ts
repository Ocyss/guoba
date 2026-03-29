import editorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import { LanguageClientWrapper, type LanguageClientConfig } from "monaco-languageclient/lcwrapper";
import { MonacoVscodeApiWrapper } from "monaco-languageclient/vscodeApiWrapper";

const YAML_SCHEMA_URL =
	"https://unpkg.com/meta-json-schema/schemas/meta-json-schema.json";
const YAML_SCHEMA_GLOB = "**/*.y*ml";

type MonacoEnvironmentWithWorker = typeof globalThis & {
	MonacoEnvironment?: {
		getWorker?: (workerId: string, label: string) => Worker;
	};
};

function installMonacoWorkerFactory() {
	const monacoEnv = globalThis as MonacoEnvironmentWithWorker;
	monacoEnv.MonacoEnvironment = {
		...(monacoEnv.MonacoEnvironment ?? {}),
		getWorker: (_workerId: string, _label: string) => new editorWorker(),
	};
}

const apiWrapper = new MonacoVscodeApiWrapper({
	$type: "classic",
	viewsConfig: {
		$type: "EditorService",
	},
	logLevel: 0,
	userConfiguration: {
		json: JSON.stringify({
			"editor.wordBasedSuggestions": "off",
			"editor.experimental.asyncTokenization": true,
		}),
	},
	monacoWorkerFactory: installMonacoWorkerFactory,
	advanced: {
		loadExtensionServices: false,
	},
});

const apiReady = apiWrapper.start();

function buildWsUrl() {
	const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
	return `${protocol}//${window.location.host}/api/editor/ws`;
}

const languageClientConfig: LanguageClientConfig = {
	languageId: "yaml",
	connection: {
		options: {
			$type: "WebSocketUrl",
			url: buildWsUrl(),
		},
	},
	clientOptions: {
		documentSelector: ["yaml"],
		initializationOptions: {
			schemaStore: false,
			schemas: {
				[YAML_SCHEMA_URL]: YAML_SCHEMA_GLOB,
			},
		},
	},
	restartOptions: {
		retries: 3,
		timeout: 1000,
	},
	logLevel: 0,
};

const languageClientWrapper = new LanguageClientWrapper(languageClientConfig);

const eagerStart = (async () => {
	await apiReady;
	try {
		await languageClientWrapper.start();
	} catch (error) {
		console.warn("YAML LSP 启动失败", error);
	}
})();

export async function ensureYamlLspStarted() {
	await eagerStart;
	if (!languageClientWrapper.isStarted()) {
		await languageClientWrapper.start();
	}
}

export async function disposeYamlLsp() {
	await languageClientWrapper.dispose(false);
}

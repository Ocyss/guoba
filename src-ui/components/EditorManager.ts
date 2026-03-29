import * as monaco from "monaco-editor";
import type { EditorType } from "@/utils/types";

export class EditorManager {
	private editors = new Map<EditorType, monaco.editor.IStandaloneCodeEditor>();
	private models = new Map<string, monaco.editor.ITextModel>();
	private toDispose = new Set<() => void>();

	constructor() {
		this.initModels();
	}

	private initModels() {
		const uris = [
			"file:///override.yaml",
			"file:///overlay-view.yaml",
			"file:///subscription-local.yaml",
		];

		for (const uriStr of uris) {
			const uri = monaco.Uri.parse(uriStr);
			const existing = monaco.editor.getModel(uri);
			const model = existing ?? monaco.editor.createModel("", "yaml", uri);
			this.models.set(uriStr, model);
			if (!existing) {
				this.toDispose.add(() => model.dispose());
			}
		}
	}

	async createEditor(
		type: EditorType,
		container: HTMLElement,
		uri: string,
		readOnly: boolean,
	) {
		const existingEditor = this.editors.get(type);
		if (existingEditor) {
			return existingEditor;
		}

		await new Promise((resolve) => {
			if (container.offsetParent !== null) {
				resolve(null);
			} else {
				const checkInterval = setInterval(() => {
					if (container.offsetParent !== null) {
						clearInterval(checkInterval);
						resolve(null);
					}
				}, 10);
			}
		});

		const model = this.models.get(uri);
		if (!model) {
			throw new Error(`Model not found: ${uri}`);
		}

		const editor = monaco.editor.create(container, {
			model,
			automaticLayout: true,
			lineNumbers: "on",
			roundedSelection: false,
			fixedOverflowWidgets: true,
			scrollBeyondLastLine: false,
			readOnly,
			theme: "vs-dark",
			fontSize: readOnly ? 13 : 14,
			lineHeight: 18,
			useShadowDOM: false,
		});

		this.editors.set(type, editor);
		this.toDispose.add(() => editor.dispose());

		return editor;
	}

	getEditor(type: EditorType) {
		return this.editors.get(type);
	}

	disposeEditor(type: EditorType) {
		const editor = this.editors.get(type);
		if (!editor) {
			return;
		}
		editor.dispose();
		this.editors.delete(type);
	}

	getModel(uri: string) {
		return this.models.get(uri);
	}

	getEditorByUri(uri: string) {
		for (const [, editor] of this.editors) {
			if (editor.getModel()?.uri.toString() === uri) {
				return editor;
			}
		}
		return undefined;
	}

	addCommand(
		editorType: EditorType,
		keybinding: number,
		command: () => void | Promise<void>,
	) {
		const editor = this.editors.get(editorType);
		if (editor) {
			editor.addCommand(keybinding, command);
		}
	}

	onDidChangeModelContent(editorType: EditorType, callback: () => void) {
		const editor = this.editors.get(editorType);
		if (editor) {
			const dispose = editor.onDidChangeModelContent(callback);
			this.toDispose.add(() => dispose.dispose());
		}
	}

	relayoutAll() {
		for (const editor of this.editors.values()) {
			editor.layout();
		}
	}

	dispose() {
		for (const disposer of this.toDispose) {
			disposer();
		}
		this.toDispose.clear();
		this.editors.clear();
		this.models.clear();
	}
}

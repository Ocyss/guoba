import {
	type Component,
	createEffect,
	createMemo,
	createSignal,
	onCleanup,
	onMount,
	Show,
} from "solid-js";
import * as monaco from "monaco-editor";
import { disposeYamlLsp, ensureYamlLspStarted } from "@/utils/yamlLsp";
import { EditorManager } from "@/components/EditorManager";
import OverlayTab from "@/components/OverlayTab";
import SubscriptionTab from "@/components/SubscriptionTab";
import CoreTab from "@/components/CoreTab";
import { errorMessage, log, requestApi } from "@/utils/utils";
import type {
	ApplyAction,
	ApplyResult,
	BootstrapData,
	ConfigDocument,
	CoreGithubSyncResult,
	CoreStatus,
	CoreVersionList,
	MainTab,
	OverlayPane,
	PreviewResult,
	SubscriptionContent,
	SubscriptionView,
	ViewMode,
} from "@/utils/types";

// ============ 应用组件 ============
const App: Component = () => {
	let overlayEditorParent!: HTMLDivElement;
	let overlayViewerParent!: HTMLDivElement;
	let subscriptionEditorParent!: HTMLDivElement;

	const editorManager = new EditorManager();
	let renderTimer: number | undefined;

	const [tab, setTab] = createSignal<MainTab>("overlay");
	const [statusText, setStatusText] = createSignal("");
	const [coreStatus, setCoreStatus] = createSignal<CoreStatus | null>(null);
	const [overlayNeedsReload, setOverlayNeedsReload] = createSignal(false);
	const [viewMode, setViewMode] = createSignal<ViewMode>("preview");
	const [overlayPane, setOverlayPane] = createSignal<OverlayPane>("editor");
	const [isNarrow, setIsNarrow] = createSignal(false);
	const [busy, setBusy] = createSignal(false);

	const [runtimeYaml, setRuntimeYaml] = createSignal("");
	const [previewYaml, setPreviewYaml] = createSignal("");
	const [sourceYaml, setSourceYaml] = createSignal("");

	const [subscriptionView, setSubscriptionView] =
		createSignal<SubscriptionView | null>(null);
	const [selectedSubscription, setSelectedSubscription] =
		createSignal<string>("local");
	const [selectedSubscriptionContent, setSelectedSubscriptionContent] =
		createSignal("");
	const isLocalSubscription = createMemo(
		() => selectedSubscription() === (subscriptionView()?.local_id ?? "local"),
	);

	const [coreVersions, setCoreVersions] = createSignal<CoreVersionList | null>(
		null,
	);
	const [installUrl, setInstallUrl] = createSignal("");
	const [keepVersionsInput, setKeepVersionsInput] = createSignal("3");

	const activeYaml = createMemo(() => {
		switch (viewMode()) {
			case "runtime":
				return runtimeYaml();
			case "source":
				return sourceYaml();
			default:
				return previewYaml();
		}
	});

	const initOverlayEditor = async () => {
		if (!overlayEditorParent || editorManager.getEditor("overlay")) {
			return;
		}
		try {
			await editorManager.createEditor(
				"overlay",
				overlayEditorParent,
				"file:///override.yaml",
				false,
			);

			const overlayEditor = editorManager.getEditor("overlay");
			overlayEditor?.addCommand(
				monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS,
				async () => {
					overlayEditor?.getAction("editor.action.formatDocument")?.run();
					await saveOverride();
				},
			);

			editorManager.onDidChangeModelContent("overlay", () => {
				if (renderTimer) {
					window.clearTimeout(renderTimer);
				}
				renderTimer = window.setTimeout(() => {
					void renderPreview();
				}, 220);
			});
		} catch (error) {
			log.warn(`覆写编辑器初始化失败: ${errorMessage(error)}`);
		}
	};

	const initOverlayViewer = async () => {
		if (!overlayViewerParent || editorManager.getEditor("overlayView")) {
			return;
		}
		try {
			await editorManager.createEditor(
				"overlayView",
				overlayViewerParent,
				"file:///overlay-view.yaml",
				true,
			);
		} catch (error) {
			log.warn(`覆写预览编辑器初始化失败: ${errorMessage(error)}`);
		}
	};

	const initSubscriptionEditor = async () => {
		if (!subscriptionEditorParent || editorManager.getEditor("subscription")) {
			return;
		}
		try {
			await editorManager.createEditor(
				"subscription",
				subscriptionEditorParent,
				"file:///subscription-local.yaml",
				true,
			);

			const subscriptionEditor = editorManager.getEditor("subscription");
			subscriptionEditor?.addCommand(
				monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS,
				async () => {
					if (isLocalSubscription()) {
						await saveLocalSubscription();
					}
				},
			);

			subscriptionEditor?.addCommand(
				monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyF,
				() => {
					subscriptionEditor?.getAction("editor.action.formatDocument")?.run();
				},
			);

			editorManager.onDidChangeModelContent("subscription", () => {
				if (isLocalSubscription()) {
					setSelectedSubscriptionContent(subscriptionEditor?.getValue() ?? "");
				}
			});
		} catch (error) {
			log.warn(`订阅编辑器初始化失败: ${errorMessage(error)}`);
		}
	};

	// ============ 初始化编辑器（按容器独立初始化） ============
	createEffect(() => {
		const currentTab = tab();
		const narrow = isNarrow();
		const pane = overlayPane();

		if (currentTab === "overlay") {
			if (!narrow || pane === "editor") {
				void initOverlayEditor();
			}
			if (!narrow || pane === "preview") {
				void initOverlayViewer();
			}
		}

		if (currentTab === "subscription") {
			void initSubscriptionEditor();
		}
	});

	createEffect(() => {
		const currentTab = tab();
		if (currentTab !== "overlay") {
			editorManager.disposeEditor("overlay");
			editorManager.disposeEditor("overlayView");
		}
		if (currentTab !== "subscription") {
			editorManager.disposeEditor("subscription");
		}
	});

	// ============ 生命周期 ============
	onMount(async () => {
		try {
			await ensureYamlLspStarted();
		} catch (error) {
			log.warn(`YAML LSP WS 初始化失败: ${errorMessage(error)}`);
		}

		const media = window.matchMedia("(max-width: 1023.98px)");
		const handleMediaChange = (event: MediaQueryListEvent) => {
			setIsNarrow(event.matches);
		};
		setIsNarrow(media.matches);
		if (media.addEventListener) {
			media.addEventListener("change", handleMediaChange);
		} else {
			media.addListener(handleMediaChange);
		}

		const relayoutEditors = () => {
			editorManager.relayoutAll();
		};

		window.addEventListener("resize", relayoutEditors);
		window.addEventListener(
			"guoba:route-active",
			relayoutEditors as EventListener,
		);
		document.addEventListener("visibilitychange", relayoutEditors);

		await loadBootstrap();
		setTimeout(relayoutEditors, 0);

		onCleanup(() => {
			if (media.removeEventListener) {
				media.removeEventListener("change", handleMediaChange);
			} else {
				media.removeListener(handleMediaChange);
			}
			window.removeEventListener("resize", relayoutEditors);
			window.removeEventListener(
				"guoba:route-active",
				relayoutEditors as EventListener,
			);
			document.removeEventListener("visibilitychange", relayoutEditors);
			if (renderTimer) {
				window.clearTimeout(renderTimer);
			}
			void disposeYamlLsp();
			editorManager.dispose();
		});
	});

	// ============ 动态更新编辑器内容（不重新创建） ============
	createEffect(() => {
		editorManager.getModel("file:///overlay-view.yaml")?.setValue(activeYaml());
	});

	createEffect(() => {
		tab();
		isNarrow();
		overlayPane();
		setTimeout(() => {
			editorManager.relayoutAll();
		}, 0);
	});

	createEffect(() => {
		const subscriptionEditor = editorManager.getEditor("subscription");
		if (!subscriptionEditor) {
			return;
		}
		const nextReadonly = !isLocalSubscription();
		subscriptionEditor.updateOptions({ readOnly: nextReadonly });
		if (nextReadonly) {
			subscriptionEditor.getModel()?.setValue(selectedSubscriptionContent());
		}
	});

	// ============ API 调用和业务逻辑 ============

	const loadBootstrap = async () => {
		setBusy(true);
		try {
			const bootstrap = await requestApi<BootstrapData>("/api/bootstrap");

			editorManager
				.getModel("file:///override.yaml")
				?.setValue(bootstrap.override_doc.content);
			editorManager
				.getModel("file:///subscription-local.yaml")
				?.setValue(bootstrap.subscriptions.local_content);

			setSubscriptionView(bootstrap.subscriptions);
			setCoreVersions(bootstrap.core_versions);
			setKeepVersionsInput(String(bootstrap.core_versions.keep_versions));

			setRuntimeYaml(bootstrap.runtime_yaml);
			setSourceYaml(bootstrap.source_yaml);
			setPreviewYaml(bootstrap.preview_yaml);
			setCoreStatus(bootstrap.status);

			setSelectedSubscription(
				bootstrap.subscriptions.default_id ||
					bootstrap.subscriptions.local_id ||
					"local",
			);
			setSelectedSubscriptionContent(bootstrap.source_yaml);
			setStatusText("");
		} catch (err) {
			setStatusText(errorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	// 监听 selectedSubscription 变化，自动加载订阅内容
	createEffect(() => {
		const selectedId = selectedSubscription();
		if (!selectedId || !subscriptionView()) {
			return;
		}

		// 如果选中本地订阅，使用编辑器的内容
		if (selectedId === (subscriptionView()?.local_id ?? "local")) {
			const content =
				editorManager.getModel("file:///subscription-local.yaml")?.getValue() ??
				"";
			setSelectedSubscriptionContent(content);
			setSourceYaml(content);
			const subscriptionEditor = editorManager.getEditor("subscription");
			subscriptionEditor?.updateOptions({ readOnly: false });
			void renderPreview();
			return;
		}

		// 否则加载远程订阅内容
		(async () => {
			try {
				const data = await requestApi<SubscriptionContent>(
					"/api/subscriptions/content",
					{
						method: "POST",
						body: JSON.stringify({ id: selectedId }),
					},
				);
				setSelectedSubscriptionContent(data.content);
				setSourceYaml(data.content);
				editorManager
					.getModel("file:///subscription-local.yaml")
					?.setValue(data.content);
				const subscriptionEditor = editorManager.getEditor("subscription");
				subscriptionEditor?.updateOptions({ readOnly: true });
				void renderPreview();
			} catch {
				// 静默处理错误，保持 UI 流畅
			}
		})();
	});

	const renderPreview = async () => {
		const overlayEditor = editorManager.getEditor("overlay");
		const overrideContent = overlayEditor?.getValue() ?? "";
		try {
			const result = await requestApi<PreviewResult>("/api/config/preview", {
				method: "POST",
				body: JSON.stringify({
					override_content: overrideContent,
					source_content: sourceYaml(),
				}),
			});
			setPreviewYaml(result.content);
		} catch {
			setPreviewYaml(overrideContent);
		}
	};

	const saveOverride = async () => {
		const overlayEditor = editorManager.getEditor("overlay");
		const content = overlayEditor?.getValue() ?? "";
		setBusy(true);
		setStatusText("保存覆写配置中...");
		try {
			await requestApi<ConfigDocument>("/api/config/override", {
				method: "POST",
				body: JSON.stringify({ content }),
			});
			setOverlayNeedsReload(true);
			await renderPreview();
			setStatusText("覆写配置已保存");
		} catch (err) {
			setStatusText(errorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	const applyConfig = async (action: ApplyAction) => {
		setBusy(true);
		setStatusText(action === "restart" ? "重启内核中..." : "重载内核中...");
		try {
			const overlayEditor = editorManager.getEditor("overlay");
			const overrideContent = overlayEditor?.getValue() ?? "";
			const result = await requestApi<ApplyResult>("/api/config/apply", {
				method: "POST",
				body: JSON.stringify({
					action,
					override_content: overrideContent,
					source_content: sourceYaml(),
					source_id: selectedSubscription(),
				}),
			});
			setRuntimeYaml(result.rendered.content);
			setPreviewYaml(result.rendered.content);
			setCoreStatus(result.core_status);
			setOverlayNeedsReload(false);
			setStatusText(action === "restart" ? "内核已重启" : "内核已重载");
		} catch (err) {
			setStatusText(errorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	const saveLocalSubscription = async () => {
		const content =
			editorManager.getModel("file:///subscription-local.yaml")?.getValue() ??
			"";
		const subscriptionEditor = editorManager.getEditor("subscription");
		setBusy(true);
		setStatusText("保存本地订阅中...");
		try {
			const view = await requestApi<SubscriptionView>(
				"/api/subscriptions/local",
				{
					method: "POST",
					body: JSON.stringify({ content }),
				},
			);
			setSubscriptionView(view);
			setSelectedSubscriptionContent(content);
			setSourceYaml(content);
			await renderPreview();
			subscriptionEditor?.updateOptions({ readOnly: false });
			setStatusText("本地订阅已保存");
		} catch (err) {
			setStatusText(errorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	const addSubscription = async () => {
		const name = window.prompt("请输入订阅名称");
		if (!name?.trim()) {
			return;
		}

		const url = window.prompt("请输入订阅 URL");
		if (!url?.trim()) {
			return;
		}

		const view = subscriptionView();
		if (!view) {
			return;
		}

		setBusy(true);
		setStatusText("添加订阅中...");
		try {
			const newId = `sub-${Date.now()}`;
			const items = [
				...view.items,
				{ id: newId, name: name.trim(), url: url.trim(), enabled: true },
			];
			const saved = await requestApi<SubscriptionView>(
				"/api/subscriptions/items",
				{
					method: "POST",
					body: JSON.stringify({
						interval_minutes: view.interval_minutes,
						keep_versions: view.keep_versions,
						items,
					}),
				},
			);
			setSubscriptionView(saved);
			setSelectedSubscription(newId);
			setStatusText("订阅已添加");
		} catch (err) {
			setStatusText(errorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	const useSelectedSubscription = async () => {
		const selectedId = selectedSubscription();
		if (!selectedId) {
			setStatusText("请选择一个订阅");
			return;
		}

		if (selectedId === (subscriptionView()?.local_id ?? "local")) {
			const content =
				editorManager.getModel("file:///subscription-local.yaml")?.getValue() ??
				"";
			setSelectedSubscriptionContent(content);
			setSourceYaml(content);
			await renderPreview();
			const subscriptionEditor = editorManager.getEditor("subscription");
			subscriptionEditor?.updateOptions({ readOnly: false });
			setStatusText("已使用本地订阅");
			return;
		}

		setBusy(true);
		setStatusText("加载订阅内容中...");
		try {
			const data = await requestApi<SubscriptionContent>(
				"/api/subscriptions/content",
				{
					method: "POST",
					body: JSON.stringify({ id: selectedId }),
				},
			);
			setSelectedSubscriptionContent(data.content);
			setSourceYaml(data.content);
			await renderPreview();
			editorManager
				.getModel("file:///subscription-local.yaml")
				?.setValue(data.content);
			const subscriptionEditor = editorManager.getEditor("subscription");
			subscriptionEditor?.updateOptions({ readOnly: true });
			setStatusText(`已载入订阅: ${data.name}`);
		} catch (err) {
			setStatusText(errorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	const syncCoreFromGithub = async () => {
		setBusy(true);
		setStatusText("从 GitHub 同步中...");
		try {
			const result = await requestApi<CoreGithubSyncResult>(
				"/api/core/sync-github",
			);
			setCoreVersions(result.versions);
			setStatusText(`同步完成: ${result.tag} - ${result.asset_name}`);
		} catch (err) {
			setStatusText(errorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	const setAutoUpdate = async (enabled: boolean) => {
		setBusy(true);
		setStatusText(enabled ? "启用自动更新中..." : "禁用自动更新中...");
		try {
			const updated = await requestApi<CoreVersionList>(
				"/api/core/auto-update",
				{
					method: "POST",
					body: JSON.stringify({ enabled }),
				},
			);
			setCoreVersions(updated);
			setStatusText(enabled ? "自动更新已启用" : "自动更新已禁用");
		} catch (err) {
			setStatusText(errorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	const installCore = async () => {
		const url = installUrl().trim();
		if (!url) {
			setStatusText("请输入下载地址或本地文件路径");
			return;
		}

		setBusy(true);
		setStatusText("安装核心中...");
		try {
			const result = await requestApi<CoreVersionList>("/api/core/install", {
				method: "POST",
				body: JSON.stringify({ url }),
			});
			setCoreVersions(result);
			setInstallUrl("");
			setStatusText("核心已安装并切换");
		} catch (err) {
			setStatusText(errorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	const selectCoreVersion = async (version: string) => {
		setBusy(true);
		setStatusText("切换核心中...");
		try {
			const result = await requestApi<CoreVersionList>("/api/core/select", {
				method: "POST",
				body: JSON.stringify({ version }),
			});
			setCoreVersions(result);
			setStatusText(`已切换到: ${version}`);
		} catch (err) {
			setStatusText(errorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	const setKeepVersions = async () => {
		const keepVersions = parseInt(keepVersionsInput());
		if (Number.isNaN(keepVersions) || keepVersions < 1) {
			setStatusText("请输入有效的保留版本数");
			return;
		}

		setBusy(true);
		setStatusText("更新保留版本配置中...");
		try {
			const result = await requestApi<CoreVersionList>(
				"/api/core/keep-versions",
				{
					method: "POST",
					body: JSON.stringify({ keep_versions: keepVersions }),
				},
			);
			setCoreVersions(result);
			setStatusText("保留版本配置已更新");
		} catch (err) {
			setStatusText(errorMessage(err));
		} finally {
			setBusy(false);
		}
	};

	return (
		<div class="app flex h-full min-h-0 flex-col overflow-hidden bg-base-100 p-4 box-border">
			<div class="mb-2 flex items-center justify-between">
				<div role="tablist" class="tabs tabs-box">
					<input
						type="radio"
						name="tabs"
						role="tab"
						class="tab"
						aria-label="覆写配置"
						checked={tab() === "overlay"}
						onChange={() => setTab("overlay")}
					/>
					<div role="tabpanel" class="tab-content" />

					<input
						type="radio"
						name="tabs"
						role="tab"
						class="tab"
						aria-label="订阅管理"
						checked={tab() === "subscription"}
						onChange={() => setTab("subscription")}
					/>
					<div role="tabpanel" class="tab-content" />

					<input
						type="radio"
						name="tabs"
						role="tab"
						class="tab"
						aria-label="核心管理"
						checked={tab() === "core"}
						onChange={() => setTab("core")}
					/>
					<div role="tabpanel" class="tab-content" />
				</div>
				<div class="flex items-center gap-2">
					<span
						class={`badge relative ${coreStatus()?.running ? "badge-success" : "badge-error"}`}
					>
						{coreStatus()?.running ? "运行中" : "未运行"}
						<Show when={overlayNeedsReload() && coreStatus()?.running}>
							<span
								class="absolute -right-0.5 -top-0.5 h-2.5 w-2.5 rounded-full border border-base-100 bg-warning"
								title="有新配置待重载"
							/>
						</Show>
					</span>
					<button
						type="button"
						class="btn btn-xs btn-info"
						onClick={() => applyConfig("reload")}
						disabled={busy()}
					>
						重载内核
					</button>
					<button
						type="button"
						class="btn btn-xs btn-warning"
						onClick={() => applyConfig("restart")}
						disabled={busy()}
					>
						重启内核
					</button>
					<Show when={busy()}>
						<span class="loading loading-spinner loading-sm" />
					</Show>
					<Show when={statusText()}>
						<span class="badge badge-ghost">{statusText()}</span>
					</Show>
				</div>
			</div>

			{tab() === "overlay" && (
				<OverlayTab
					isNarrow={isNarrow}
					overlayPane={overlayPane}
					setOverlayPane={setOverlayPane}
					viewMode={viewMode}
					setViewMode={setViewMode}
					busy={busy}
					overlayEditorRef={(el) => { overlayEditorParent = el; }}
					overlayViewerRef={(el) => { overlayViewerParent = el; }}
					onFormat={() => {
						editorManager.getEditor("overlay")?.getAction("editor.action.formatDocument")?.run();
					}}
					onSave={saveOverride}
				/>
			)}

			{tab() === "subscription" && (
				<SubscriptionTab
					busy={busy}
					subscriptionView={subscriptionView}
					selectedSubscription={selectedSubscription}
					setSelectedSubscription={setSelectedSubscription}
					isLocalSubscription={isLocalSubscription}
					subscriptionEditorRef={(el) => { subscriptionEditorParent = el; }}
					onUseSelected={useSelectedSubscription}
					onAdd={addSubscription}
					onFormat={() => {
						editorManager.getEditor("subscription")?.getAction("editor.action.formatDocument")?.run();
					}}
					onSave={saveLocalSubscription}
				/>
			)}

			{tab() === "core" && (
				<CoreTab
					busy={busy}
					coreVersions={coreVersions}
					installUrl={installUrl}
					setInstallUrl={setInstallUrl}
					keepVersionsInput={keepVersionsInput}
					setKeepVersionsInput={setKeepVersionsInput}
					onSyncGithub={syncCoreFromGithub}
					onSetAutoUpdate={setAutoUpdate}
					onInstall={installCore}
					onSelectVersion={selectCoreVersion}
					onSetKeepVersions={setKeepVersions}
				/>
			)}
		</div>
	);
};

export default App;

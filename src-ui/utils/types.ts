export type MainTab = "overlay" | "subscription" | "core";
export type ViewMode = "runtime" | "preview" | "source";
export type ApplyAction = "none" | "reload" | "restart";
export type EditorType = "overlay" | "overlayView" | "subscription";
export type OverlayPane = "editor" | "preview";

export type ApiOk<T> = { ok: true; data: T };
export type ApiErr = { ok: false; error: string };
export type ApiResponse<T> = ApiOk<T> | ApiErr;

export type ConfigDocument = {
	path: string;
	content: string;
};

export type RenderResult = {
	source_path: string;
	override_path: string;
	rendered_path: string;
	content: string;
};

export type CoreStatus = {
	running: boolean;
	pid: number | null;
	executable: string | null;
	runtime_config: string;
};

export type ApplyResult = {
	rendered: RenderResult;
	action: ApplyAction;
	core_status: CoreStatus;
};

export type PreviewResult = {
	content: string;
};

export type SubscriptionItem = {
	id: string;
	name: string;
	url: string;
	enabled: boolean;
};

export type SubscriptionView = {
	interval_minutes: number;
	keep_versions: number;
	items: SubscriptionItem[];
	local_id: string;
	default_id: string;
	local_file: string;
	local_content: string;
};

export type CoreVersionItem = {
	version: string;
	file: string;
	downloaded_unix: number;
	released_at: string | null;
	source_tag: string | null;
	is_local: boolean;
	is_current: boolean;
};

export type CoreVersionList = {
	current: string;
	auto_update: boolean;
	keep_versions: number;
	versions: CoreVersionItem[];
};

export type CoreGithubSyncResult = {
	tag: string;
	asset_name: string;
	asset_url: string;
	release_published_at: string | null;
	versions: CoreVersionList;
};

export type BootstrapData = {
	status: CoreStatus;
	override_doc: ConfigDocument;
	subscriptions: SubscriptionView;
	core_versions: CoreVersionList;
	runtime_yaml: string;
	source_yaml: string;
	preview_yaml: string;
};

export type SubscriptionContent = {
	id: string;
	name: string;
	path: string;
	content: string;
};

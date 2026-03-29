import { For, Show } from "solid-js";
import type { CoreVersionList } from "@/utils/types";
import { formatUnix } from "@/utils/utils";

interface CoreTabProps {
	busy: () => boolean;
	coreVersions: () => CoreVersionList | null;
	installUrl: () => string;
	setInstallUrl: (url: string) => void;
	keepVersionsInput: () => string;
	setKeepVersionsInput: (val: string) => void;
	onSyncGithub: () => Promise<void>;
	onSetAutoUpdate: (enabled: boolean) => Promise<void>;
	onInstall: () => Promise<void>;
	onSelectVersion: (version: string) => Promise<void>;
	onSetKeepVersions: () => Promise<void>;
}

const CoreTab = (props: CoreTabProps) => (
	<div class="grid min-h-0 gap-3 lg:grid-cols-[1fr_2fr] h-full">
		<section class="rounded-box border border-base-300 bg-base-200 p-3 gap-3 overflow-hidden flex flex-col">
			<h3 class="mb-2 text-xs font-semibold uppercase opacity-70">
				同步与安装
			</h3>

			<div class="join w-full">
				<button
					type="button"
					class="btn btn-sm btn-info join-item flex-1"
					onClick={props.onSyncGithub}
					disabled={props.busy()}
				>
					从 GitHub 同步
				</button>
				<button
					type="button"
					class={`btn btn-sm join-item ${props.coreVersions()?.auto_update ? "btn-success" : "btn-outline"}`}
					title="每天 4:30 自动更新"
					onClick={() =>
						props.onSetAutoUpdate(!(props.coreVersions()?.auto_update ?? false))
					}
					disabled={props.busy()}
				>
					自动更新 {props.coreVersions()?.auto_update ? "✓" : "✗"}
				</button>
			</div>
			<input
				class="input input-sm input-bordered w-auto"
				placeholder="下载地址或本地路径"
				value={props.installUrl()}
				onInput={(e) => props.setInstallUrl(e.currentTarget.value)}
			/>
			<button
				type="button"
				class="btn btn-sm btn-success w-full"
				onClick={props.onInstall}
				disabled={props.busy()}
			>
				安装并切换
			</button>
			<div class="divider my-1" />
			<div class="space-y-1">
				<label class="text-xs opacity-70" for="keep-versions-input">
					保留版本数
				</label>
				<div class="flex gap-2">
					<input
						id="keep-versions-input"
						type="number"
						min="1"
						class="input input-sm input-bordered flex-1"
						value={props.keepVersionsInput()}
						onInput={(e) => props.setKeepVersionsInput(e.currentTarget.value)}
					/>
					<button
						type="button"
						class="btn btn-sm btn-outline"
						onClick={props.onSetKeepVersions}
						disabled={props.busy()}
					>
						设置
					</button>
				</div>
			</div>
		</section>

		<section class="rounded-box border border-base-300 bg-base-200 p-3 overflow-hidden flex flex-col">
			<h3 class="mb-2 text-xs font-semibold uppercase opacity-70">
				已安装版本
			</h3>
			<div class="flex-1 overflow-y-auto space-y-2">
				{props.coreVersions()?.versions.length ? (
					<For each={props.coreVersions()?.versions}>
						{(item) => (
							<div class="rounded border border-base-300 bg-base-100 p-2 text-xs">
								<div class="flex items-center justify-between mb-1">
									<div class="font-semibold flex items-center gap-2">
										<span>
											{item.is_local ? "🖥️ 本地核心" : item.version}
										</span>
										<Show when={item.is_current}>
											<span class="badge badge-success badge-xs">
												当前
											</span>
										</Show>
									</div>
									<button
										type="button"
										class={`btn btn-xs transition-all ${item.is_current ? "btn-success" : "btn-outline"}`}
										onClick={() => props.onSelectVersion(item.version)}
										disabled={props.busy() || item.is_current}
									>
										{item.is_current ? "已用" : "切换"}
									</button>
								</div>
								<div class="opacity-70 space-y-0.5">
									<Show when={item.released_at}>
										<div>
											发布:
											{new Date(item.released_at || "").toLocaleString()}
										</div>
									</Show>
									<div>下载: {formatUnix(item.downloaded_unix)}</div>
								</div>
							</div>
						)}
					</For>
				) : (
					<div class="text-center opacity-50 text-xs mt-4">
						暂无已安装核心
					</div>
				)}
			</div>
		</section>
	</div>
);

export default CoreTab;

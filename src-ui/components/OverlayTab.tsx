import { Show } from "solid-js";
import type { OverlayPane, ViewMode } from "@/utils/types";

interface OverlayTabProps {
	isNarrow: () => boolean;
	overlayPane: () => OverlayPane;
	setOverlayPane: (pane: OverlayPane) => void;
	viewMode: () => ViewMode;
	setViewMode: (mode: ViewMode) => void;
	busy: () => boolean;
	overlayEditorRef: (el: HTMLDivElement) => void;
	overlayViewerRef: (el: HTMLDivElement) => void;
	onFormat: () => void;
	onSave: () => Promise<void>;
}

const OverlayTab = (props: OverlayTabProps) => (
	<div
		class={
			props.isNarrow()
				? "flex h-full min-h-0 flex-col gap-3"
				: "grid h-full min-h-0 gap-3 lg:grid-cols-2"
		}
	>
		<Show when={props.isNarrow()}>
			<section class="rounded-box border border-base-300 bg-base-200 p-3">
				<div class="join w-full">
					<button
						type="button"
						class={`btn btn-sm join-item flex-1 ${props.overlayPane() === "editor" ? "btn-primary" : "btn-outline"}`}
						onClick={() => props.setOverlayPane("editor")}
					>
						编辑
					</button>
					<button
						type="button"
						class={`btn btn-sm join-item flex-1 ${props.overlayPane() === "preview" ? "btn-primary" : "btn-outline"}`}
						onClick={() => props.setOverlayPane("preview")}
					>
						预览
					</button>
				</div>
			</section>
		</Show>

		<section
			class={`overflow-hidden rounded-box border border-base-300 bg-base-200 p-3 ${props.isNarrow() && props.overlayPane() !== "editor" ? "hidden" : "flex flex-col"}`}
		>
			<h3 class="mb-2 text-xs font-semibold uppercase opacity-70">
				覆写配置编辑
			</h3>
			<div class="mb-2 flex flex-wrap items-center gap-2">
				<button
					type="button"
					class="btn btn-xs btn-outline"
					onClick={props.onFormat}
					disabled={props.busy()}
				>
					格式化
				</button>
				<button
					type="button"
					class="btn btn-xs btn-primary"
					onClick={props.onSave}
					disabled={props.busy()}
				>
					保存 (Ctrl+S)
				</button>
			</div>
			<div
				ref={props.overlayEditorRef}
				class="flex-1 min-h-0 rounded border border-base-300"
			/>
		</section>

		<section
			class={`overflow-hidden rounded-box border border-base-300 bg-base-200 p-3 ${props.isNarrow() && props.overlayPane() !== "preview" ? "hidden" : "flex flex-col"}`}
		>
			<h3 class="mb-2 text-xs font-semibold uppercase opacity-70">
				预览模式
			</h3>
			<div class="mb-2 flex flex-wrap items-center gap-2">
				{(["runtime", "preview", "source"] as const).map((mode) => (
					<button
						type="button"
						class={`btn btn-xs ${props.viewMode() === mode && "btn-active"}`}
						onClick={() => props.setViewMode(mode)}
					>
						{mode === "runtime"
							? "运行时"
							: mode === "preview"
								? "预览"
								: "源"}
					</button>
				))}
			</div>
			<div
				ref={props.overlayViewerRef}
				class="flex-1 min-h-0 rounded border border-base-300"
			/>
		</section>
	</div>
);

export default OverlayTab;

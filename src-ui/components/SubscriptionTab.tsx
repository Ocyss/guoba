import { Show } from "solid-js";
import type { SubscriptionView } from "@/utils/types";

interface SubscriptionTabProps {
	busy: () => boolean;
	subscriptionView: () => SubscriptionView | null;
	selectedSubscription: () => string;
	setSelectedSubscription: (id: string) => void;
	isLocalSubscription: () => boolean;
	subscriptionEditorRef: (el: HTMLDivElement) => void;
	onUseSelected: () => Promise<void>;
	onAdd: () => Promise<void>;
	onFormat: () => void;
	onSave: () => Promise<void>;
}

const SubscriptionTab = (props: SubscriptionTabProps) => (
	<div class="flex-1 min-h-0 overflow-hidden flex flex-col gap-3">
		<section class="rounded-box border border-base-300 bg-base-200 p-3 overflow-hidden">
			<h3 class="mb-2 text-xs font-semibold uppercase opacity-70">
				订阅管理
			</h3>
			<div class="flex flex-row items-center gap-2 mb-2">
				<select
					class="select select-sm select-bordered flex-auto"
					value={props.selectedSubscription()}
					onChange={(e) => props.setSelectedSubscription(e.currentTarget.value)}
				>
					<option value={props.subscriptionView()?.local_id ?? "local"}>
						本地订阅
					</option>
					{props.subscriptionView()?.items.map((item) => (
						<option value={item.id}>{item.name}</option>
					))}
				</select>
				<button
					type="button"
					class="btn btn-sm btn-success w-40"
					onClick={props.onUseSelected}
					disabled={props.busy()}
				>
					使用选中订阅
				</button>
				<button
					type="button"
					class="btn btn-sm btn-primary w-40"
					onClick={props.onAdd}
					disabled={props.busy()}
				>
					+ 添加订阅
				</button>
			</div>
		</section>

		<section class="rounded-box border border-base-300 bg-base-200 flex-1 overflow-hidden flex flex-col min-h-0 p-3 h-full mb-2">
			<h3 class="mb-2 text-xs font-semibold uppercase opacity-70 flex items-center justify-between">
				<span>
					{props.isLocalSubscription()
						? "订阅内容编辑（本地）"
						: "订阅内容查看（远程，只读）"}
				</span>
				<Show when={props.isLocalSubscription()}>
					<div class="flex items-center gap-2">
						<button
							type="button"
							class="btn btn-xs btn-outline"
							onClick={props.onFormat}
							disabled={props.busy()}
						>
							格式化 (Ctrl+F)
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
				</Show>
			</h3>
			<div
				ref={props.subscriptionEditorRef}
				class="flex-1 min-h-0 rounded border border-base-300"
			/>
		</section>
	</div>
);

export default SubscriptionTab;

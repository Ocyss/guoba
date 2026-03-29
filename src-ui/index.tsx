// import hoverStyles from "monaco-editor/esm/vs/editor/contrib/hover/browser/hover.css?inline";
import monacoStyles from "monaco-editor/min/vs/editor/editor.main.css?inline";
import { Dynamic } from "solid-js/web";
import styles from "@/assets/index.css?inline";
import "solid-devtools";
import { customElement } from "solid-element";
import { type Component, createSignal } from "solid-js";
import type { RouteRecordNormalized, Router } from "vue-router";

const GUOBA_ROUTE_PATH = "/guoba";

function isGuobaRouteActive() {
	const routeText = `${window.location.pathname}${window.location.hash}${window.location.search}`;
	return routeText.includes(GUOBA_ROUTE_PATH);
}

function installShortcutGuard() {
	const guardState = window as Window & {
		__guobaShortcutGuardInstalled?: boolean;
	};
	if (guardState.__guobaShortcutGuardInstalled) {
		return;
	}

	const guard = (event: KeyboardEvent) => {
		if (!isGuobaRouteActive()) {
			return;
		}

		if (event.isComposing) {
			return;
		}

		const target = event.target as HTMLElement | null;
		const insideGuoba = Boolean(target?.closest("guoba-manager"));
		if (!insideGuoba) {
			return;
		}

		const shouldBlockHostShortcut =
			event.ctrlKey || event.metaKey || event.altKey;
		if (!shouldBlockHostShortcut) {
			return;
		}

		// Let Monaco process the shortcut first, then stop host-level bubbling.
		event.stopImmediatePropagation();
		event.stopPropagation();
	};

	window.addEventListener("keydown", guard, false);
	guardState.__guobaShortcutGuardInstalled = true;
}

installShortcutGuard();

customElement("guoba-manager", () => {
	const [app, setApp] = createSignal<Component>(() => <div>loading...</div>);

	import("@/App").then((x) => {
		setApp(() => x.default);
	});

	return (
		<>
			<style>{styles}</style>
			<style>{monacoStyles}</style>
			<Dynamic component={app()} />
		</>
	);
});

function injectGuobaMenu() {
	if (document.getElementById("guoba-menu-item")) return;

	const menuContainer = document.querySelector(".sidebar .menu.w-full");
	if (!menuContainer) return;

	const menuItems = menuContainer.querySelectorAll("li");
	if (menuItems.length === 0) return;

	const lastItem = menuItems[menuItems.length - 1];

	const guobaItem = lastItem.cloneNode(true) as HTMLElement;

	guobaItem.id = "guoba-menu-item";
	const aaa = guobaItem.querySelector("a");
	aaa?.classList.remove("menu-active");

	const svgElement = guobaItem.querySelector("svg");
	if (svgElement) {
		svgElement.setAttribute("viewBox", "0 0 48 48");
		svgElement.innerHTML = `
      <g fill="none" stroke="currentColor" stroke-width="4">
        <ellipse cx="24" cy="27" rx="18" ry="17"/><ellipse cx="16.933" cy="24.64" rx="3" ry="4" transform="rotate(15 16.933 24.64)"/>
        <ellipse rx="3" ry="4" transform="scale(-1 1)rotate(15 -109.047 -105.16)"/>
        <path stroke-linecap="round" d="M41 20.999c3.124-3.124 3.438-8.219.313-11.343S32.124 6.876 29 10M7 21c-3.124-3.125-3.468-8.22-.344-11.343C9.781 6.532 15.876 6.875 19 10"/>
        <path stroke-linecap="round" stroke-linejoin="round" d="M20 35c.5 1.294 2.2 3.277 5 .862c2.8 2.415 4.5.431 5-.862"/>
      </g>
    `;
	}

	const textNode = Array.from(guobaItem.querySelectorAll("*")).find(
		(el) => el.children.length === 0 && el.textContent?.includes("设置"),
	);

	if (textNode) {
		textNode.textContent = "锅巴";
	} else {
		guobaItem.innerHTML = guobaItem.innerHTML.replace("设置", "锅巴");
	}
	const router: Router = (document.getElementById("app") as any)?.__vue_app__
		?.config?.globalProperties?.$router;
	if (!router) {
		return;
	}

	if (!router.hasRoute("guoba")) {
		const root = router
			.getRoutes()
			.find((x: any) => x.path === "/") as RouteRecordNormalized;
		router.addRoute({
			...root,
			children: [
				...root.children,
				{
					path: "/guoba",
					name: "guoba",
					component: {
						template: "<guoba-manager></guoba-manager>",
					},
				},
			],
		});
		router.beforeEach((to, from) => {
			if (to.name === "guoba") {
				document
					.querySelector("#guoba-menu-item a")
					?.classList.add("menu-active");
				requestAnimationFrame(() => {
					window.dispatchEvent(new CustomEvent("guoba:route-active"));
				});
			} else if (from.name === "guoba") {
				document
					.querySelector("#guoba-menu-item a")
					?.classList.remove("menu-active");
			}
		});
	}

	guobaItem.onclick = (e) => {
		e.preventDefault();
		e.stopPropagation();
		router.push("/guoba");
	};

	menuContainer.insertBefore(guobaItem, lastItem);
}

const observer = new MutationObserver(() => {
	injectGuobaMenu();
});

observer.observe(document.body, {
	childList: true,
	subtree: true,
});

injectGuobaMenu();

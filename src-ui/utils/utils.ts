import type { ApiResponse } from "./types";

export function formatUnix(unix: number): string {
	if (!unix || unix === 0) return "-";
	return new Date(unix * 1000).toLocaleString("zh-CN");
}

export async function requestApi<T>(url: string, init?: RequestInit): Promise<T> {
	const response = await fetch(url, {
		...init,
		headers: {
			"Content-Type": "application/json",
			...(init?.headers ?? {}),
		},
	});

	const payload = (await response.json()) as ApiResponse<T>;
	if (!response.ok || !payload.ok) {
		throw new Error(
			payload.ok ? `请求失败: ${response.status}` : payload.error,
		);
	}
	return payload.data;
}

export function errorMessage(err: unknown): string {
	if (err instanceof Error) {
		return err.message;
	}
	return String(err);
}

export const log = {
	warn: (msg: string) => console.warn(msg),
	debug: (msg: string) => console.debug(msg),
	error: (msg: string) => console.error(msg),
};

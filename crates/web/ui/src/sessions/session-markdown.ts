import type { HistoryMessage } from "../types";
import type { HistoryPayload } from "./session-history";
import { fetchSessionHistoryViaHttp, mergeHistoryPages } from "./session-history";

const SESSION_EXPORT_PAGE_LIMIT = 500;

interface ContentBlock {
	type?: unknown;
	text?: unknown;
	image_url?: { url?: unknown };
}

interface ValidatedHistoryPage {
	history: HistoryMessage[];
	hasMore: boolean;
	nextCursor?: number;
	totalMessages: number;
}

function markdownLinkDestination(url: string): string {
	return Array.from(url)
		.map((character) => {
			if (character === "(") return "%28";
			if (character === ")") return "%29";
			const codePoint = character.codePointAt(0) ?? 0;
			return codePoint <= 0x20 || codePoint === 0x7f || "<>\\".includes(character)
				? encodeURIComponent(character)
				: character;
		})
		.join("");
}

function contentBlockToMarkdown(block: unknown): string {
	if (typeof block === "string") return block;
	if (!(block && typeof block === "object")) return "";
	const typedBlock = block as ContentBlock;
	if (typedBlock.type === "text" && typeof typedBlock.text === "string") {
		return typedBlock.text;
	}
	if (typedBlock.type === "image_url" && typeof typedBlock.image_url?.url === "string") {
		return `![Image](${markdownLinkDestination(typedBlock.image_url.url)})`;
	}
	return "";
}

function messageContentToMarkdown(content: unknown): string {
	if (typeof content === "string") return content.trim() ? content : "";
	if (!Array.isArray(content)) return "";
	return content
		.map(contentBlockToMarkdown)
		.filter((block) => block.trim())
		.join("\n\n");
}

function inlineTitle(title: string): string {
	return title.replace(/\s+/g, " ").trim() || "Moltis session";
}

export function sessionHistoryToMarkdown(title: string, history: HistoryMessage[]): string {
	const sections = history.flatMap((message) => {
		const role = message.role === "user" ? "User" : message.role === "assistant" ? "Assistant" : null;
		if (!role) return [];
		const content = messageContentToMarkdown(message.content);
		if (!content) return [];
		return [`## ${role}\n\n${content}`];
	});
	return `${[`# ${inlineTitle(title)}`, ...sections].join("\n\n")}\n`;
}

export function sessionMarkdownFilename(title: string): string {
	let stem = Array.from(inlineTitle(title))
		.map((character) => {
			const codePoint = character.codePointAt(0) ?? 0;
			const isBidiControl =
				codePoint === 0x200e ||
				codePoint === 0x200f ||
				(codePoint >= 0x202a && codePoint <= 0x202e) ||
				(codePoint >= 0x2066 && codePoint <= 0x2069);
			return codePoint < 32 || isBidiControl || '<>:"/\\|?*'.includes(character) ? "-" : character;
		})
		.join("")
		.replace(/\s+/g, " ")
		.trim();
	stem = Array.from(stem)
		.slice(0, 100)
		.join("")
		.replace(/[. ]+$/g, "");
	if (!stem) stem = "moltis-session";
	if (/^(aux|con|nul|prn|com[1-9]|lpt[1-9])(?:\.|$)/i.test(stem)) stem = `moltis-${stem}`;
	return `${stem}.md`;
}

function invalidHistoryResponse(): never {
	throw new Error("Failed to load complete session history");
}

function validateHistoryPage(payload: HistoryPayload, expectedTotalMessages: number | null): ValidatedHistoryPage {
	if (!(Array.isArray(payload.history) && payload.history.every((message) => message && typeof message === "object"))) {
		return invalidHistoryResponse();
	}
	if (typeof payload.hasMore !== "boolean") return invalidHistoryResponse();
	if (
		!(typeof payload.totalMessages === "number" && Number.isInteger(payload.totalMessages)) ||
		payload.totalMessages < 0 ||
		(expectedTotalMessages !== null && payload.totalMessages !== expectedTotalMessages)
	) {
		return invalidHistoryResponse();
	}
	return {
		history: payload.history,
		hasMore: payload.hasMore,
		nextCursor: payload.nextCursor,
		totalMessages: payload.totalMessages,
	};
}

function validateNextCursor(nextCursor: number | undefined, currentCursor: number | undefined): number {
	if (
		typeof nextCursor !== "number" ||
		!Number.isInteger(nextCursor) ||
		nextCursor < 0 ||
		(currentCursor !== undefined && nextCursor >= currentCursor)
	) {
		return invalidHistoryResponse();
	}
	return nextCursor;
}

function flattenHistoryPages(pages: HistoryMessage[][]): HistoryMessage[] {
	const olderHistory: HistoryMessage[] = [];
	for (let pageIndex = pages.length - 1; pageIndex > 0; pageIndex -= 1) {
		for (const message of pages[pageIndex]) olderHistory.push(message);
	}
	return mergeHistoryPages(pages[0] || [], olderHistory);
}

export async function fetchCompleteSessionHistory(sessionKey: string): Promise<HistoryMessage[]> {
	const pages: HistoryMessage[][] = [];
	let cursor: number | undefined;
	let expectedTotalMessages: number | null = null;

	for (;;) {
		const payload = validateHistoryPage(
			await fetchSessionHistoryViaHttp(sessionKey, {
				...(cursor === undefined ? {} : { cursor }),
				limit: SESSION_EXPORT_PAGE_LIMIT,
			}),
			expectedTotalMessages,
		);
		expectedTotalMessages ??= payload.totalMessages;
		pages.push(payload.history);
		if (payload.hasMore && payload.history.length === 0) return invalidHistoryResponse();
		if (!payload.hasMore) {
			const history = flattenHistoryPages(pages);
			if (history.length !== expectedTotalMessages) return invalidHistoryResponse();
			return history;
		}

		cursor = validateNextCursor(payload.nextCursor, cursor);
	}
}

function downloadMarkdown(markdown: string, filename: string): void {
	const blob = new Blob([markdown], { type: "text/markdown;charset=utf-8" });
	const url = URL.createObjectURL(blob);
	const link = document.createElement("a");
	link.href = url;
	link.download = filename;
	link.hidden = true;
	document.body.appendChild(link);
	link.click();
	link.remove();
	setTimeout(() => URL.revokeObjectURL(url), 0);
}

export async function saveSessionAsMarkdown(sessionKey: string, title: string): Promise<void> {
	const history = await fetchCompleteSessionHistory(sessionKey);
	downloadMarkdown(sessionHistoryToMarkdown(title, history), sessionMarkdownFilename(title));
}

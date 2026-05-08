// ── Hooks page Traditional Chinese (Taiwan) strings ─────────

export default {
	// ── Page header & intro ─────────────────────────────────
	title: "Hooks",
	reloading: "重新載入中\u2026",
	introDescriptionBody: "在生命週期事件（工具呼叫、訊息、工作階段等）發生時執行 shell 指令。存放於",
	introDescriptionSuffix: "目錄中。",
	introHookMdPrefix: "每個 hook 都是一個包含",
	introHookMdMiddle: "檔案的目錄，內含 TOML frontmatter（事件、指令、需求）及選用的說明文件。編輯下方內容後按一下",
	introHookMdSuffix: "以更新。",
	flowEvent: "事件",
	flowHookScript: "Hook 腳本",
	flowResult: "繼續 / 修改 / 封鎖",

	// ── Empty & loading states ──────────────────────────────
	emptyStatePrefix: "未偵測到 hook。請建立",
	emptyStateSuffix: "以開始使用。",
	loadingHooks: "載入 hooks 中\u2026",

	// ── Status badges ───────────────────────────────────────
	statusIneligible: "不符合",
	statusActive: "使用中",

	// ── Source badges ───────────────────────────────────────
	sourceProject: "專案",
	sourceUser: "使用者",
	sourceBuiltin: "內建",

	// ── Card detail labels ──────────────────────────────────
	eventsLabel: "事件：",
	commandLabel: "指令：",
	priorityLabel: "優先順序：{{value}}",
	timeoutLabel: "逾時：{{value}} 秒",
	clickToCopyPath: "按一下以複製路徑",

	// ── Card stats ──────────────────────────────────────────
	callCount: "{{count}} 次呼叫",
	callCountTitle: "呼叫次數",
	failedCount: "{{count}} 次失敗",
	avgLatency: "平均 {{value}}ms",

	// ── Missing requirements ────────────────────────────────
	missingOs: "不支援此作業系統",
	missingBins: "缺少：{{bins}}",
	missingEnv: "環境變數：{{vars}}",

	// ── Tabs ────────────────────────────────────────────────
	tabPreview: "預覽",
	tabSource: "原始碼",

	// ── Built-in card ───────────────────────────────────────
	viewSourceOnGitHub: "在 GitHub 上檢視原始碼 \u2197",

	// ── Toasts ──────────────────────────────────────────────
	hookEnabled: "已啟用 hook「{{name}}」",
	hookDisabled: "已停用 hook「{{name}}」",
	failedAction: "失敗：{{error}}",
	savedHook: "已儲存「{{name}}」",
	failedToSave: "儲存失敗：{{error}}",
	pathCopied: "已複製路徑",
	hooksReloaded: "已重新載入 hooks",
	reloadFailed: "重新載入失敗：{{error}}",
	unknownError: "未知的錯誤",
};

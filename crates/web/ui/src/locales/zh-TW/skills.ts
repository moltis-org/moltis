// ── Skills page Traditional Chinese (Taiwan) strings ────────

export default {
	// ── Page header ─────────────────────────────────────────
	title: "Skills",
	refresh: "重新整理",
	emergencyDisable: "緊急停用",
	description:
		"從專案、個人及已安裝路徑中探索到的 SKILL.md Skills。匯入的 Skill Pack 會保持隔離，直到明確解除隔離為止。",
	howToWriteSkill: "如何撰寫 Skill？",

	// ── Emergency disable ───────────────────────────────────
	emergencyDisableConfirm: "確定要立即停用所有第三方 Skills 嗎？",
	disableAll: "全部停用",
	emergencyDisableFailed: "緊急停用失敗：{{error}}",
	disabledCount: "已停用 {{count}} 個 Skills",

	// ── Connection ──────────────────────────────────────────
	notConnected: "未連線至閘道。",

	// ── Install ─────────────────────────────────────────────
	installPlaceholder: "owner/repo 或完整 URL（例如 anthropics/skills）",
	install: "安裝",
	installing: "安裝中\u2026",
	installingSource: "正在安裝 {{source}}...",
	installMayTakeWhile: "這可能需要一些時間（下載 + 掃描）。",
	installedSuccess: "已安裝 {{source}}（{{count}} 個 Skills）",
	failedGeneric: "失敗：{{error}}",

	// ── Loading ─────────────────────────────────────────────
	loadingSkills: "載入 Skills 中\u2026",

	// ── Featured section ────────────────────────────────────
	featuredTitle: "精選儲存庫",

	// ── Repos section ───────────────────────────────────────
	reposTitle: "已安裝的儲存庫",
	noRepos: "尚未安裝任何儲存庫。",
	enabledCount: "{{enabled}}/{{total}} 已啟用",
	sha: "sha {{sha}}",
	sourceChanged: "來源已變更",
	orphanedOnDisk: "磁碟上的孤立項目",
	remove: "移除",
	removing: "移除中...",
	searchSkillsIn: "在 {{source}} 中搜尋 Skills\u2026",
	orphanedRepoHint: "孤立的儲存庫：請重新安裝以還原中繼資料",
	noMatchingSkills: "沒有相符的 Skills。",

	// ── Enabled skills table ────────────────────────────────
	enabledTitle: "已啟用的 Skills",
	colName: "名稱",
	colDescription: "說明",
	colSource: "來源",
	deletedSkill: "已刪除 {{name}}",
	disabledSkill: "已停用 {{name}}",
	cannotDisableUnknownSource: "無法停用：未知的 Skill 來源。",
	deleteSkillConfirm: "確定要刪除 Skill「{{name}}」嗎？這會移除 SKILL.md 檔案。",
	disabling: "停用中...",
	deleting: "刪除中...",

	// ── Skill detail panel ──────────────────────────────────
	protected: "受保護",
	protectedCannotDelete: "Skill {{name}} 受到保護，無法從 UI 中刪除",
	trustAndEnable: "信任並啟用",
	trustSkillConfirm: "確定要信任來自 {{source}} 的 Skill「{{name}}」嗎？",
	trustFailed: "信任失敗：{{error}}",
	failedToLoad: "載入失敗：{{error}}",
	skillMdSource: "SKILL.md 原始碼",

	// ── Metadata ────────────────────────────────────────────
	author: "作者：{{name}}",
	commit: "提交：",
	commitAge: "提交時間：{{days}} 天前",
	viewSource: "檢視原始碼",
	allowedTools: "允許的工具：{{tools}}",

	// ── Badges ──────────────────────────────────────────────
	blocked: "已封鎖",
	eligible: "符合資格",
	noDeps: "未宣告相依項目",
	untrusted: "未信任",
	enabled: "已啟用",

	// ── Missing deps ────────────────────────────────────────
	missing: "缺少：{{deps}}",
	installVia: "透過 {{kind}} 安裝",
	installDepConfirm: "確定要為 {{name}} 安裝相依套件嗎？\n\n{{preview}}\n\n僅在信任此 Skill 及其來源時才繼續。",
	installedDep: "已為 {{name}} 安裝相依套件",
	installFailed: "安裝失敗：{{error}}",

	// ── Commit warning ──────────────────────────────────────
	recentCommitWarning: "近期提交警告：",
	recentCommitMessage: "此 Skill 於 {{days}} 天前更新。請將近期更新視為高風險，並在信任／啟用前檢閱差異。",

	// ── Drift warning ───────────────────────────────────────
	driftWarning: "自上次信任後來源已變更；請在重新啟用前檢閱更新內容。",

	// ── Security warning ────────────────────────────────────
	securityTitle: "\u26a0\ufe0f Skills 會在您的機器上執行程式碼 — 請將每個 Skill 都視為不受信任的程式碼",
	securityIntro:
		"Skills 是社群撰寫的指令，AI 代理會<strong>以您的完整系統權限</strong>來遵循執行。熱門程度或下載次數並不代表 Skill 是安全的。惡意 Skill 可能會指示代理：",
	threat1: "在您的機器上執行任意 shell 指令（安裝惡意軟體、加密貨幣挖礦程式、後門程式）",
	threat2: "讀取並竊取敏感資料 — SSH 金鑰、API 權杖、瀏覽器 Cookie、憑證、環境變數",
	threat3: "修改或刪除您檔案系統中的檔案，包括其他專案",
	threat4: "在您不知情的情況下透過 curl/wget 將您的資料傳送至遠端伺服器",
	securityReview:
		"在啟用每個 Skill 前，請仔細檢查其原始碼。請閱讀完整的 SKILL.md 及其參照的所有腳本 — 這些就是代理將代替您執行的指令。不要僅因為某個 Skill 熱門、下載量高或出現在排行榜上就輕易信任。",
	securitySandbox:
		"啟用沙盒模式（Docker、Apple Container 或 cgroup）後，指令執行會被隔離，惡意 Skill 所能造成的損害將大幅受限。",
	dismiss: "關閉",
	disableAllThirdParty: "停用所有第三方 Skills",

	// ── Bundled categories ──────────────────────────────────
	bundledTitle: "內建 Skills 類別",
	bundledDescription: "切換內建 Skills 的類別。停用的類別將不會納入代理的上下文。",

	// ── Featured skill descriptions ─────────────────────────
	featuredOpenClaw: "來自 ClawdHub 的社群 Skills",
	featuredAnthropic: "Anthropic 官方代理 Skills",
	featuredVercelAgent: "Vercel 代理 Skills 集合",
	featuredVercelSkills: "Vercel Skills 工具組",
};

// ── Skills page Chinese (Simplified) strings ────────────────

export default {
	// ── Page header ─────────────────────────────────────────
	title: "技能",
	refresh: "刷新",
	emergencyDisable: "紧急禁用",
	description: "从项目、个人和已安装路径发现的基于 SKILL.md 的技能。",
	howToWriteSkill: "如何编写技能？",

	// ── Emergency disable ───────────────────────────────────
	emergencyDisableConfirm: "立即禁用所有第三方技能？",
	disableAll: "全部禁用",
	emergencyDisableFailed: "紧急禁用失败：{{error}}",
	disabledCount: "已禁用 {{count}} 个技能",

	// ── Connection ──────────────────────────────────────────
	notConnected: "未连接到网关。",

	// ── Install ─────────────────────────────────────────────
	installPlaceholder: "owner/repo 或完整 URL（例如 anthropics/skills）",
	install: "安装",
	installing: "安装中\u2026",
	installingSource: "正在安装 {{source}}...",
	installMayTakeWhile: "这可能需要一些时间（下载 + 扫描）。",
	installedSuccess: "已安装 {{source}}（{{count}} 个技能）",
	failedGeneric: "失败：{{error}}",

	// ── Loading ─────────────────────────────────────────────
	loadingSkills: "加载技能中\u2026",

	// ── Featured section ────────────────────────────────────
	featuredTitle: "精选仓库",

	// ── Repos section ───────────────────────────────────────
	reposTitle: "已安装仓库",
	noRepos: "未安装仓库。",
	enabledCount: "{{enabled}}/{{total}} 个已启用",
	sha: "sha {{sha}}",
	sourceChanged: "源已变更",
	orphanedOnDisk: "磁盘上的孤立文件",
	remove: "移除",
	removing: "移除中...",
	searchSkillsIn: "在 {{source}} 中搜索技能\u2026",
	orphanedRepoHint: "孤立仓库：重新安装以恢复元数据",
	noMatchingSkills: "没有匹配的技能。",

	// ── Enabled skills table ────────────────────────────────
	enabledTitle: "已启用技能",
	colName: "名称",
	colDescription: "描述",
	colSource: "来源",
	deletedSkill: "已删除 {{name}}",
	disabledSkill: "已禁用 {{name}}",
	cannotDisableUnknownSource: "无法禁用：技能来源未知。",
	deleteSkillConfirm: '删除技能 "{{name}}"？这将移除 SKILL.md 文件。',
	disabling: "禁用中...",
	deleting: "删除中...",

	// ── Skill detail panel ──────────────────────────────────
	protected: "受保护",
	protectedCannotDelete: "技能 {{name}} 受保护，无法从 UI 删除",
	trustAndEnable: "信任并启用",
	trustSkillConfirm: '信任来自 {{source}} 的技能 "{{name}}"？',
	trustFailed: "信任失败：{{error}}",
	failedToLoad: "加载失败：{{error}}",
	skillMdSource: "SKILL.md 源代码",

	// ── Metadata ────────────────────────────────────────────
	author: "作者：{{name}}",
	commit: "提交：",
	commitAge: "提交时间：{{days}} 天前",
	viewSource: "查看源代码",
	allowedTools: "允许的工具：{{tools}}",

	// ── Badges ──────────────────────────────────────────────
	blocked: "已阻止",
	eligible: "符合条件",
	noDeps: "无依赖声明",
	untrusted: "不受信任",
	enabled: "已启用",

	// ── Missing deps ────────────────────────────────────────
	missing: "缺少：{{deps}}",
	installVia: "通过 {{kind}} 安装",
	installDepConfirm: "为 {{name}} 安装依赖？\n\n{{preview}}\n\n仅在你信任此技能及其来源时继续。",
	installedDep: "已为 {{name}} 安装依赖",
	installFailed: "安装失败：{{error}}",

	// ── Commit warning ──────────────────────────────────────
	recentCommitWarning: "近期提交警告：",
	recentCommitMessage: "此技能在 {{days}} 天前更新。将近期更新视为高风险，在信任/启用前请审查差异。",

	// ── Drift warning ───────────────────────────────────────
	driftWarning: "自上次信任以来源已变更；请在再次启用前审查更新。",

	// ── Security warning ────────────────────────────────────
	securityTitle: "\u26a0\ufe0f 技能在你的机器上运行代码 \u2014 将每个技能视为不受信任的",
	securityIntro:
		"技能是社区编写的指令，AI 代理以 <strong>你的完整系统权限</strong> 执行。流行度或下载量并不意味着技能是安全的。恶意技能可以指示代理：",
	threat1: "在你的机器上执行任意 shell 命令（安装恶意软件、挖矿程序、后门）",
	threat2: "读取和窃取敏感数据 \u2014 SSH 密钥、API 令牌、浏览器 cookie、凭据、环境变量",
	threat3: "修改或删除整个文件系统中的文件，包括其他项目",
	threat4: "在你不知情的情况下通过 curl/wget 将数据发送到远程服务器",
	securityReview:
		"在启用每个技能之前，请仔细检查源代码。阅读完整的 SKILL.md 及其引用的所有脚本 \u2014 这些是代理将代你执行的确切指令。不要仅因为技能流行、下载量高或出现在排行榜上就信任它。",
	securitySandbox:
		"启用沙盒模式（Docker、Apple Container 或 cgroup）后，命令执行将被隔离，恶意技能造成的损害将大大减少。",
	dismiss: "忽略",
	disableAllThirdParty: "禁用所有第三方技能",

	// ── Featured skill descriptions ─────────────────────────
	featuredOpenClaw: "来自 ClawdHub 的社区技能",
	featuredAnthropic: "Anthropic 官方代理技能",
	featuredVercelAgent: "Vercel 代理技能合集",
	featuredVercelSkills: "Vercel 技能工具包",
};

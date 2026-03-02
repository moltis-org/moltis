// ── Hooks page Chinese (Simplified) strings ─────────────────

export default {
	// ── Page header & intro ─────────────────────────────────
	title: "钩子",
	reloading: "重新加载中\u2026",
	introDescriptionBody: "响应生命周期事件（工具调用、消息、会话等）运行 shell 命令。它们位于",
	introDescriptionSuffix: "目录中。",
	introHookMdPrefix: "每个钩子是一个包含",
	introHookMdMiddle: "文件的目录，其中包含 TOML 前置元数据（事件、命令、要求）和可选文档。编辑下方内容并点击",
	introHookMdSuffix: "来更新。",
	flowEvent: "事件",
	flowHookScript: "钩子脚本",
	flowResult: "继续 / 修改 / 阻止",

	// ── Empty & loading states ──────────────────────────────
	emptyStatePrefix: "未发现钩子。创建一个",
	emptyStateSuffix: "来开始使用。",
	loadingHooks: "加载钩子中\u2026",

	// ── Status badges ───────────────────────────────────────
	statusIneligible: "不符合条件",
	statusActive: "活跃",

	// ── Source badges ───────────────────────────────────────
	sourceProject: "项目",
	sourceUser: "用户",
	sourceBuiltin: "内置",

	// ── Card detail labels ──────────────────────────────────
	eventsLabel: "事件：",
	commandLabel: "命令：",
	priorityLabel: "优先级：{{value}}",
	timeoutLabel: "超时：{{value}}s",
	clickToCopyPath: "点击复制路径",

	// ── Card stats ──────────────────────────────────────────
	callCount: "{{count}} 次调用",
	callCountTitle: "调用次数",
	failedCount: "{{count}} 次失败",
	avgLatency: "平均 {{value}}ms",

	// ── Missing requirements ────────────────────────────────
	missingOs: "不支持此操作系统",
	missingBins: "缺少：{{bins}}",
	missingEnv: "环境变量：{{vars}}",

	// ── Tabs ────────────────────────────────────────────────
	tabPreview: "预览",
	tabSource: "源代码",

	// ── Built-in card ───────────────────────────────────────
	viewSourceOnGitHub: "在 GitHub 上查看源代码 \u2197",

	// ── Toasts ──────────────────────────────────────────────
	hookEnabled: '钩子 "{{name}}" 已启用',
	hookDisabled: '钩子 "{{name}}" 已禁用',
	failedAction: "失败：{{error}}",
	savedHook: '已保存 "{{name}}"',
	failedToSave: "保存失败：{{error}}",
	pathCopied: "路径已复制",
	hooksReloaded: "钩子已重新加载",
	reloadFailed: "重新加载失败：{{error}}",
	unknownError: "未知错误",
};

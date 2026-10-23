// ── Sessions Chinese (Simplified) strings ───────────────────

export default {
	// Welcome card
	welcome: {
		greeting: "你好！",
		greetingWithName: "你好，{{name}}！",
		noProviders: {
			title: "未配置 LLM 供应商",
			description: "配置至少一个供应商以开始聊天。",
		},
	},

	// Session list tabs
	tabs: {
		sessions: "会话",
		cron: "定时任务",
	},

	// Session list
	list: {
		newSession: "新建会话",
		clearAll: "清除",
		clearAllConfirm: "删除 {{count}} 个会话？主会话、Telegram 和定时任务会话将保留。",
		clearAllConfirmPlural: "删除 {{count}} 个会话？主会话、Telegram 和定时任务会话将保留。",
		clearing: "清除中\u2026",
	},

	// Session header
	header: {
		fork: "分叉",
		forkTooltip: "分叉会话",
		share: "分享",
		shareTooltip: "分享快照",
		clear: "清除",
		clearTooltip: "清除会话",
		clearing: "清除中\u2026",
		delete: "删除",
		deleteTooltip: "删除会话",
		deleteConfirm: "删除此会话？",
		deleteWorktreeConfirm: "工作树有未提交的更改。强制删除？",
		renameTooltip: "点击重命名",
		renameSessionTooltip: "重命名会话",
		generateTitleTooltip: "根据对话自动生成标题",
		generateTitleFailed: "自动生成标题失败",
		autoTitle: "自动命名",
		deleteFailed: "删除会话失败",
		stopFailed: "停止回复失败",
		stopTooltip: "停止生成回复",
		stopping: "正在停止\u2026",
		archive: "归档",
		archiveTooltip: "归档会话",
		unarchive: "取消归档",
		unarchiveTooltip: "取消归档会话",
		archiveFailed: "更新归档状态失败",
		markdownSaved: "已将会话保存为 Markdown",
		markdownSaveFailed: "无法将会话保存为 Markdown",
		saveMarkdownTooltip: "保存为 Markdown",
		saveMarkdownLabel: "保存 .md",
		releaseChannel: "解除频道绑定",
		releasing: "正在解除绑定\u2026",
		releaseFailed: "解除频道绑定失败",
		attachedToNamedChannel: "已绑定到 {{channel}}。解除绑定前，运行时无法使用工具或私人上下文。",
		attachedToChannel: "已绑定到频道。解除绑定前，运行时无法使用工具或私人上下文。",
		releasedFromNamedChannel: "已解除与 {{channel}} 的绑定。现在可以再次使用工具和私人上下文。",
		releasedFromChannel: "已解除频道绑定。现在可以再次使用工具和私人上下文。",
		agentSwitchFailed: "切换代理失败",
		nodeSwitchFailed: "切换节点失败",
		defaultAgentSuffix: "（默认）",
		switching: "切换中\u2026",
		local: "本地",
		sshTarget: "SSH：{{target}}",
		sessionAgent: "会话代理",
		sessionNode: "会话节点",
	},

	// Session item (list)
	item: {
		activeTelegram: "活跃 Telegram 会话",
		inactiveTelegram: "Telegram 会话（不活跃）",
		fork: "fork@{{point}}",
	},

	// Session metadata (footer)
	meta: {
		tokenUsage: "{{inTokens}} 输入 / {{outTokens}} 输出",
		modelProvider: "{{provider}} / {{model}}",
	},

	// Session actions
	actions: {
		clearFailed: "清除失败",
	},

	// Share
	share: {
		linkCopied: "分享链接已复制",
		privateNotice: "私密链接包含密钥，仅与信任的人分享",
		createFailed: "创建分享链接失败",
	},

	// Search
	search: {
		noResults: "无结果",
		placeholder: "搜索会话\u2026",
	},

	// Projects
	projects: {
		allSessions: "所有会话",
		noMatching: "没有匹配的项目",
		filterTooltip: "按项目筛选会话",
	},
};

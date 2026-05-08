// ── Crons page Chinese (Simplified) strings ─────────────────

export default {
	// ── Sidebar ─────────────────────────────────────────────
	sidebar: {
		cronJobs: "定时任务",
		heartbeat: "心跳",
	},

	// ── Heartbeat section ───────────────────────────────────
	heartbeat: {
		title: "心跳",
		enable: "启用",
		runNow: "立即运行",
		running: "运行中\u2026",
		description: "定期 AI 检查，监控你的环境并报告状态。",
		inactiveLabel: "心跳未激活：",
		blockedDisabled: "心跳已禁用。启用后才能手动运行。",
		blockedNoPrompt: "心跳未激活，因为未配置提示词。添加自定义提示词或在 HEARTBEAT.md 中写入可执行内容。",
		blockedNoJob: "心跳尚无活跃的定时任务。保存心跳设置以重新创建。",
		lastLabel: "上次：",
		nextLabel: "下次：",
		// Schedule
		scheduleTitle: "调度",
		intervalLabel: "间隔",
		modelLabel: "模型",
		modelDefaultPlaceholder: "（默认：{{model}}）",
		modelServerDefault: "（服务器默认）",
		// Prompt
		promptTitle: "提示词",
		customPromptLabel: "自定义提示词（可选）",
		customPromptPlaceholder: "留空使用默认心跳提示词",
		customPromptHint:
			"留空则使用工作区根目录的 HEARTBEAT.md。如果该文件存在但为空/仅含注释，则跳过心跳 LLM 运行以节省 token。",
		promptSourceLabel: "有效提示词来源：",
		promptSourceConfig: "配置自定义提示词",
		promptSourceMd: "HEARTBEAT.md",
		promptSourceDefault: "无（心跳未激活）",
		maxResponseCharsLabel: "最大响应字符数",
		// Active Hours
		activeHoursTitle: "活跃时段",
		activeHoursDescription: "仅在这些时段运行心跳。",
		startLabel: "开始",
		endLabel: "结束",
		timezoneLabel: "时区",
		timezoneLocal: "本地 ({{tz}})",
		// Sandbox
		sandboxTitle: "沙盒",
		sandboxDescription: "在隔离容器中运行心跳命令。",
		enableSandbox: "启用沙盒",
		sandboxImageLabel: "沙盒镜像",
		sandboxImagePlaceholder: "默认镜像",
		sandboxSearchPlaceholder: "搜索镜像\u2026",
		// Recent Runs
		recentRunsTitle: "最近运行",
		noRunsYet: "暂无运行记录。",
		// Token display
		tokenIn: "{{count}} 输入",
		tokenOut: "{{count}} 输出",
	},

	// ── Cron Jobs section ───────────────────────────────────
	jobs: {
		title: "定时任务",
		addJob: "+ 添加任务",
		noCronJobs: "未配置定时任务。",
		// Status bar
		statusRunning: "运行中",
		statusStopped: "已停止",
		jobCount: "{{count}} 个任务",
		jobCountPlural: "{{count}} 个任务",
		enabledCount: "{{count}} 个已启用",
		nextRun: "下次：{{time}}",
		// Table headers
		headerName: "名称",
		headerSchedule: "调度",
		headerNextRun: "下次运行",
		headerLastStatus: "上次状态",
		headerActions: "操作",
		headerEnabled: "已启用",
		// Actions
		edit: "编辑",
		run: "运行",
		history: "历史",
		// Schedule formatting
		scheduleAt: "在 {{time}}",
		scheduleEveryHours: "每 {{count}} 小时",
		scheduleEveryMinutes: "每 {{count}} 分钟",
		scheduleEverySeconds: "每 {{count}} 秒",
		// Run history panel
		runHistoryTitle: "运行历史：{{name}}",
		noRunsYet: "暂无运行记录。",
		// Delete confirmation
		deleteConfirm: "删除任务 '{{name}}'？",
	},

	// ── Modal (Add / Edit) ──────────────────────────────────
	modal: {
		addJobTitle: "添加任务",
		editJobTitle: "编辑任务",
		nameLabel: "名称",
		namePlaceholder: "任务名称",
		scheduleTypeLabel: "调度类型",
		scheduleAtOption: "定时（一次性）",
		scheduleEveryOption: "间隔（周期性）",
		scheduleCronOption: "Cron（表达式）",
		everyPlaceholder: "间隔秒数",
		cronPlaceholder: "*/5 * * * *",
		timezonePlaceholder: "时区（可选，例如 Asia/Shanghai）",
		payloadTypeLabel: "载荷类型",
		systemEventOption: "系统事件",
		agentTurnOption: "代理轮次",
		messageLabel: "消息",
		messagePlaceholder: "消息文本",
		sessionTargetLabel: "会话目标",
		targetIsolated: "隔离",
		targetMain: "主会话",
		deleteAfterRun: "运行后删除",
		create: "创建",
	},
};

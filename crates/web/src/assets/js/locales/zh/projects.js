// ── Projects page Chinese (Simplified) strings ──────────────

export default {
	title: "仓库",
	description:
		"项目将会话绑定到代码库目录。当会话关联到项目时，上下文文件（CLAUDE.md、AGENTS.md）会自动加载，并可注入自定义系统提示。启用自动工作树可为每个会话创建独立的 git 分支进行隔离工作。",
	autoDetectDescription:
		'<strong class="text-[var(--text)]">自动检测</strong> 扫描你主目录下的常见目录（<code class="font-mono text-xs">~/Projects</code>、<code class="font-mono text-xs">~/Developer</code>、<code class="font-mono text-xs">~/src</code>、<code class="font-mono text-xs">~/code</code>、<code class="font-mono text-xs">~/repos</code>、<code class="font-mono text-xs">~/workspace</code>、<code class="font-mono text-xs">~/dev</code>、<code class="font-mono text-xs">~/git</code>）和 Superset 工作树（<code class="font-mono text-xs">~/.superset/worktrees</code>）中的 git 仓库并添加为项目。',
	clearAllHint: "清除全部仅从 Moltis 中移除仓库条目，不会删除磁盘上的任何内容。",
	noProjectsConfigured: "未配置项目。在上方添加目录或使用自动检测。",
	confirmClearAll: "从 Moltis 清除所有仓库？这仅从列表中移除，不会删除磁盘上的文件。",
	confirmClearAllButton: "全部清除",
	autoDetect: "自动检测",
	detecting: "检测中\u2026",
	clearAll: "全部清除",
	clearing: "清除中\u2026",
	autoDetectTooltip: "扫描常见位置的 git 仓库并添加为项目",
	clearAllTooltip: "从 Moltis 移除所有仓库条目，不删除磁盘上的文件",
	pathInput: {
		directory: "目录",
		placeholder: "/path/to/project",
	},
	badges: {
		auto: "自动",
		worktree: "工作树",
		setup: "初始化",
		teardown: "清理",
		image: "镜像",
	},
	card: {
		systemPromptPrefix: "系统提示：",
		editProject: "编辑项目",
		edit: "编辑",
		removeProject: "移除项目",
	},
	editForm: {
		label: "标签",
		labelPlaceholder: "项目名称",
		directory: "目录",
		directoryPlaceholder: "/path/to/project",
		systemPrompt: "系统提示（可选）",
		systemPromptPlaceholder: "处理此项目时给 LLM 的额外指令...",
		setupCommand: "初始化命令",
		setupCommandPlaceholder: "例如 pnpm install",
		teardownCommand: "清理命令",
		teardownCommandPlaceholder: "例如 docker compose down",
		branchPrefix: "分支前缀",
		branchPrefixPlaceholder: "默认：moltis",
		sandboxImage: "沙盒镜像",
		sandboxImagePlaceholder: "默认 (ubuntu:25.10)",
		autoWorktree: "每个会话自动创建 git 工作树",
	},
};

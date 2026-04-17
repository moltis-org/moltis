// ── Images/Sandboxes page Chinese (Simplified) strings ──────

export default {
	// ── Page-level ──────────────────────────────────────────
	title: "沙盒",
	description:
		"moltis 缓存的用于沙盒执行的容器镜像。你可以删除单个镜像或清理全部。使用基础镜像和 apt 包构建自定义镜像。",
	appleContainerNote:
		"Apple Container 提供 VM 隔离执行，但不支持构建镜像。需要在 Apple Container 旁安装 Docker（或 OrbStack）来构建和缓存自定义镜像。沙盒命令通过 Apple Container 运行；镜像构建使用 Docker。",
	sandboxDisabledHint:
		"在没有容器运行时的云部署中，沙盒功能已禁用。请在装有 Docker 或 Apple Container 的虚拟机上安装以启用此功能。",
	noCachedImages: "没有缓存的镜像。",

	// ── Prune ──────────────────────────────────────────────
	pruneAll: "清理全部",
	pruning: "清理中\u2026",

	// ── Default image selector ─────────────────────────────
	defaultImage: {
		title: "默认镜像",
		description: "用于新会话和项目的基础镜像，除非被覆盖。留空使用内置默认值 (ubuntu:25.10)。",
	},

	// ── Image row ──────────────────────────────────────────
	deleteImage: "删除镜像",

	// ── Build section ──────────────────────────────────────
	build: {
		title: "构建自定义镜像",
		imageNameLabel: "镜像名称",
		baseImageLabel: "基础镜像",
		packagesLabel: "软件包（以空格或换行分隔）",
		buildButton: "构建",
		building: "构建中\u2026",
		buildingImage: "构建镜像中\u2026",
		checkingPackages: "检查基础镜像中的软件包\u2026",
		noPackages: "请指定至少一个软件包。",
		builtTag: "已构建：{{tag}}",
		errorPrefix: "错误：{{message}}",
		allPresent: "所有请求的软件包已存在于 {{base}} 中：{{packages}}。无需构建镜像。",
		alreadyInBase: "已在 {{base}} 中：{{present}}。仅安装：{{missing}}。",
	},

	// ── Backend labels ─────────────────────────────────────
	backend: {
		appleContainer: "Apple Container（VM 隔离）",
		docker: "Docker",
		cgroup: "cgroup (systemd-run)",
		restrictedHost: "受限主机（环境变量 + rlimits）",
		wasm: "Wasmtime（WASM 隔离）",
		none: "无（主机执行）",
		containerBackendLabel: "容器后端：",
	},

	// ── Recommendations ────────────────────────────────────
	recommendation: {
		noRuntimeMacos: "未检测到容器运行时。安装 Apple Container（macOS 26+）以获得 VM 隔离沙盒，或安装 Docker 作为替代。",
		noRuntimeLinux: "未检测到容器运行时。安装 Docker 进行沙盒执行，或确保 systemd 可用以进行 cgroup 隔离。",
		noRuntimeGeneric: "未检测到容器运行时。安装 Docker 进行沙盒执行。",
		macosDockerTip:
			"Apple Container 在 macOS 26+ 上提供更强的 VM 级隔离。安装后会自动使用（moltis 优先选择它而非 Docker）。运行：brew install container",
		linuxDockerTip: "Docker 是 Linux 上的好选择。如需更轻量的隔离且无 Docker 开销，也支持 systemd cgroup 沙盒。",
		restrictedHostTip: "使用受限主机执行（环境变量清除、rlimits）。如需更强隔离，请安装 Docker 或 Apple Container。",
		wasmTip: "使用具有文件系统隔离的 WASM 沙盒。如需容器级隔离，请安装 Docker 或 Apple Container。",
	},

	// ── Alert labels ───────────────────────────────────────
	alertWarning: "警告：",
	alertTip: "提示：",
};

// ── Images/Sandboxes page Traditional Chinese (Taiwan) strings ──

export default {
	// ── Page-level ──────────────────────────────────────────
	title: "沙盒",
	description:
		"moltis 快取的容器映像檔，用於沙盒執行。您可以刪除個別映像檔或清除全部。也可以從基底映像檔搭配 apt 套件建置自訂映像檔。",
	appleContainerNote:
		"Apple Container 提供 VM 隔離執行，但不支援建置映像檔。需要同時安裝 Docker（或 OrbStack）才能建置和快取自訂映像檔。沙盒指令透過 Apple Container 執行；映像檔建置則使用 Docker。",
	sandboxDisabledHint:
		"在沒有容器執行環境的雲端部署中，沙盒功能已停用。請在具有 Docker 或 Apple Container 的 VM 上安裝以啟用此功能。",
	noCachedImages: "沒有快取的映像檔。",

	// ── Prune ──────────────────────────────────────────────
	pruneAll: "清除全部",
	pruning: "清除中\u2026",

	// ── Default image selector ─────────────────────────────
	defaultImage: {
		title: "預設映像檔",
		description: "除非另行指定，否則新的工作階段和專案會使用此基底映像檔。留空以使用內建預設值（ubuntu:25.10）。",
	},

	// ── Image row ──────────────────────────────────────────
	deleteImage: "刪除映像檔",

	// ── Build section ──────────────────────────────────────
	build: {
		title: "建置自訂映像檔",
		imageNameLabel: "映像檔名稱",
		baseImageLabel: "基底映像檔",
		packagesLabel: "套件（以空格或換行分隔）",
		buildButton: "建置",
		building: "建置中\u2026",
		buildingImage: "建置映像檔中\u2026",
		checkingPackages: "檢查基底映像檔中的套件\u2026",
		noPackages: "請至少指定一個套件。",
		builtTag: "已建置：{{tag}}",
		errorPrefix: "錯誤：{{message}}",
		allPresent: "所有請求的套件已存在於 {{base}} 中：{{packages}}。無須建置映像檔。",
		alreadyInBase: "已存在於 {{base}} 中：{{present}}。僅安裝：{{missing}}。",
	},

	// ── Backend labels ─────────────────────────────────────
	backend: {
		appleContainer: "Apple Container（VM 隔離）",
		docker: "Docker",
		cgroup: "cgroup (systemd-run)",
		restrictedHost: "受限主機（env + rlimits）",
		wasm: "Wasmtime（WASM 隔離）",
		none: "無（主機直接執行）",
		containerBackendLabel: "容器後端：",
	},

	// ── Recommendations ────────────────────────────────────
	recommendation: {
		noRuntimeMacos:
			"未偵測到容器執行環境。請安裝 Apple Container（macOS 26+）以獲得 VM 隔離沙盒功能，或安裝 Docker 作為替代方案。",
		noRuntimeLinux: "未偵測到容器執行環境。請安裝 Docker 以執行沙盒，或確認 systemd 可用以啟用 cgroup 隔離。",
		noRuntimeGeneric: "未偵測到容器執行環境。請安裝 Docker 以執行沙盒。",
		macosDockerTip:
			"Apple Container 在 macOS 26+ 上提供更強的 VM 層級隔離。安裝後 moltis 會自動優先使用（優先於 Docker）。執行：brew install container",
		linuxDockerTip:
			"Docker 是 Linux 上的良好選擇。如需更輕量的隔離而不想承擔 Docker 開銷，也可使用 systemd cgroup 沙盒。",
		restrictedHostTip: "目前使用受限主機執行（env 清除、rlimits）。如需更強的隔離，請安裝 Docker 或 Apple Container。",
		wasmTip: "目前使用 WASM 沙盒搭配檔案系統隔離。如需容器等級的隔離，請安裝 Docker 或 Apple Container。",
	},

	// ── Alert labels ───────────────────────────────────────
	alertWarning: "警告：",
	alertTip: "提示：",
};

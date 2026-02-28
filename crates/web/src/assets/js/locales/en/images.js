// ── Images/Sandboxes page English strings ────────────────

export default {
	// ── Page-level ──────────────────────────────────────────
	title: "Sandboxes",
	description:
		"Container images cached by moltis for sandbox execution. You can delete individual images or prune all. Build custom images from a base with apt packages.",
	appleContainerNote:
		"Apple Container provides VM-isolated execution but does not support building images. Docker (or OrbStack) is required alongside Apple Container to build and cache custom images. Sandboxed commands run via Apple Container; image builds use Docker.",
	sandboxDisabledHint:
		"Sandboxes are disabled on cloud deploys without a container runtime. Install on a VM with Docker or Apple Container to enable this feature.",
	noCachedImages: "No cached images.",

	// ── Prune ──────────────────────────────────────────────
	pruneAll: "Prune all",
	pruning: "Pruning\u2026",

	// ── Default image selector ─────────────────────────────
	defaultImage: {
		title: "Default image",
		description:
			"Base image used for new sessions and projects unless overridden. Leave empty to use the built-in default (ubuntu:25.10).",
	},

	// ── Image row ──────────────────────────────────────────
	deleteImage: "Delete image",

	// ── Build section ──────────────────────────────────────
	build: {
		title: "Build custom image",
		imageNameLabel: "Image name",
		baseImageLabel: "Base image",
		packagesLabel: "Packages (space or newline separated)",
		buildButton: "Build",
		building: "Building\u2026",
		buildingImage: "Building image\u2026",
		checkingPackages: "Checking packages in base image\u2026",
		noPackages: "Please specify at least one package.",
		builtTag: "Built: {{tag}}",
		errorPrefix: "Error: {{message}}",
		allPresent: "All requested packages are already present in {{base}}: {{packages}}. No image build needed.",
		alreadyInBase: "Already in {{base}}: {{present}}. Only installing: {{missing}}.",
	},

	// ── Backend labels ─────────────────────────────────────
	backend: {
		appleContainer: "Apple Container (VM-isolated)",
		docker: "Docker",
		cgroup: "cgroup (systemd-run)",
		restrictedHost: "Restricted Host (env + rlimits)",
		wasm: "Wasmtime (WASM-isolated)",
		none: "None (host execution)",
		containerBackendLabel: "Container backend:",
	},

	// ── Recommendations ────────────────────────────────────
	recommendation: {
		noRuntimeMacos:
			"No container runtime detected. Install Apple Container (macOS 26+) for VM-isolated sandboxing, or install Docker as an alternative.",
		noRuntimeLinux:
			"No container runtime detected. Install Docker for sandboxed execution, or ensure systemd is available for cgroup isolation.",
		noRuntimeGeneric: "No container runtime detected. Install Docker for sandboxed execution.",
		macosDockerTip:
			"Apple Container provides stronger VM-level isolation on macOS 26+. Install it for automatic use (moltis prefers it over Docker). Run: brew install container",
		linuxDockerTip:
			"Docker is a good choice on Linux. For lighter-weight isolation without Docker overhead, systemd cgroup sandboxing is also supported.",
		restrictedHostTip:
			"Using restricted host execution (env clearing, rlimits). For stronger isolation, install Docker or Apple Container.",
		wasmTip:
			"Using WASM sandbox with filesystem isolation. For container-level isolation, install Docker or Apple Container.",
	},

	// ── Alert labels ───────────────────────────────────────
	alertWarning: "Warning: ",
	alertTip: "Tip: ",
};

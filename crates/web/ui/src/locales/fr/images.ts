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
		appleContainer: "Apple Container (isolé par VM)",
		docker: "Docker",
		cgroup: "cgroup (systemd-run)",
		restrictedHost: "Hôte restreint (env + rlimits)",
		wasm: "Wasmtime (isolé par WASM)",
		none: "Aucun (exécution sur l'hôte)",
		containerBackendLabel: "Backend de conteneur :",
	},

	// ── Recommendations ────────────────────────────────────
	recommendation: {
		noRuntimeMacos:
			"Aucun runtime de conteneur détecté. Installez Apple Container (macOS 26+) pour une isolation par VM, ou installez Docker comme alternative.",
		noRuntimeLinux:
			"Aucun runtime de conteneur détecté. Installez Docker pour l'exécution en sandbox, ou assurez-vous que systemd est disponible pour l'isolation cgroup.",
		noRuntimeGeneric: "Aucun runtime de conteneur détecté. Installez Docker pour l'exécution en sandbox.",
		macosDockerTip:
			"Apple Container offre une isolation VM plus forte sur macOS 26+. Installez-le pour une utilisation automatique (moltis le préfère à Docker). Exécutez : brew install container",
		linuxDockerTip:
			"Docker est un bon choix sous Linux. Pour une isolation plus légère sans la surcharge de Docker, le sandboxing cgroup systemd est également supporté.",
		restrictedHostTip:
			"Exécution en hôte restreint (nettoyage d'environnement, rlimits). Pour une isolation plus forte, installez Docker ou Apple Container.",
		wasmTip:
			"Utilisation du bac à sable WASM avec isolation du système de fichiers. Pour une isolation conteneur, installez Docker ou Apple Container.",
	},

	// ── Alert labels ───────────────────────────────────────
	alertWarning: "Warning: ",
	alertTip: "Tip: ",
};

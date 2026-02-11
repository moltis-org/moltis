// ── API key help text for providers ───────────────────────

var KEY_SOURCE_BY_PROVIDER = {
	anthropic: {
		url: "https://console.anthropic.com/settings/keys",
		label: "Anthropic Console",
	},
	openai: {
		url: "https://platform.openai.com/api-keys",
		label: "OpenAI Platform",
	},
	gemini: {
		url: "https://aistudio.google.com/app/apikey",
		label: "Google AI Studio",
	},
	groq: {
		url: "https://console.groq.com/keys",
		label: "Groq Console",
	},
	xai: {
		url: "https://console.x.ai/",
		label: "xAI Console",
	},
	deepseek: {
		url: "https://platform.deepseek.com/api_keys",
		label: "DeepSeek Platform",
	},
	mistral: {
		url: "https://console.mistral.ai/api-keys/",
		label: "Mistral Console",
	},
	openrouter: {
		url: "https://openrouter.ai/settings/keys",
		label: "OpenRouter Settings",
	},
	cerebras: {
		url: "https://cloud.cerebras.ai/",
		label: "Cerebras Cloud",
	},
	minimax: {
		url: "https://www.minimax.io/platform",
		label: "MiniMax Platform",
	},
	moonshot: {
		url: "https://platform.moonshot.ai/console/api-keys",
		label: "Moonshot Platform",
	},
	"kimi-code": {
		url: "https://www.kimi.com/code/console",
		label: "Kimi Code Console",
	},
	venice: {
		url: "https://venice.ai/settings/api-keys",
		label: "Venice Settings",
	},
};

export function providerApiKeyHelp(provider) {
	if (!provider || provider.authType !== "api-key") return null;

	if (provider.name === "ollama") {
		return {
			text: "API key is optional for local Ollama. Leave blank unless your gateway requires one.",
		};
	}

	var source = KEY_SOURCE_BY_PROVIDER[provider.name];
	if (source) {
		return {
			text: "Get your key at",
			url: source.url,
			label: source.label,
		};
	}

	return {
		text: `Get your API key from the ${provider.displayName} dashboard.`,
	};
}

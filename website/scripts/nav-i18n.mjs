export const SUPPORTED = ["en", "fr", "zh", "es", "de", "it", "pt", "ja", "ko", "ru"];
export const DEFAULT_LANG = "en";

const NORMAL_CLASS =
	"w-full text-left px-3 py-1.5 text-xs text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700";
const HIGHLIGHT_CLASS =
	"w-full text-left px-3 py-1.5 text-xs font-semibold text-orange-600 dark:text-orange-400 hover:bg-gray-100 dark:hover:bg-gray-700";

export const NAV_I18N = {
	en: {
		langBtn: "EN",
		langTitle: "Change language",
		tabs: {
			home: "Home",
			install: "Install",
			features: "Features",
			security: "Security",
			compare: "Compare",
			changelog: "Changelog",
		},
	},
	fr: {
		langBtn: "FR",
		langTitle: "Changer de langue",
		tabs: {
			home: "Accueil",
			install: "Installer",
			features: "Fonctions",
			security: "Sécurité",
			compare: "Comparer",
			changelog: "Changelog",
		},
	},
	zh: {
		langBtn: "中文",
		langTitle: "切换语言",
		tabs: {
			home: "首页",
			install: "安装",
			features: "功能",
			security: "安全",
			compare: "对比",
			changelog: "更新日志",
		},
	},
	es: {
		langBtn: "ES",
		langTitle: "Cambiar idioma",
		tabs: {
			home: "Inicio",
			install: "Instalar",
			features: "Funciones",
			security: "Seguridad",
			compare: "Comparar",
			changelog: "Changelog",
		},
	},
	de: {
		langBtn: "DE",
		langTitle: "Sprache ändern",
		tabs: {
			home: "Start",
			install: "Installieren",
			features: "Funktionen",
			security: "Sicherheit",
			compare: "Vergleichen",
			changelog: "Changelog",
		},
	},
	it: {
		langBtn: "IT",
		langTitle: "Cambia lingua",
		tabs: {
			home: "Home",
			install: "Installa",
			features: "Funzionalità",
			security: "Sicurezza",
			compare: "Confronta",
			changelog: "Changelog",
		},
	},
	pt: {
		langBtn: "PT",
		langTitle: "Mudar idioma",
		tabs: {
			home: "Início",
			install: "Instalar",
			features: "Recursos",
			security: "Segurança",
			compare: "Comparar",
			changelog: "Changelog",
		},
	},
	ja: {
		langBtn: "JA",
		langTitle: "言語を変更",
		tabs: {
			home: "ホーム",
			install: "インストール",
			features: "機能",
			security: "セキュリティ",
			compare: "比較",
			changelog: "変更履歴",
		},
	},
	ko: {
		langBtn: "KO",
		langTitle: "언어 변경",
		tabs: {
			home: "홈",
			install: "설치",
			features: "기능",
			security: "보안",
			compare: "비교",
			changelog: "변경 로그",
		},
	},
	ru: {
		langBtn: "RU",
		langTitle: "Сменить язык",
		tabs: {
			home: "Главная",
			install: "Установка",
			features: "Возможности",
			security: "Безопасность",
			compare: "Сравнение",
			changelog: "Журнал изменений",
		},
	},
};

function escapeRegExp(value) {
	return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function resolvePageLang(html, fallback = DEFAULT_LANG) {
	const match = html.match(/<html[^>]*\blang="([a-z]{2})"/i);
	const lang = match?.[1]?.toLowerCase();
	return lang && SUPPORTED.includes(lang) ? lang : fallback;
}

export function localizeNavHtml(navHtml, lang) {
	const copy = NAV_I18N[lang] ?? NAV_I18N[DEFAULT_LANG];
	let localized = navHtml;

	localized = localized.replace('title="Change language"', `title="${copy.langTitle}"`);
	localized = localized.replace(
		/(<button onclick="this\.nextElementSibling\.classList\.toggle\('hidden'\)"[^>]*>\s*)EN(\s*<svg width="10")/,
		`$1${copy.langBtn}$2`,
	);

	localized = localized.replaceAll(">Home<", `>${copy.tabs.home}<`);
	localized = localized.replaceAll(">Install<", `>${copy.tabs.install}<`);
	localized = localized.replaceAll(">Features<", `>${copy.tabs.features}<`);
	localized = localized.replaceAll(">Security<", `>${copy.tabs.security}<`);
	localized = localized.replaceAll(">Compare<", `>${copy.tabs.compare}<`);
	localized = localized.replaceAll(">Changelog<", `>${copy.tabs.changelog}<`);

	localized = localized.replace(
		/onclick="setLang\('en'\)" class="[^"]*font-semibold text-orange-600 dark:text-orange-400[^"]*"/,
		`onclick="setLang('en')" class="${NORMAL_CLASS}"`,
	);

	const langClassPattern = new RegExp(
		`(onclick="setLang\\('${lang}'\\)" class=")${escapeRegExp(NORMAL_CLASS)}"`,
	);
	localized = localized.replace(langClassPattern, `$1${HIGHLIGHT_CLASS}"`);

	return localized;
}

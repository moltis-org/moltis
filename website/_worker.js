const SUPPORTED = ['en', 'fr', 'zh', 'es', 'de', 'it', 'pt', 'ja', 'ko', 'ru'];
const DEFAULT_LANG = 'en';

function detectLang(acceptLanguage) {
  if (!acceptLanguage) return DEFAULT_LANG;
  // Parse Accept-Language: fr-FR,fr;q=0.9,en-US;q=0.8,en;q=0.7
  const parts = acceptLanguage.split(',').map(function (p) {
    const [tag, q] = p.trim().split(';q=');
    return { tag: tag.trim().toLowerCase(), q: q ? parseFloat(q) : 1.0 };
  });
  parts.sort(function (a, b) { return b.q - a.q; });
  for (const { tag } of parts) {
    const primary = tag.split('-')[0];
    if (SUPPORTED.includes(primary)) return primary;
  }
  return DEFAULT_LANG;
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (url.pathname === "/") {
      try {
        const cookie = request.headers.get("Cookie") || "";
        const langMatch = cookie.match(/(?:^|;\s*)lang=([a-z]{2})(?:;|$)/);
        let lang = langMatch && SUPPORTED.includes(langMatch[1]) ? langMatch[1] : null;

        if (!lang) {
          lang = detectLang(request.headers.get("Accept-Language"));
        }

        url.pathname = `/index.${lang}.html`;
        const response = await env.ASSETS.fetch(url);
        if (response.ok) {
          return response;
        }
      } catch (_) {
        // Fall through to default static asset serving
      }
    }

    return env.ASSETS.fetch(request);
  },
};

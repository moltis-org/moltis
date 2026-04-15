import {
	DEFAULT_LANG,
	SUPPORTED,
	localizeNavHtml,
	resolvePageLang,
} from "./scripts/nav-i18n.mjs";

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

/** Inject shared partials (<!--NAV-->) into HTML responses. */
async function injectPartials(response, env) {
  const contentType = response.headers.get('content-type') || '';
  if (!contentType.includes('text/html')) return response;

  const html = await response.text();
  if (!html.includes('<!--NAV-->')) return new Response(html, response);

  // Fetch the nav partial from static assets
  const navUrl = new URL('/_partials/nav.html', 'http://localhost');
  const navResponse = await env.ASSETS.fetch(navUrl);
  const navHtml = navResponse.ok ? await navResponse.text() : '';
  const localizedNavHtml = localizeNavHtml(navHtml, resolvePageLang(html));

  const injected = html.replace('<!--NAV-->', localizedNavHtml);
  return new Response(injected, {
    status: response.status,
    headers: response.headers,
  });
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
          return injectPartials(response, env);
        }
      } catch (_) {
        // Fall through to default static asset serving
      }
    }

    const response = await env.ASSETS.fetch(request);
    return injectPartials(response, env);
  },
};

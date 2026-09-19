// Point Swagger UI at this server's live document, so the schema always
// describes the listener you loaded the page from — including whether it wants
// a token. Resolving relative to location.pathname keeps the page working under
// both the root mount and the /v1 mount.
(function () {
  var base = window.location.pathname.replace(/\/docs\/?$/, '');
  var url = base + '/openapi.json';
  var origin = window.location.origin + base;

  function start(spec) {
    var config = {
      dom_id: '#swagger-ui',
      presets: [SwaggerUIBundle.presets.apis],
      layout: 'BaseLayout',
      deepLinking: true,
      displayRequestDuration: true,
      tryItOutEnabled: true,
      defaultModelsExpandDepth: 0,
      docExpansion: 'list',
      // Never write the token to disk; it lives in page memory only.
      persistAuthorization: false,
    };
    if (spec) {
      // Pin the server to the exact origin+prefix this page was served from.
      // The document lists "/" first, which under the /v1 mount would send
      // every "Try it out" to the dashboard's SPA fallback instead of the API.
      spec.servers = [{ url: origin, description: 'This listener' }];
      config.spec = spec;
    } else {
      config.url = url;
    }
    window.ui = SwaggerUIBundle(config);

    var el = document.getElementById('mc-auth');
    if (!el || !spec) return;
    el.textContent = spec['x-bridge-require-token']
      ? 'This listener requires the Mother Claude API token. Use Authorize and paste it without a "Bearer " prefix. Find it in the app under Settings \u2192 Pair a phone, or in the console at startup.'
      : 'Bearer checks are disabled on this loopback listener \u2014 Try it out works without a token.';
  }

  fetch(url, { credentials: 'same-origin' })
    .then(function (r) { return r.ok ? r.json() : null; })
    .then(start)
    .catch(function () { start(null); });
})();

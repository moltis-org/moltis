// E2E compatibility shim — the e2e test helpers locate the app
// via querySelector('script[src*="js/app.js"]') and derive the
// asset prefix from its URL. This file is NOT the real entry point
// (that's dist/main.js) but keeps the e2e helpers working without
// modifications to the test suite.

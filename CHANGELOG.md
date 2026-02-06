# Changelog

## Unreleased

### Changed

- **TLS HTTP redirect port** now defaults to `gateway_port + 1` instead of
  the hardcoded port `18790`. This makes the Dockerfile simpler (both ports
  are adjacent) and avoids collisions when running multiple instances.
  Override via `[tls] http_redirect_port` in `moltis.toml` or the
  `MOLTIS_TLS__HTTP_REDIRECT_PORT` environment variable.

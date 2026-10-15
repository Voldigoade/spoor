# Security

Report vulnerabilities through [GitHub private vulnerability reporting](https://github.com/Voldigoade/spoor/security/advisories/new). Include the affected version, reproduction steps and impact. Keep undisclosed vulnerabilities out of public issues.

## Boundaries

Targets are parsed before provider requests. Only github.com and www.github.com targets are accepted; other hosts, URL credentials, nonstandard ports and ambiguous paths are rejected. Provider clients use configured upstream endpoints. Spoor never executes code or package managers from inspected repositories.

Provider credentials stay on the server and are not included in reports or exports. Avoid logging request credentials when adding diagnostics.

The HTTP limiter stores startup-salted IP hashes in process memory. Raw addresses are processed briefly to derive a key; proxy and hosting logs remain outside this guarantee. Only explicitly configured proxies may supply forwarded client addresses. Keep exact production CORS origins and one replica; see [operations](docs/OPERATIONS.md).

Reports and the SQLite cache contain public metadata, including historical identity evidence. Treat cache access and report sharing accordingly. Public data is not proof of identity or permission to contact someone.

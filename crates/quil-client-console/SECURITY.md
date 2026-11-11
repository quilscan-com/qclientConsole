# Security

## Reporting a vulnerability

Report vulnerabilities through the private security channel provided by the maintainers. Do not post private keys, configuration files, wallet journals, authentication cookies, node addresses, or material that can reproduce a real asset operation in a public issue.

Include the affected version, operating system, minimal reproduction steps, expected behavior, and actual behavior. Redact sensitive samples before sharing them.

## Operating boundaries

- The Web service is designed for trusted local networks, uses HTTP, and must not be exposed directly to the public Internet.
- `config.yml`, `keys.yml`, and files under `~/.qclient-console/` may contain key or recovery material. Restrict their permissions and keep offline backups.
- Mint, transfer, escrow acceptance, and legacy asset migration are on-chain write operations. Use a small amount or test wallet for the first run.
- Do not blindly retry an unknown submission. Check on-chain state first and use an explicit recovery action.
- Obtain binaries only from trusted release channels and verify the published checksums.

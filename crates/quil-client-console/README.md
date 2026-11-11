# qclient-console

`qclient-console` is an interactive Quilibrium client. It adds a terminal UI and a local-network Web console to the complete qclient command set. It does not locate or launch an external `qclient` executable: CLI, TUI, and Web operations all use the same Rust command implementation embedded in the Console binary.

## Running

```bash
# Start the TUI with an automatically discovered node configuration.
# The Web console listens on port 8888 by default.
./qclient-console --signature-check=false

# Start the TUI and Web console with a specific node configuration directory.
./qclient-console --signature-check=false --config-dir /path/to/node/config

# Run a qclient command directly without starting the TUI or Web console.
./qclient-console --signature-check=false token balance --config /path/to/node/config
```

Verify downloaded repository artifacts with the independent Ed25519 verification tool described below, then run this distribution with `--signature-check=false`. Artifact authenticity is separate from the application's inherited official release signature mechanism. This repository's `.sig` files are not inputs to that runtime mechanism.

The Web console listens on `0.0.0.0:8888` by default and is available at `http://server-ip:8888`. On first use, sign in with the fixed `admin` account and set a password. Sessions expire after 30 minutes of inactivity. The Web console is designed for trusted local networks, does not provide TLS, and must not be exposed directly to the public Internet.

## Features

- Node status, prover, shard, diagnostics, log, key, and wallet views.
- One active node and one shared core state across the TUI and Web console.
- Reward minting, transfers, pending escrows, and recovery journals.
- Legacy QUIL migration into an existing or newly generated confidential wallet, including downloads for generated `config.yml` and `keys.yml` files.
- Direct access to qclient Token, Node, Config, Key, Alias, Hypergraph, Message, Deploy, and Compute commands.
- A TUI/Web QClient task center generated from the same Clap command tree, so newly exposed qclient commands do not require a second hand-maintained menu.
- English, Traditional Chinese, Japanese, and French interfaces.

Every public qclient leaf command in the task center runs through the current binary's embedded runner. Native node, wallet, and shard pages remain available as dedicated shortcuts outside the task center. Node, wallet, and system write operations require explicit confirmation. Sensitive fields use masked input and are never shown on confirmation pages. Command output is redacted before entering the shared task store used by the TUI and Web console.

Test asset operations with a small amount or a test wallet first, and keep offline backups of configuration and key files. An unknown submission result is never rebroadcast automatically. Check the on-chain state first, then use the explicit recovery action provided by the interface.

## Source and build layout

This directory is the standalone Console project inside the Quilibrium monorepo. `Cargo.toml` uses relative paths to official sibling crates. Console changes must stay within this directory, but builds still need this project at `crates/quil-client-console` inside a matching monorepo checkout. Copying this directory alone does not include its dependencies.

The Web bundle is written to `web/dist/` and embedded into the Rust binary at compile time. From the repository root:

```bash
cd crates/quil-client-console/web
npm ci
npm run build
cd ../../..
bash scripts/build-qclient-console.sh darwin-arm64 release
```

Use the native platform name `darwin-arm64` or `linux-amd64`. A macOS build needs static FLINT/EMP libraries; set `FLINT_DIR` to the FLINT install root. The helper checks the compiled version against the source version and `subpatch`, runs `--signature-check=false --version`, and rejects non-system macOS dynamic-library dependencies. Independent tests remain local and are not required by a production build.

## Build and artifact signatures

`.github/workflows/qclient-console-release.yml` runs on a push to `main` or a manual workflow dispatch. It builds the Web assets once, then builds Linux amd64 on Ubuntu 22.04 and macOS arm64 on `macos-15`. The final job signs and verifies both binaries, then uploads `qclient-console-release-<version>` as a workflow artifact. It does not publish a GitHub Release.

Configure the repository Actions secret `DEV_NODE_SIGNING_PRIVATE_KEY` with the existing key used by `quilscan-qclient`. Its format is Base64-encoded Go Ed25519 private-key bytes (64 bytes: seed followed by public key). The signer checks it against the public key pinned in `scripts/qclient-console-signing-public-key.b64`. The optional repository variable `QCLIENT_CONSOLE_RELEASE_BASE_URL` records the download base URL in metadata.

The bundle contains both versioned binaries, a `.dgst` SHA3-256 digest and Base64 `.sig` Ed25519 signature for each binary, `qclient-console-version.json`, its signature, and the `qclient-console-release` manifest. Signed metadata records the repository, commit and Actions run. No private key is included in the bundle.

From the repository root, verify an extracted bundle before using its binary:

```bash
bash scripts/verify-qclient-console-artifacts.sh 2.1.0.25.117 /path/to/extracted-bundle
chmod +x /path/to/extracted-bundle/qclient-console-2.1.0.25.117-darwin-arm64
/path/to/extracted-bundle/qclient-console-2.1.0.25.117-darwin-arm64 --signature-check=false
```

Use the version shown in the artifact name. Verification needs Python 3 and OpenSSL 3. If the system `openssl` is older, set `OPENSSL_BIN` to an OpenSSL 3 executable. The verification tool checks signed metadata, both binaries, digest labels, hashes and signatures using the pinned public key. It does not change application settings or enable runtime signature checking.

## Security

Never commit `config.yml`, `keys.yml`, wallet journals, password files, or private keys. See [SECURITY.md](SECURITY.md) for reporting instructions and operating boundaries. This project is licensed under Apache-2.0.

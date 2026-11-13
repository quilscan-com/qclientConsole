# Source provenance

The migrated source baseline is `8c24f839` from the prior Console checkout. Console-specific maintenance and releases now use `quilscan-com/qclientConsole`; upstream native SDK and node release destinations remain owned by their respective projects.

The current functional source is synchronized through `aa70f69c83e51197eb4587a187acc8c862b1c5a2` on the prior Console repository's `v2.1.0.25` branch. The Quilscan build, artifact-signing, publication exclusions, and English-default localization remain maintained separately.

This project is the `quil-client-console` crate in the Quilibrium monorepo. Relative path dependencies in `Cargo.toml` use the matching `quil-client`, `quil-config`, `quil-rpc`, `quil-keys`, `quil-execution`, and other official sibling crates.

Console does not bundle, download, or launch an external qclient executable. CLI mode owns only the Console startup boundary and root argument routing. Command argument types and execution logic come from the `quil-client` dependency. The TUI/Web capability catalog is generated from the same Clap command tree, while native flows reuse the same configuration, key, RPC, proof, and transaction types.

Console-specific update, signature download, verification, and link commands accept only releases from `quilscan-com/qclientConsole` named `qclient-console-<version>-<os>-<arch>`. They do not fall back to qclient release names or installation paths. Sensitive capability arguments exist only for the short execution lifetime, and task output is redacted before it enters the task store shared by the TUI and Web console.

Source releases should identify the corresponding Quilibrium monorepo version or commit and retain `Cargo.lock` plus the embedded `web/dist/` bundle so dependencies and build inputs remain auditable. Internal design notes under `docs/`, user configuration, keys, wallet journals, runtime logs, temporary directories, and build-machine paths are not part of the source release.

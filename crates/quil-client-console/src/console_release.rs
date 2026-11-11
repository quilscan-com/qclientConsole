//! Signed release boundary owned by `qclient-console`.
//!
//! Console releases come only from the Console GitHub repository. The shared
//! qclient verifier still owns version validation, digest labels, signature
//! quorum, atomic installation, and downgrade protection.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use quil_client::release::{self, HttpTimeouts, Release};
use quil_config::{signatory_quorum, SIGNATORIES};
use serde::Deserialize;

pub const RELEASE_TYPE: &str = "qclient-console";
pub const RELEASE_REPOSITORY: &str = "quilscan-com/qclientConsole";
pub const RELEASE_API: &str = "https://api.github.com/repos/quilscan-com/qclientConsole/releases";

const INSTALL_ROOT: &str = "/var/quilibrium/bin/qclient-console";

pub fn release_filename(version: &str, os: &str, arch: &str) -> String {
    format!("{RELEASE_TYPE}-{version}-{os}-{arch}")
}

pub fn default_link() -> PathBuf {
    PathBuf::from("/usr/local/bin/qclient-console")
}

pub fn user_agent() -> String {
    format!("qclient-console/{}", quil_config::VERSION_STRING)
}

pub fn release_metadata_url(version: Option<&str>) -> anyhow::Result<String> {
    match version {
        Some(version) if !version.is_empty() => {
            release::validate_version(version)?;
            Ok(format!("{RELEASE_API}/tags/v{version}"))
        }
        _ => Ok(format!("{RELEASE_API}/latest")),
    }
}

pub fn self_check(
    enabled: bool,
) -> Result<quil_crypto::release_check::SelfCheck, quil_crypto::release_check::ReleaseCheckError> {
    quil_crypto::release_check::self_check(enabled)
}

pub fn verify_current_executable(enabled: bool) -> anyhow::Result<()> {
    use quil_crypto::release_check::{SelfCheck, SELF_CHECK_HINT};

    match self_check(enabled) {
        Ok(SelfCheck::Verified(_)) | Ok(SelfCheck::Unavailable) | Ok(SelfCheck::Disabled) => Ok(()),
        Err(error) => anyhow::bail!(
            "qclient-console signature check failed: {error} — {SELF_CHECK_HINT}; keep the \
             matching qclient-console .dgst and .dgst.sig.N files beside this binary, or run \
             'qclient-console download-signatures'"
        ),
    }
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug)]
struct ReleaseAssets {
    version: String,
    base: String,
    binary_url: String,
    digest_url: String,
    signature_urls: Vec<(usize, String)>,
}

fn no_verifiable_release(detail: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!(
        "no verifiable Console release is available from {RELEASE_REPOSITORY}: {detail}"
    )
}

fn client(timeouts: HttpTimeouts) -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .connect_timeout(timeouts.connect)
        .read_timeout(timeouts.read)
        .timeout(timeouts.total)
        .build()
        .context("build the Console release HTTP client")
}

async fn response_bytes(
    client: &reqwest::Client,
    url: &str,
    description: &str,
) -> anyhow::Result<Vec<u8>> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("download {description} from {url}"))?;
    if !response.status().is_success() {
        anyhow::bail!(
            "download {description} from {url}: HTTP {}",
            response.status()
        );
    }
    Ok(response
        .bytes()
        .await
        .with_context(|| format!("read {description} from {url}"))?
        .to_vec())
}

fn normalized_version(tag: &str) -> anyhow::Result<String> {
    let version = tag.trim().strip_prefix('v').unwrap_or(tag.trim());
    release::validate_version(version)?;
    Ok(version.to_owned())
}

fn select_assets(
    metadata: GithubRelease,
    requested: Option<&str>,
    os: &str,
    arch: &str,
) -> anyhow::Result<ReleaseAssets> {
    let version = normalized_version(&metadata.tag_name)
        .map_err(|error| no_verifiable_release(format!("invalid release tag: {error}")))?;
    if let Some(requested) = requested {
        if requested != version {
            return Err(no_verifiable_release(format!(
                "requested {requested}, but GitHub returned tag {}",
                metadata.tag_name
            )));
        }
    }
    let base = release_filename(&version, os, arch);
    let download_prefix = format!("https://github.com/{RELEASE_REPOSITORY}/releases/download/");
    let mut exact: BTreeMap<String, String> = BTreeMap::new();
    for asset in metadata.assets {
        let is_exact = asset.name == base
            || asset.name == format!("{base}.dgst")
            || asset
                .name
                .strip_prefix(&format!("{base}.dgst.sig."))
                .is_some_and(|suffix| {
                    suffix
                        .parse::<usize>()
                        .is_ok_and(|index| (1..=SIGNATORIES.len()).contains(&index))
                });
        if is_exact {
            if !asset.browser_download_url.starts_with(&download_prefix) {
                return Err(no_verifiable_release(format!(
                    "asset {} has an unexpected download URL",
                    asset.name
                )));
            }
            if exact
                .insert(asset.name.clone(), asset.browser_download_url)
                .is_some()
            {
                return Err(no_verifiable_release(format!(
                    "duplicate GitHub asset {}",
                    asset.name
                )));
            }
        }
    }
    let binary_url = exact
        .remove(&base)
        .ok_or_else(|| no_verifiable_release(format!("missing asset {base}")))?;
    let digest_name = format!("{base}.dgst");
    let digest_url = exact
        .remove(&digest_name)
        .ok_or_else(|| no_verifiable_release(format!("missing asset {digest_name}")))?;
    let signature_prefix = format!("{base}.dgst.sig.");
    let signature_urls: Vec<_> = exact
        .into_iter()
        .filter_map(|(name, url)| {
            name.strip_prefix(&signature_prefix)
                .and_then(|index| index.parse::<usize>().ok())
                .map(|index| (index, url))
        })
        .collect();
    if signature_urls.len() < signatory_quorum() {
        return Err(no_verifiable_release(format!(
            "only {} signature asset(s), need at least {}",
            signature_urls.len(),
            signatory_quorum()
        )));
    }
    Ok(ReleaseAssets {
        version,
        base,
        binary_url,
        digest_url,
        signature_urls,
    })
}

async fn release_assets(version: Option<&str>) -> anyhow::Result<(reqwest::Client, ReleaseAssets)> {
    if let Some(version) = version.filter(|version| !version.is_empty()) {
        release::validate_version(version)?;
    }
    let client = client(release::HTTP_TIMEOUTS)?;
    let url = release_metadata_url(version)?;
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("query Console release metadata at {url}"))?;
    if !response.status().is_success() {
        return Err(no_verifiable_release(format!(
            "GitHub metadata request returned HTTP {}",
            response.status()
        )));
    }
    let metadata: GithubRelease = response
        .json()
        .await
        .with_context(|| format!("parse Console release metadata from {url}"))?;
    let (os, arch) = quil_client::system::system_info()?;
    let assets = select_assets(metadata, version, os, arch)?;
    Ok((client, assets))
}

async fn fetch_signatures(
    client: &reqwest::Client,
    assets: &ReleaseAssets,
) -> anyhow::Result<(Vec<u8>, Vec<(usize, Vec<u8>)>)> {
    let digest = response_bytes(client, &assets.digest_url, "Console release digest").await?;
    let mut signatures = Vec::with_capacity(assets.signature_urls.len());
    for (index, url) in &assets.signature_urls {
        signatures.push((
            *index,
            response_bytes(client, url, &format!("Console release signature {index}")).await?,
        ));
    }
    Ok((digest, signatures))
}

fn install_dir(version: &str) -> PathBuf {
    Path::new(INSTALL_ROOT).join(version)
}

fn signature_dir(exe: Option<&Path>, base: &str, fallback: &Path) -> PathBuf {
    exe.and_then(|exe| std::fs::canonicalize(exe).ok())
        .filter(|exe| exe.file_name().is_some_and(|name| name == base))
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| fallback.to_path_buf())
}

pub async fn download_signatures(version: Option<&str>) -> anyhow::Result<()> {
    let version = version
        .filter(|version| !version.is_empty())
        .unwrap_or(quil_config::VERSION_STRING);
    let (client, assets) = release_assets(Some(version)).await?;
    let (digest, signatures) = fetch_signatures(&client, &assets).await?;
    let valid = release::verify_quorum(&digest, &signatures)?;
    let quorum = signatory_quorum();
    if valid < quorum {
        return Err(no_verifiable_release(format!(
            "signature quorum not met ({valid}/{quorum})"
        )));
    }
    release::check_digest_label(&digest, &assets.base)?;
    let destination = signature_dir(
        std::env::current_exe().ok().as_deref(),
        &assets.base,
        &install_dir(&assets.version),
    );
    release::write_signature_files(&destination, &assets.base, &digest, &signatures)?;
    println!(
        "Wrote verified qclient-console {} signatures to {}",
        assets.version,
        destination.display()
    );
    Ok(())
}

pub async fn update(version: Option<&str>, allow_downgrade: bool) -> anyhow::Result<()> {
    let (client, assets) = release_assets(version).await?;
    let link = default_link();
    let installed = release::linked_version(&link, RELEASE_TYPE);
    release::check_downgrade(installed.as_deref(), &assets.version, allow_downgrade)?;
    release::check_unversioned_link(&link, RELEASE_TYPE, &assets.version, allow_downgrade)?;
    let destination = install_dir(&assets.version);
    let binary_path = destination.join(&assets.base);
    if release::is_linked_and_verified_with(&binary_path, &link, &SIGNATORIES, signatory_quorum()) {
        println!(
            "qclient-console {} is already installed; nothing to update",
            assets.version
        );
        return Ok(());
    }

    let binary = response_bytes(&client, &assets.binary_url, "Console release binary").await?;
    let (digest, signatures) = fetch_signatures(&client, &assets).await?;
    let bundle = Release {
        base: assets.base,
        bin: binary,
        digest,
        sigs: signatures,
    };
    let (binary_path, valid) =
        bundle.install_with(&destination, &SIGNATORIES, signatory_quorum())?;
    release::switch_symlink(&binary_path, &link)?;
    println!(
        "Updated qclient-console to {} at {} ({valid} valid signatures)",
        assets.version,
        binary_path.display()
    );
    Ok(())
}

pub fn link() -> anyhow::Result<()> {
    let executable =
        std::env::current_exe().context("determine the qclient-console executable path")?;
    let executable = std::fs::canonicalize(&executable).unwrap_or(executable);
    let link = default_link();
    release::switch_symlink(&executable, &link)?;
    println!(
        "Symlink created at {} → {}",
        link.display(),
        executable.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(tag: &str, names: &[String]) -> GithubRelease {
        GithubRelease {
            tag_name: tag.to_owned(),
            assets: names
                .iter()
                .map(|name| GithubAsset {
                    name: name.clone(),
                    browser_download_url: format!(
                        "https://github.com/{RELEASE_REPOSITORY}/releases/download/v2.1.0.25/{name}"
                    ),
                })
                .collect(),
        }
    }

    fn complete_names(base: &str) -> Vec<String> {
        let mut names = vec![base.to_owned(), format!("{base}.dgst")];
        names.extend((1..=signatory_quorum()).map(|index| format!("{base}.dgst.sig.{index}")));
        names
    }

    #[test]
    fn exact_console_assets_are_selected_and_qclient_assets_are_ignored() {
        let base = "qclient-console-2.1.0.25-linux-amd64";
        let mut names = complete_names(base);
        names.extend([
            "qclient-2.1.0.25-linux-amd64".to_owned(),
            "qclient-2.1.0.25-linux-amd64.dgst".to_owned(),
            format!("{base}.backup"),
            format!("{base}.dgst.sig.999"),
        ]);
        let selected =
            select_assets(metadata("v2.1.0.25", &names), None, "linux", "amd64").unwrap();
        assert_eq!(selected.base, base);
        assert_eq!(selected.signature_urls.len(), signatory_quorum());
        assert!(selected
            .signature_urls
            .iter()
            .all(|(_, url)| url.contains(base) && !url.contains("qclient-2")));
    }

    #[test]
    fn incomplete_or_wrong_release_assets_fail_closed() {
        let base = "qclient-console-2.1.0.25-linux-amd64";
        for names in [
            vec!["qclient-2.1.0.25-linux-amd64".to_owned()],
            vec![base.to_owned()],
            vec![base.to_owned(), format!("{base}.dgst")],
        ] {
            let error = select_assets(
                metadata("2.1.0.25", &names),
                Some("2.1.0.25"),
                "linux",
                "amd64",
            )
            .unwrap_err();
            assert!(error.to_string().contains("no verifiable Console release"));
        }
    }

    #[test]
    fn returned_tag_must_match_the_requested_version() {
        let base = "qclient-console-2.1.0.26-linux-amd64";
        let error = select_assets(
            metadata("v2.1.0.26", &complete_names(base)),
            Some("2.1.0.25"),
            "linux",
            "amd64",
        )
        .unwrap_err();
        assert!(error.to_string().contains("requested 2.1.0.25"));
    }

    #[test]
    fn metadata_urls_reject_path_like_versions() {
        for version in ["../../latest", "v2.1", "2.1.0-beta"] {
            assert!(release_metadata_url(Some(version)).is_err(), "{version}");
        }
    }
}

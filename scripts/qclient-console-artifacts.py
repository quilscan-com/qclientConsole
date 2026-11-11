#!/usr/bin/env python3
"""Independent Ed25519 release artifacts; unrelated to native Ed448 verification."""
import argparse
import base64
import binascii
from datetime import datetime, timezone
import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile


REPOSITORY = "quilscan-com/qclientConsole"
CHANNEL = "qclient-console"
PLATFORMS = ("linux-amd64", "darwin-arm64")
MANIFEST = "qclient-console-release"
METADATA = "qclient-console-version.json"
PUBLIC_KEY = Path(__file__).with_name("qclient-console-signing-public-key.b64")
PRIVATE_PREFIX = bytes.fromhex("302e020100300506032b657004220420")
PUBLIC_PREFIX = bytes.fromhex("302a300506032b6570032100")
SECRET_NAMES = ("DEV_NODE_SIGNING_PRIVATE_KEY", "AGENT_SIGNING_PRIVATE_KEY")


class ArtifactError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise ArtifactError(message)


def openssl(*args):
    # Do not pass the release secret to subprocesses or include OpenSSL's input
    # diagnostics in errors. All key bytes stay in private temporary files.
    env = {k: v for k, v in os.environ.items() if k not in SECRET_NAMES}
    result = subprocess.run([os.environ.get("OPENSSL_BIN", "openssl"), *map(str, args)], env=env, capture_output=True)
    require(result.returncode == 0, "OpenSSL operation failed")
    return result.stdout


def decode64(value, length, label):
    try:
        decoded = base64.b64decode(value.strip(), validate=True)
    except (ValueError, binascii.Error):
        raise ArtifactError(f"invalid Base64 {label}") from None
    require(len(decoded) == length, f"{label} must decode to {length} bytes")
    return decoded


def regular_file(path):
    try:
        info = path.lstat()
    except FileNotFoundError:
        raise ArtifactError(f"missing asset: {path.name}") from None
    require(stat.S_ISREG(info.st_mode), f"asset must be a regular file: {path.name}")
    return path


def digest(path):
    hasher = hashlib.sha3_256()
    with regular_file(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def digest_text(name, checksum):
    # Use only the release basename, independent of the builder's directory.
    return f"SHA3-256({name})= {checksum}\n"


def files_for(version):
    return [{"platform": platform, "binary": f"qclient-console-{version}-{platform}",
             "digest": f"qclient-console-{version}-{platform}.dgst",
             "signature": f"qclient-console-{version}-{platform}.sig",
             "signature_type": "ed25519-binary"} for platform in PLATFORMS]


def manifest_text(files):
    names = [entry[field] for entry in files for field in ("binary", "digest", "signature")]
    return "\n".join(sorted(names + [METADATA, METADATA + ".sig"])) + "\n"


def check_assets(dist, files, signing=False):
    expected = {entry[field] for entry in files for field in ("binary", "digest", "signature")}
    expected.update((MANIFEST, METADATA, METADATA + ".sig"))
    for asset in dist.iterdir():
        if asset.name.startswith("qclient-console-"):
            require(asset.name in expected, f"unexpected release asset: {asset.name}")
    for entry in files:
        regular_file(dist / entry["binary"])
    for name in expected:
        path = dist / name
        if not signing or path.exists() or path.is_symlink():
            regular_file(path)


def validate_provenance(commit, run_id):
    require(isinstance(commit, str) and (commit == "" or re.fullmatch(r"[0-9a-fA-F]{40}|[0-9a-fA-F]{64}", commit)),
            "commit must be a full Git commit hash or empty outside CI")
    require(isinstance(run_id, str) and (run_id == "" or re.fullmatch(r"[0-9]+", run_id)),
            "run_id must be numeric or empty outside CI")


def sign_file(path, key, temporary):
    signature = temporary / "signature.raw"
    openssl("pkeyutl", "-sign", "-rawin", "-inkey", key, "-keyform", "DER",
            "-in", path, "-out", signature)
    return base64.b64encode(signature.read_bytes()).decode("ascii") + "\n"


def verify_file(path, signature_path, key, temporary):
    regular_file(path)
    encoded = regular_file(signature_path).read_text(encoding="ascii")
    signature = temporary / "signature.raw"
    signature.write_bytes(decode64(encoded, 64, "signature"))
    openssl("pkeyutl", "-verify", "-rawin", "-pubin", "-inkey", key, "-keyform", "DER",
            "-in", path, "-sigfile", signature)


def sign(version, dist, trusted_public, temporary):
    secret = os.environ.get(SECRET_NAMES[0]) or os.environ.get(SECRET_NAMES[1]) or ""
    require(secret, "DEV_NODE_SIGNING_PRIVATE_KEY is required (AGENT_SIGNING_PRIVATE_KEY fallback supported)")
    raw = decode64(secret, 64, "Ed25519 private key")
    private = temporary / "private.der"
    # mkdtemp is mode 0700; make the key itself mode 0600 as well.
    with private.open("xb") as output:
        os.chmod(private, 0o600)
        output.write(PRIVATE_PREFIX + raw[:32])
    public_der = openssl("pkey", "-inform", "DER", "-in", private, "-pubout", "-outform", "DER")
    require(public_der.startswith(PUBLIC_PREFIX) and len(public_der) == 44, "unexpected Ed25519 public key")
    derived_public = public_der[len(PUBLIC_PREFIX):]
    require(hmac.compare_digest(derived_public, raw[32:]), "private key public trailer does not match its seed")
    require(hmac.compare_digest(derived_public, trusted_public), "signing key does not match the pinned public key")

    repository = os.environ.get("GITHUB_REPOSITORY", REPOSITORY)
    require(repository == REPOSITORY, f"GITHUB_REPOSITORY must be {REPOSITORY}")
    commit, run_id = os.environ.get("GITHUB_SHA", ""), os.environ.get("GITHUB_RUN_ID", "")
    validate_provenance(commit, run_id)
    if os.environ.get("GITHUB_ACTIONS") == "true":
        require(commit and run_id, "GitHub Actions requires GITHUB_SHA and GITHUB_RUN_ID")
    files = files_for(version)
    check_assets(dist, files, signing=True)
    metadata = {"schema": 1, "channel": CHANNEL, "version": version,
                "repository": repository, "commit": commit, "run_id": run_id,
                "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                "base_url": os.environ.get("QCLIENT_CONSOLE_RELEASE_BASE_URL", ""),
                "manifest": MANIFEST, "files": files}
    # Only release payloads enter this staging directory. Private keys and
    # OpenSSL scratch files stay in the separate system temporary directory.
    with tempfile.TemporaryDirectory(prefix=".qclient-console-artifacts-", dir=dist) as name:
        staging = Path(name)
        for entry in files:
            binary = dist / entry["binary"]
            entry["sha3_256"] = digest(binary)
            (staging / entry["digest"]).write_text(digest_text(entry["binary"], entry["sha3_256"]), encoding="ascii")
            (staging / entry["signature"]).write_text(sign_file(binary, private, temporary), encoding="ascii")
        (staging / METADATA).write_text(json.dumps(metadata, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")
        (staging / (METADATA + ".sig")).write_text(sign_file(staging / METADATA, private, temporary), encoding="ascii")
        (staging / MANIFEST).write_text(manifest_text(files), encoding="ascii")
        # Stage every generated asset first. Publish the manifest last so a
        # failed signing operation cannot create a complete new bundle.
        names = manifest_text(files).splitlines() + [MANIFEST]
        binary_names = {entry["binary"] for entry in files}
        for name in names:
            if name not in binary_names:
                os.replace(staging / name, dist / name)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON field")
        result[key] = value
    return result


def verify(version, dist, public, temporary):
    key = temporary / "public.der"
    key.write_bytes(PUBLIC_PREFIX + public)
    files = files_for(version)
    check_assets(dist, files)
    # Authenticate the exact bytes before interpreting any metadata claims.
    verify_file(dist / METADATA, dist / (METADATA + ".sig"), key, temporary)
    try:
        metadata = json.loads((dist / METADATA).read_text(encoding="utf-8"), object_pairs_hook=unique_object)
    except (ValueError, UnicodeError):
        raise ArtifactError("invalid metadata JSON") from None
    require(isinstance(metadata, dict), "metadata must be a JSON object")
    for field, expected in (("schema", 1), ("channel", CHANNEL), ("version", version),
                            ("repository", REPOSITORY), ("manifest", MANIFEST)):
        require(metadata.get(field) == expected, f"metadata {field} mismatch")
    require(type(metadata["schema"]) is int, "schema must be an integer")
    validate_provenance(metadata.get("commit"), metadata.get("run_id"))
    require(isinstance(metadata.get("generated_at"), str) and
            re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", metadata["generated_at"]),
            "invalid generated_at")
    require(isinstance(metadata.get("base_url"), str), "base_url must be a string")
    actual_files = metadata.get("files")
    require(isinstance(actual_files, list) and len(actual_files) == len(files), "metadata must contain exactly two platforms")
    for actual, expected in zip(actual_files, files):
        require(isinstance(actual, dict), "file metadata must be an object")
        require(set(actual) == set(expected) | {"sha3_256"}, "unexpected file metadata fields")
        for field, value in expected.items():
            require(actual.get(field) == value, f"metadata file {field} mismatch")
        checksum = digest(dist / expected["binary"])
        require(actual.get("sha3_256") == checksum, "binary SHA3-256 mismatch")
        require((dist / expected["digest"]).read_text(encoding="ascii") == digest_text(expected["binary"], checksum),
                "digest content or basename mismatch")
        verify_file(dist / expected["binary"], dist / expected["signature"], key, temporary)
    require((dist / MANIFEST).read_text(encoding="ascii") == manifest_text(files), "release manifest mismatch")


def main():
    parser = argparse.ArgumentParser(description=__doc__, epilog=(
        "Usage: {sign,verify}-qclient-console-artifacts.sh <version> <dist-dir> [--public-key FILE]. "
        "Requires Python 3 and OpenSSL 3 (select its executable with OPENSSL_BIN if needed). "
        "Both linux-amd64 and darwin-arm64 binaries are required. "
        "Signing uses DEV_NODE_SIGNING_PRIVATE_KEY (Base64 Go Ed25519 seed+public, 64 bytes), "
        "with AGENT_SIGNING_PRIVATE_KEY fallback. GITHUB_REPOSITORY defaults to " + REPOSITORY +
        "; GITHUB_SHA and GITHUB_RUN_ID supply signed provenance and are required in GitHub Actions. "
        "Optional QCLIENT_CONSOLE_RELEASE_BASE_URL is signed as metadata. --public-key explicitly "
        "replaces the pinned Base64 public key for testing or deliberate offline trust selection; "
        "no environment variable can replace trust. This tool does not configure Console runtime verification."))
    parser.add_argument("action", choices=("sign", "verify"))
    parser.add_argument("version")
    parser.add_argument("dist_dir", type=Path)
    parser.add_argument("--public-key", type=Path, default=PUBLIC_KEY, metavar="FILE")
    args = parser.parse_args()
    try:
        require(re.fullmatch(r"[0-9]+(?:\.[0-9]+){2,4}", args.version), "version must have 3 to 5 numeric dotted parts")
        require(args.dist_dir.is_dir() and not args.dist_dir.is_symlink(), "dist-dir must be an existing directory, not a symlink")
        require(openssl("version").startswith(b"OpenSSL 3."), "OpenSSL 3 is required")
        public = decode64(regular_file(args.public_key).read_text(encoding="ascii"), 32, "public key")
        # Verification never writes into dist. Keep all key material and
        # OpenSSL scratch outside the artifact tree for both operations.
        with tempfile.TemporaryDirectory(prefix="qclient-console-crypto-") as name:
            if args.action == "sign":
                sign(args.version, args.dist_dir, public, Path(name))
            else:
                verify(args.version, args.dist_dir, public, Path(name))
        print(f"{args.action} completed for qclient-console {args.version} (2 platforms)")
        return 0
    except (ArtifactError, OSError, UnicodeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

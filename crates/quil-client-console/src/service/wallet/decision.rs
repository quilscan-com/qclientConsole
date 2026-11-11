#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum OperationDecision {
    Committed { frame: u64 },
    Rejected { frame: u64, reason: String },
}

pub(super) fn parse_record(blob: &[u8]) -> anyhow::Result<OperationDecision> {
    let tree = quil_tries::VectorCommitmentTree {
        root: quil_tries::deserialize_go_tree(blob)
            .map_err(|_| anyhow::anyhow!("malformed decision record"))?,
    };
    let expected_kind =
        quil_crypto::poseidon::hash_bytes_to_32(b"quil/commit/decision-record/v1\0")
            .map_err(|error| anyhow::anyhow!("decision record kind: {error}"))?;
    anyhow::ensure!(
        tree.get(&[0xff; 32]) == Some(expected_kind.as_slice()),
        "not a decision record"
    );
    let field = |key: u8| {
        tree.get(&[key])
            .ok_or_else(|| anyhow::anyhow!("decision record missing a field"))
    };
    let frame = u64::from_be_bytes(
        field(4)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("malformed decision frame"))?,
    );
    match field(0)? {
        [1] if field(8)?.is_empty() => Ok(OperationDecision::Committed { frame }),
        [0] => Ok(OperationDecision::Rejected {
            frame,
            reason: std::str::from_utf8(field(8)?)
                .map_err(|_| anyhow::anyhow!("decision rejection reason is not UTF-8"))?
                .to_owned(),
        }),
        _ => anyhow::bail!("malformed decision record"),
    }
}

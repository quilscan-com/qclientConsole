use std::collections::BTreeSet;

use quil_lattice_ct::confidential::{relation::membership::IDENTITY_BYTES, transfer::Output};
use zeroize::Zeroizing;

use super::ServiceError;

#[derive(Debug)]
pub(super) struct OwnedCoin {
    pub(super) address: [u8; 32],
    #[allow(dead_code)]
    pub(super) output: Output,
    pub(super) amount: Zeroizing<u128>,
    pub(super) image: [u8; IDENTITY_BYTES],
}

impl OwnedCoin {
    pub(super) fn amount(&self) -> u128 {
        *self.amount
    }
}

pub(super) fn select_coins(
    mut coins: Vec<OwnedCoin>,
    amount: u128,
    fee: u128,
    max_inputs: usize,
) -> Result<(Vec<OwnedCoin>, Zeroizing<u128>), ServiceError> {
    if amount == 0 || max_inputs == 0 {
        return Err(ServiceError::Wallet(
            "Amount and input limit must be positive".into(),
        ));
    }
    let needed = amount
        .checked_add(fee)
        .ok_or_else(|| ServiceError::Wallet("Amount and fee overflow".into()))?;
    coins.sort_by(|left, right| {
        right
            .amount()
            .cmp(&left.amount())
            .then(left.address.cmp(&right.address))
    });

    let mut selected = Vec::new();
    let mut addresses = BTreeSet::new();
    let mut images = BTreeSet::new();
    let mut total = Zeroizing::new(0u128);
    for coin in coins {
        if coin.amount() == 0 {
            continue;
        }
        if selected.len() >= max_inputs {
            return Err(ServiceError::Wallet(
                "Cannot fund transfer within the input limit".into(),
            ));
        }
        if !addresses.insert(coin.address) || !images.insert(coin.image) {
            return Err(ServiceError::Wallet(
                "Selected Coins contain a duplicate address or spend image".into(),
            ));
        }
        *total = total
            .checked_add(coin.amount())
            .ok_or_else(|| ServiceError::Wallet("Selected balance overflow".into()))?;
        selected.push(coin);
        if *total >= needed {
            return Ok((selected, Zeroizing::new(*total - needed)));
        }
    }

    Err(ServiceError::Wallet(
        "Insufficient node-reported unspent funds".into(),
    ))
}

#[cfg(test)]
mod tests {
    use quil_lattice_ct::confidential::{
        transfer::{Output, MEMO_BYTES},
        AmountCommitment, COMMITMENT_BYTES,
    };
    use zeroize::Zeroizing;

    use super::*;

    fn owned(amount: u128, marker: u8) -> OwnedCoin {
        OwnedCoin {
            address: [marker; 32],
            output: Output {
                commitment: AmountCommitment::from_bytes(&[0; COMMITMENT_BYTES]).unwrap(),
                owner: [marker;
                    quil_lattice_ct::confidential::relation::membership::IDENTITY_BYTES],
                memo: [marker; MEMO_BYTES],
            },
            amount: Zeroizing::new(amount),
            image: [marker; quil_lattice_ct::confidential::relation::membership::IDENTITY_BYTES],
        }
    }

    #[test]
    fn selection_uses_largest_coins_and_returns_change() {
        let coins = vec![owned(3, 1), owned(9, 2), owned(5, 3)];
        let (selected, change) = select_coins(coins, 10, 1, 4).unwrap();

        assert_eq!(selected.iter().map(OwnedCoin::amount).sum::<u128>(), 14);
        assert_eq!(*change, 3);
    }

    #[test]
    fn selection_refuses_to_exceed_the_circuit_input_limit() {
        let coins = vec![
            owned(2, 1),
            owned(2, 2),
            owned(2, 3),
            owned(2, 4),
            owned(2, 5),
        ];

        let error = select_coins(coins, 9, 0, 4).unwrap_err();
        assert!(error.to_string().contains("input limit"), "{error}");
    }

    #[test]
    fn selection_rejects_duplicate_coin_identity() {
        let error = select_coins(vec![owned(5, 1), owned(5, 1)], 9, 0, 4).unwrap_err();
        assert!(error.to_string().contains("duplicate"), "{error}");
    }
}

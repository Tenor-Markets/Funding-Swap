#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError, UpdateWithdrawalLimitsArgs};
use solana_sdk::signer::Signer;

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_asset_withdrawal_limits,
    assert_instruction_error, assert_transaction_success, create_asset, create_asset_args, new_svm,
    read_asset_state, send_tx, send_without_signer, setup_active_asset, setup_governance,
    tighten_withdrawal_limits_instruction,
};

#[test]
fn test_tighten_withdrawal_limits_updates_limits_and_rejects_wrong_direction() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let (asset, _) = setup_active_asset(&mut svm, &setup).expect("active asset");
    let before = read_asset_state(&svm, asset).expect("read asset state");

    let authority = setup.authority(GovernanceCategory::Emergency);

    // Tightening only allows limits to decrease, so each case doubles the
    // live limit.
    let wrong_direction_cases = [
        UpdateWithdrawalLimitsArgs {
            short_window_limit_atoms: Some(before.short_window.limit_atoms * 2),
            ..UpdateWithdrawalLimitsArgs::default()
        },
        UpdateWithdrawalLimitsArgs {
            medium_window_limit_atoms: Some(before.medium_window.limit_atoms * 2),
            ..UpdateWithdrawalLimitsArgs::default()
        },
        UpdateWithdrawalLimitsArgs {
            long_window_limit_atoms: Some(before.long_window.limit_atoms * 2),
            ..UpdateWithdrawalLimitsArgs::default()
        },
    ];

    for args in wrong_direction_cases {
        let ix = tighten_withdrawal_limits_instruction(authority.pubkey(), asset, args);

        assert_instruction_error(
            send_tx(&mut svm, authority, vec![ix], &[]),
            0,
            u32::from(StateError::RiskUpdateWrongDirection),
        );
    }

    let short_limit_atoms = before.short_window.limit_atoms / 2;
    let medium_limit_atoms = before.medium_window.limit_atoms / 2;
    let long_limit_atoms = before.long_window.limit_atoms / 2;

    let args = UpdateWithdrawalLimitsArgs {
        short_window_limit_atoms: Some(short_limit_atoms),
        medium_window_limit_atoms: Some(medium_limit_atoms),
        long_window_limit_atoms: Some(long_limit_atoms),
    };

    let ix = tighten_withdrawal_limits_instruction(authority.pubkey(), asset, args);

    assert_transaction_success(send_tx(&mut svm, authority, vec![ix], &[]));

    assert_asset_withdrawal_limits(
        &svm,
        asset,
        short_limit_atoms,
        medium_limit_atoms,
        long_limit_atoms,
    );
}

#[test]
fn test_tighten_withdrawal_limits_fails_while_pending() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let asset = create_asset(&mut svm, &setup, create_asset_args()).expect("create asset");
    let before = read_asset_state(&svm, asset).expect("read asset state");

    let authority = setup.authority(GovernanceCategory::Emergency);
    let args = UpdateWithdrawalLimitsArgs {
        short_window_limit_atoms: Some(before.short_window.limit_atoms / 2),
        ..UpdateWithdrawalLimitsArgs::default()
    };

    let ix = tighten_withdrawal_limits_instruction(authority.pubkey(), asset, args);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_tighten_withdrawal_limits_fails_when_signer_is_not_emergency_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let (asset, _) = setup_active_asset(&mut svm, &setup).expect("active asset");
    let before = read_asset_state(&svm, asset).expect("read asset state");

    let attacker = airdrop_keypair(&mut svm);
    let args = UpdateWithdrawalLimitsArgs {
        short_window_limit_atoms: Some(before.short_window.limit_atoms / 2),
        ..UpdateWithdrawalLimitsArgs::default()
    };

    let ix = tighten_withdrawal_limits_instruction(attacker.pubkey(), asset, args);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_tighten_withdrawal_limits_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let (asset, _) = setup_active_asset(&mut svm, &setup).expect("active asset");
    let before = read_asset_state(&svm, asset).expect("read asset state");

    let authority = setup.authority(GovernanceCategory::Emergency);
    let payer = airdrop_keypair(&mut svm);
    let args = UpdateWithdrawalLimitsArgs {
        short_window_limit_atoms: Some(before.short_window.limit_atoms / 2),
        ..UpdateWithdrawalLimitsArgs::default()
    };

    let ix = tighten_withdrawal_limits_instruction(authority.pubkey(), asset, args);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}

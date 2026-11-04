#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError};
use solana_sdk::signer::Signer;

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_instruction_error,
    assert_margin_group_health_ratios, assert_transaction_success, create_margin_group, fixed_raw,
    group_risk_args, new_svm, relax_group_risk_instruction, send_tx, send_without_signer,
    setup_active_margin_group, setup_governance,
};

#[test]
fn test_relax_group_risk_updates_both_fields_and_rejects_wrong_direction() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 1, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let authority = setup.authority(GovernanceCategory::RiskRecovery);

    let relaxed_critical = fixed_raw(3, 4);
    let relaxed_risky = fixed_raw(5, 4);

    let args = group_risk_args(Some(relaxed_critical), Some(relaxed_risky));
    let ix = relax_group_risk_instruction(authority.pubkey(), margin_group, args);

    assert_transaction_success(send_tx(&mut svm, authority, vec![ix], &[]));

    assert_margin_group_health_ratios(&svm, margin_group, relaxed_critical, relaxed_risky);

    let higher_critical = group_risk_args(Some(fixed_raw(1, 1)), None);
    let ix = relax_group_risk_instruction(authority.pubkey(), margin_group, higher_critical);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::RiskUpdateWrongDirection),
    );

    let higher_risky = group_risk_args(None, Some(fixed_raw(3, 2)));
    let ix = relax_group_risk_instruction(authority.pubkey(), margin_group, higher_risky);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::RiskUpdateWrongDirection),
    );
}

#[test]
fn test_relax_group_risk_fails_while_pending() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group = create_margin_group(&mut svm, &setup, 2, fixed_raw(1, 1), fixed_raw(3, 2))
        .expect("create margin group");

    let authority = setup.authority(GovernanceCategory::RiskRecovery);
    let args = group_risk_args(Some(fixed_raw(3, 4)), None);

    let ix = relax_group_risk_instruction(authority.pubkey(), margin_group, args);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_relax_group_risk_fails_when_signer_is_not_recovery_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 3, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let attacker = airdrop_keypair(&mut svm);
    let args = group_risk_args(Some(fixed_raw(3, 4)), None);

    let ix = relax_group_risk_instruction(attacker.pubkey(), margin_group, args);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_relax_group_risk_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 4, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let authority = setup.authority(GovernanceCategory::RiskRecovery);
    let payer = airdrop_keypair(&mut svm);
    let args = group_risk_args(Some(fixed_raw(3, 4)), None);

    let ix = relax_group_risk_instruction(authority.pubkey(), margin_group, args);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}

#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError};
use solana_sdk::signer::Signer;

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_instruction_error,
    assert_margin_group_created, create_margin_group, create_margin_group_args,
    create_margin_group_instruction, create_margin_group_result, fixed_raw, new_svm, send_tx,
    send_without_signer, setup_governance,
};

#[test]
fn test_create_margin_group_validates_health_ratios_and_creates_pending_group() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let zero_critical = create_margin_group_args(1, 0, fixed_raw(1, 1));

    assert_instruction_error(
        create_margin_group_result(&mut svm, &setup, zero_critical),
        0,
        u32::from(StateError::InvalidGroupHealthRatios),
    );

    let risky_below_critical = create_margin_group_args(2, fixed_raw(2, 1), fixed_raw(1, 1));

    assert_instruction_error(
        create_margin_group_result(&mut svm, &setup, risky_below_critical),
        0,
        u32::from(StateError::InvalidGroupHealthRatios),
    );

    let group_id = 7;
    let critical_health_ratio = fixed_raw(1, 1);
    let risky_order_cancellation_ratio = fixed_raw(3, 2);

    let margin_group = create_margin_group(
        &mut svm,
        &setup,
        group_id,
        critical_health_ratio,
        risky_order_cancellation_ratio,
    )
    .expect("create margin group");

    assert_margin_group_created(
        &svm,
        margin_group,
        group_id,
        critical_health_ratio,
        risky_order_cancellation_ratio,
    );
}

#[test]
fn test_create_margin_group_fails_when_signer_is_not_resources_authority() {
    let mut svm = new_svm().expect("svm");

    setup_governance(&mut svm).expect("governance setup");

    let attacker = airdrop_keypair(&mut svm);
    let args = create_margin_group_args(8, fixed_raw(1, 1), fixed_raw(3, 2));

    let ix = create_margin_group_instruction(attacker.pubkey(), args);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_create_margin_group_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let authority = setup.authority(GovernanceCategory::Resources);
    let payer = airdrop_keypair(&mut svm);

    let args = create_margin_group_args(9, fixed_raw(1, 1), fixed_raw(3, 2));

    let ix = create_margin_group_instruction(authority.pubkey(), args);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}

#![cfg(feature = "test-helpers")]

use funding_swap::state::StateError;
use solana_sdk::signer::Signer;

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_instruction_error,
    assert_margin_group_counts, assert_subaccount_created, create_margin_group, create_subaccount,
    create_subaccount_instruction, fixed_raw, new_svm, read_governance, send_tx,
    send_without_signer, set_subaccount_creation_fee, setup_active_margin_group, setup_governance,
};

#[test]
fn test_create_subaccount_succeeds_and_collects_creation_fee() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let fee_destination = airdrop_keypair(&mut svm);
    let creation_fee_lamports = 1_000_000;

    set_subaccount_creation_fee(
        &mut svm,
        &setup,
        creation_fee_lamports,
        fee_destination.pubkey(),
    )
    .expect("set creation fee");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 1, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let authority = airdrop_keypair(&mut svm);

    let destination_before = svm
        .get_account(&fee_destination.pubkey())
        .expect("fee destination")
        .lamports;

    let subaccount = create_subaccount(&mut svm, &setup, &authority, 1, margin_group)
        .expect("create subaccount");

    assert_subaccount_created(&svm, subaccount, authority.pubkey(), 1, 1);
    assert_margin_group_counts(&svm, margin_group, 0, 1);

    let destination_after = svm
        .get_account(&fee_destination.pubkey())
        .expect("fee destination")
        .lamports;

    assert_eq!(
        destination_after,
        destination_before + creation_fee_lamports
    );
}

#[test]
fn test_create_subaccount_fails_when_margin_group_is_not_active() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group = create_margin_group(&mut svm, &setup, 2, fixed_raw(1, 1), fixed_raw(3, 2))
        .expect("create margin group");

    let governance = read_governance(&svm, setup.governance).expect("read governance");

    let authority = airdrop_keypair(&mut svm);

    let ix = create_subaccount_instruction(
        authority.pubkey(),
        2,
        margin_group,
        governance.subaccount_creation_fee_destination,
    );

    assert_instruction_error(
        send_tx(&mut svm, &authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_create_subaccount_fails_with_wrong_creation_fee_destination() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 3, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");
    let authority = airdrop_keypair(&mut svm);
    let wrong_destination = airdrop_keypair(&mut svm);

    let ix = create_subaccount_instruction(
        authority.pubkey(),
        3,
        margin_group,
        wrong_destination.pubkey(),
    );

    assert_instruction_error(
        send_tx(&mut svm, &authority, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidCreationFeeDestination),
    );
}

#[test]
fn test_create_subaccount_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 4, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");
    let authority = airdrop_keypair(&mut svm);
    let payer = airdrop_keypair(&mut svm);
    let governance = read_governance(&svm, setup.governance).expect("read governance");

    let ix = create_subaccount_instruction(
        authority.pubkey(),
        4,
        margin_group,
        governance.subaccount_creation_fee_destination,
    );

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}

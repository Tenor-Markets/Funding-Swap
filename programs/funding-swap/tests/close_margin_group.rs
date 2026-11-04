#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError};
use solana_sdk::{signature::Keypair, signer::Signer};

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_instruction_error,
    assert_margin_group_closed, assert_transaction_success, close_margin_group_instruction,
    fixed_raw, new_svm, send_tx, send_without_signer, set_margin_group_counts,
    setup_active_margin_group, setup_governance,
};

#[test]
fn test_close_margin_group_succeeds_and_rejects_live_counts() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 1, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let authority = setup.authority(GovernanceCategory::Resources);
    let rent_recipient = Keypair::new().pubkey();

    set_margin_group_counts(&mut svm, margin_group, 1, 0).expect("set market count");

    let ix = close_margin_group_instruction(authority.pubkey(), margin_group, rent_recipient);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::MarketsNotEmpty),
    );

    set_margin_group_counts(&mut svm, margin_group, 0, 1).expect("set subaccount count");

    svm.expire_blockhash();

    let ix = close_margin_group_instruction(authority.pubkey(), margin_group, rent_recipient);

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![ix], &[]),
        0,
        u32::from(StateError::SubaccountsNotEmpty),
    );

    set_margin_group_counts(&mut svm, margin_group, 0, 0).expect("clear counts");

    svm.expire_blockhash();

    let ix = close_margin_group_instruction(authority.pubkey(), margin_group, rent_recipient);

    assert_transaction_success(send_tx(&mut svm, authority, vec![ix], &[]));

    assert_margin_group_closed(&svm, margin_group);
}

#[test]
fn test_close_margin_group_fails_when_signer_is_not_resources_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 2, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let attacker = airdrop_keypair(&mut svm);
    let rent_recipient = Keypair::new().pubkey();

    let ix = close_margin_group_instruction(attacker.pubkey(), margin_group, rent_recipient);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_close_margin_group_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 3, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let authority = setup.authority(GovernanceCategory::Resources);
    let payer = airdrop_keypair(&mut svm);
    let rent_recipient = Keypair::new().pubkey();

    let ix = close_margin_group_instruction(authority.pubkey(), margin_group, rent_recipient);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}

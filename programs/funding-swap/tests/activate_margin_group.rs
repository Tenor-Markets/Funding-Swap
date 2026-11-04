#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError, GROUP_ACTIVE, GROUP_PENDING};
use solana_sdk::signer::Signer;

mod common;
use common::{
    activate_margin_group, activate_margin_group_instruction, airdrop_keypair,
    assert_account_not_signer, assert_instruction_error, assert_margin_group_status,
    create_margin_group, fixed_raw, new_svm, send_tx, send_without_signer, setup_governance,
};

#[test]
fn test_activate_margin_group_succeeds_and_rejects_repeat() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group = create_margin_group(&mut svm, &setup, 1, fixed_raw(1, 1), fixed_raw(3, 2))
        .expect("create margin group");

    assert_margin_group_status(&svm, margin_group, GROUP_PENDING);

    activate_margin_group(&mut svm, &setup, margin_group).expect("activate margin group");

    assert_margin_group_status(&svm, margin_group, GROUP_ACTIVE);

    let authority = setup.authority(GovernanceCategory::Resources);
    let repeat = activate_margin_group_instruction(authority.pubkey(), margin_group);

    svm.expire_blockhash();

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![repeat], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_activate_margin_group_fails_when_signer_is_not_resources_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group = create_margin_group(&mut svm, &setup, 2, fixed_raw(1, 1), fixed_raw(3, 2))
        .expect("create margin group");

    let attacker = airdrop_keypair(&mut svm);
    let ix = activate_margin_group_instruction(attacker.pubkey(), margin_group);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_activate_margin_group_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group = create_margin_group(&mut svm, &setup, 3, fixed_raw(1, 1), fixed_raw(3, 2))
        .expect("create margin group");

    let authority = setup.authority(GovernanceCategory::Resources);
    let payer = airdrop_keypair(&mut svm);

    let ix = activate_margin_group_instruction(authority.pubkey(), margin_group);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}

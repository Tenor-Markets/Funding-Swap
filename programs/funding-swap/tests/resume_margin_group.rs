#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError, GROUP_ACTIVE, GROUP_PAUSED};
use solana_sdk::signer::Signer;

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_instruction_error,
    assert_margin_group_status, fixed_raw, new_svm, pause_margin_group, resume_margin_group,
    resume_margin_group_instruction, send_tx, send_without_signer, setup_active_margin_group,
    setup_governance,
};

#[test]
fn test_resume_margin_group_succeeds_and_rejects_active_group() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 1, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    pause_margin_group(&mut svm, &setup, margin_group).expect("pause margin group");

    assert_margin_group_status(&svm, margin_group, GROUP_PAUSED);

    resume_margin_group(&mut svm, &setup, margin_group).expect("resume margin group");

    assert_margin_group_status(&svm, margin_group, GROUP_ACTIVE);

    let authority = setup.authority(GovernanceCategory::RiskRecovery);
    let repeat = resume_margin_group_instruction(authority.pubkey(), margin_group);

    svm.expire_blockhash();

    assert_instruction_error(
        send_tx(&mut svm, authority, vec![repeat], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}

#[test]
fn test_resume_margin_group_fails_when_signer_is_not_recovery_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 2, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    pause_margin_group(&mut svm, &setup, margin_group).expect("pause margin group");

    let attacker = airdrop_keypair(&mut svm);
    let ix = resume_margin_group_instruction(attacker.pubkey(), margin_group);

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_resume_margin_group_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 3, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    pause_margin_group(&mut svm, &setup, margin_group).expect("pause margin group");

    let authority = setup.authority(GovernanceCategory::RiskRecovery);
    let payer = airdrop_keypair(&mut svm);

    let ix = resume_margin_group_instruction(authority.pubkey(), margin_group);

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}

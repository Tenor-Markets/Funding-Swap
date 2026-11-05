#![cfg(feature = "test-helpers")]

use funding_swap::state::StateError;
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer};

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_instruction_error,
    assert_subaccount_delegate, assert_transaction_success, create_subaccount, fixed_raw, new_svm,
    send_tx, send_without_signer, setup_active_margin_group, setup_governance,
    update_delegate_instruction,
};

fn setup_subaccount(
    svm: &mut litesvm::LiteSVM,
    setup: &common::GovernanceSetup,
) -> (solana_sdk::signature::Keypair, Pubkey, Pubkey) {
    let margin_group = setup_active_margin_group(svm, setup, 1, fixed_raw(1, 1), fixed_raw(3, 2))
        .expect("active margin group");

    let owner = airdrop_keypair(svm);

    let subaccount =
        create_subaccount(svm, setup, &owner, 1, margin_group).expect("create subaccount");

    (owner, subaccount, margin_group)
}

#[test]
fn test_update_delegate_appoints_and_revokes() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let (owner, subaccount, margin_group) = setup_subaccount(&mut svm, &setup);
    let delegate = Keypair::new().pubkey();

    let ix = update_delegate_instruction(owner.pubkey(), subaccount, delegate, margin_group);

    assert_transaction_success(send_tx(&mut svm, &owner, vec![ix], &[]));
    assert_subaccount_delegate(&svm, subaccount, delegate);

    let revoke =
        update_delegate_instruction(owner.pubkey(), subaccount, Pubkey::default(), margin_group);

    assert_transaction_success(send_tx(&mut svm, &owner, vec![revoke], &[]));
    assert_subaccount_delegate(&svm, subaccount, Pubkey::default());
}

#[test]
fn test_update_delegate_fails_when_signer_is_not_the_owner() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let (owner, subaccount, margin_group) = setup_subaccount(&mut svm, &setup);
    let delegate = airdrop_keypair(&mut svm);

    let appoint =
        update_delegate_instruction(owner.pubkey(), subaccount, delegate.pubkey(), margin_group);
    assert_transaction_success(send_tx(&mut svm, &owner, vec![appoint], &[]));

    let ix = update_delegate_instruction(
        delegate.pubkey(),
        subaccount,
        Keypair::new().pubkey(),
        margin_group,
    );

    assert_instruction_error(
        send_tx(&mut svm, &delegate, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_update_delegate_rejects_owner_as_delegate() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let (owner, subaccount, margin_group) = setup_subaccount(&mut svm, &setup);

    let ix = update_delegate_instruction(owner.pubkey(), subaccount, owner.pubkey(), margin_group);

    assert_instruction_error(
        send_tx(&mut svm, &owner, vec![ix], &[]),
        0,
        u32::from(StateError::DelegateIsOwner),
    );
}

#[test]
fn test_update_delegate_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let (owner, subaccount, margin_group) = setup_subaccount(&mut svm, &setup);
    let payer = airdrop_keypair(&mut svm);

    let ix = update_delegate_instruction(
        owner.pubkey(),
        subaccount,
        Keypair::new().pubkey(),
        margin_group,
    );

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}

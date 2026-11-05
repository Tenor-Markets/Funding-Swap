#![cfg(feature = "test-helpers")]

use funding_swap::state::StateError;
use solana_sdk::signer::Signer;

mod common;
use common::{
    airdrop_keypair, assert_asset_window_withdrawn, assert_instruction_error,
    assert_subaccount_collateral, assert_token_account_amount, assert_transaction_success,
    create_funded_token_account, create_margin_group, create_subaccount,
    create_token_account_for_mint, deposit_asset_instruction, fixed_raw, new_svm, read_asset_state,
    send_tx, set_asset_withdrawn_atoms, set_usdc_mint_account, setup_active_asset,
    setup_active_asset_with_custody, setup_active_margin_group, setup_governance, USDC_MINT,
};

#[test]
fn test_deposit_asset_credits_beneficiary_and_transfers_tokens() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    set_usdc_mint_account(&mut svm).expect("set USDC mint");

    let (asset, token_vault, _) =
        setup_active_asset_with_custody(&mut svm, &setup).expect("active asset");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 1, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let depositor = airdrop_keypair(&mut svm);
    let subaccount = create_subaccount(&mut svm, &setup, &depositor, 1, margin_group)
        .expect("create subaccount");
    let amount = 1_000_000u64;
    let source = create_funded_token_account(&mut svm, USDC_MINT, depositor.pubkey(), amount * 2)
        .expect("source token account");

    let ix = deposit_asset_instruction(
        depositor.pubkey(),
        source,
        token_vault,
        margin_group,
        subaccount,
        amount,
    );

    assert_transaction_success(send_tx(&mut svm, &depositor, vec![ix], &[]));

    svm.expire_blockhash();

    let ix = deposit_asset_instruction(
        depositor.pubkey(),
        source,
        token_vault,
        margin_group,
        subaccount,
        amount,
    );

    assert_transaction_success(send_tx(&mut svm, &depositor, vec![ix], &[]));

    assert_subaccount_collateral(&svm, subaccount, i128::from(amount * 2));
    assert_token_account_amount(&svm, source, 0);
    assert_token_account_amount(&svm, token_vault, amount * 2);

    // Deposits reduce net outflow into negative territory.
    assert_asset_window_withdrawn(
        &svm,
        asset,
        -i128::from(amount * 2),
        -i128::from(amount * 2),
        -i128::from(amount * 2),
    );
}

#[test]
fn test_deposit_asset_cancels_withdrawn_atoms_and_allows_negative() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    set_usdc_mint_account(&mut svm).expect("set USDC mint");

    let (asset, token_vault, _) =
        setup_active_asset_with_custody(&mut svm, &setup).expect("active asset");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 1, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let depositor = airdrop_keypair(&mut svm);
    let subaccount = create_subaccount(&mut svm, &setup, &depositor, 1, margin_group)
        .expect("create subaccount");
    let amount = 1_000_000u64;
    let source = create_funded_token_account(&mut svm, USDC_MINT, depositor.pubkey(), amount * 3)
        .expect("source token account");

    set_asset_withdrawn_atoms(&mut svm, asset, amount.into(), amount.into(), amount.into())
        .expect("set withdrawn atoms");

    let ix = deposit_asset_instruction(
        depositor.pubkey(),
        source,
        token_vault,
        margin_group,
        subaccount,
        amount,
    );

    assert_transaction_success(send_tx(&mut svm, &depositor, vec![ix], &[]));

    assert_asset_window_withdrawn(&svm, asset, 0, 0, 0);

    set_asset_withdrawn_atoms(&mut svm, asset, amount.into(), amount.into(), amount.into())
        .expect("set withdrawn atoms again");

    svm.expire_blockhash();

    let ix = deposit_asset_instruction(
        depositor.pubkey(),
        source,
        token_vault,
        margin_group,
        subaccount,
        amount,
    );

    assert_transaction_success(send_tx(&mut svm, &depositor, vec![ix], &[]));

    assert_asset_window_withdrawn(&svm, asset, 0, 0, 0);

    svm.expire_blockhash();

    let ix = deposit_asset_instruction(
        depositor.pubkey(),
        source,
        token_vault,
        margin_group,
        subaccount,
        amount,
    );

    assert_transaction_success(send_tx(&mut svm, &depositor, vec![ix], &[]));

    // Once withdrawals are fully cancelled, further deposits go negative.
    assert_asset_window_withdrawn(
        &svm,
        asset,
        -i128::from(amount),
        -i128::from(amount),
        -i128::from(amount),
    );
    assert_subaccount_collateral(&svm, subaccount, i128::from(amount * 3));
    assert_token_account_amount(&svm, token_vault, amount * 3);
}

#[test]
fn test_deposit_asset_rejects_wrong_token_vault() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    set_usdc_mint_account(&mut svm).expect("set USDC mint");

    let (asset, _token_vault, _) =
        setup_active_asset_with_custody(&mut svm, &setup).expect("active asset");
    let wrong_token_vault = create_token_account_for_mint(&mut svm, USDC_MINT, asset, None, None)
        .expect("wrong token vault");

    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 1, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");

    let depositor = airdrop_keypair(&mut svm);
    let subaccount = create_subaccount(&mut svm, &setup, &depositor, 1, margin_group)
        .expect("create subaccount");
    let source = create_funded_token_account(&mut svm, USDC_MINT, depositor.pubkey(), 1_000_000)
        .expect("source token account");

    let ix = deposit_asset_instruction(
        depositor.pubkey(),
        source,
        wrong_token_vault,
        margin_group,
        subaccount,
        1_000_000,
    );

    assert_instruction_error(
        send_tx(&mut svm, &depositor, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidTargetCustody),
    );
}

#[test]
fn test_deposit_asset_rejects_different_or_inactive_margin_group() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    set_usdc_mint_account(&mut svm).expect("set USDC mint");

    let (_asset, token_vault, _) =
        setup_active_asset_with_custody(&mut svm, &setup).expect("active asset");
    let margin_group =
        setup_active_margin_group(&mut svm, &setup, 1, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("active margin group");
    let other_group =
        setup_active_margin_group(&mut svm, &setup, 2, fixed_raw(1, 1), fixed_raw(3, 2))
            .expect("other active margin group");
    let pending_group = create_margin_group(&mut svm, &setup, 3, fixed_raw(1, 1), fixed_raw(3, 2))
        .expect("pending margin group");

    let depositor = airdrop_keypair(&mut svm);
    let subaccount = create_subaccount(&mut svm, &setup, &depositor, 1, margin_group)
        .expect("create subaccount");
    let source = create_funded_token_account(&mut svm, USDC_MINT, depositor.pubkey(), 2_000_000)
        .expect("source token account");

    let ix = deposit_asset_instruction(
        depositor.pubkey(),
        source,
        token_vault,
        other_group,
        subaccount,
        1_000_000,
    );

    assert_instruction_error(
        send_tx(&mut svm, &depositor, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidGroupID),
    );

    let ix = deposit_asset_instruction(
        depositor.pubkey(),
        source,
        token_vault,
        pending_group,
        subaccount,
        1_000_000,
    );

    assert_instruction_error(
        send_tx(&mut svm, &depositor, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidStatus),
    );
}
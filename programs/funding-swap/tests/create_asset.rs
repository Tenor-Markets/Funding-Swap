#![cfg(feature = "test-helpers")]

use funding_swap::state::{GovernanceCategory, StateError, USDC_MINT};
use funding_swap::CreateAssetArgs;
use solana_sdk::{pubkey::Pubkey, signer::Signer};

mod common;
use common::{
    airdrop_keypair, assert_account_not_signer, assert_asset_created, assert_instruction_error,
    assert_transaction_success, create_asset_args, create_asset_instruction,
    create_token_account_for_mint, create_usdc_token_account, fixed_raw, new_svm,
    send_create_asset, send_tx, send_without_signer, setup_governance, usdc_asset_address,
};

#[test]
fn test_create_asset_succeeds_for_resources_authority() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let args = create_asset_args();
    let asset = usdc_asset_address();
    let token_account = create_usdc_token_account(&mut svm, asset).expect("token account");

    assert_transaction_success(send_create_asset(&mut svm, &setup, token_account, args));

    assert_asset_created(&svm, asset, token_account, &args);
}

#[test]
fn test_create_asset_validates_oracle_and_window_config() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let asset = usdc_asset_address();
    let token_account = create_usdc_token_account(&mut svm, asset).expect("token account");
    let base = create_asset_args();

    type Mutator = fn(&mut CreateAssetArgs);

    let cases: [(&str, Mutator, StateError); 12] = [
        (
            "zero oracle max age",
            |args| args.max_age_seconds = 0,
            StateError::InvalidOraclePolicy,
        ),
        (
            "negative confidence",
            |args| args.max_confidence_ratio_raw = -1,
            StateError::InvalidOraclePolicy,
        ),
        (
            "negative EMA deviation",
            |args| args.max_ema_deviation_raw = -1,
            StateError::InvalidOraclePolicy,
        ),
        (
            "negative peg deviation",
            |args| args.max_peg_deviation_raw = -1,
            StateError::InvalidOraclePolicy,
        ),
        (
            "peg deviation at one",
            |args| args.max_peg_deviation_raw = fixed_raw(1, 1),
            StateError::InvalidOraclePolicy,
        ),
        (
            "zero short window",
            |args| args.window_durations = [0, 86_400, 604_800],
            StateError::InvalidWithdrawalWindowDurations,
        ),
        (
            "equal short and medium windows",
            |args| args.window_durations = [3_600, 3_600, 604_800],
            StateError::InvalidWithdrawalWindowDurations,
        ),
        (
            "equal medium and long windows",
            |args| args.window_durations = [3_600, 86_400, 86_400],
            StateError::InvalidWithdrawalWindowDurations,
        ),
        (
            "decreasing windows",
            |args| args.window_durations = [86_400, 3_600, 604_800],
            StateError::InvalidWithdrawalWindowDurations,
        ),
        (
            "equal short and medium limits",
            |args| {
                args.short_window_limit_atoms = 1_000_000;
                args.medium_window_limit_atoms = 1_000_000;
            },
            StateError::InvalidWithdrawalLimits,
        ),
        (
            "equal medium and long limits",
            |args| {
                args.medium_window_limit_atoms = 100_000_000;
                args.long_window_limit_atoms = 100_000_000;
            },
            StateError::InvalidWithdrawalLimits,
        ),
        (
            "decreasing limits",
            |args| {
                args.short_window_limit_atoms = 10_000_000;
                args.medium_window_limit_atoms = 1_000_000;
            },
            StateError::InvalidWithdrawalLimits,
        ),
    ];

    for (_name, mutate, expected_error) in cases {
        let mut args = base;
        mutate(&mut args);

        assert_instruction_error(
            send_create_asset(&mut svm, &setup, token_account, args),
            0,
            u32::from(expected_error),
        );
    }
}

#[test]
fn test_create_asset_rejects_invalid_token_account() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");
    let args = create_asset_args();
    let asset = usdc_asset_address();

    let wrong_mint =
        create_token_account_for_mint(&mut svm, Pubkey::new_unique(), asset, None, None)
            .expect("wrong mint token account");

    let wrong_owner = create_usdc_token_account(&mut svm, Pubkey::new_unique())
        .expect("wrong owner token account");

    let delegated =
        create_token_account_for_mint(&mut svm, USDC_MINT, asset, Some(Pubkey::new_unique()), None)
            .expect("delegated token account");

    let closable =
        create_token_account_for_mint(&mut svm, USDC_MINT, asset, None, Some(Pubkey::new_unique()))
            .expect("closable token account");

    let cases: [(Pubkey, StateError); 4] = [
        (wrong_mint, StateError::InvalidTokenAccount),
        (wrong_owner, StateError::InvalidTokenAuthority),
        (delegated, StateError::InvalidTokenAuthority),
        (closable, StateError::InvalidTokenAuthority),
    ];

    for (token_account, expected_error) in cases {
        assert_instruction_error(
            send_create_asset(&mut svm, &setup, token_account, args),
            0,
            u32::from(expected_error),
        );
    }
}

#[test]
fn test_create_asset_fails_when_signer_is_not_resources_authority() {
    let mut svm = new_svm().expect("svm");

    setup_governance(&mut svm).expect("governance setup");

    let attacker = airdrop_keypair(&mut svm);
    let asset = usdc_asset_address();
    let token_account = create_usdc_token_account(&mut svm, asset).expect("token account");

    let ix = create_asset_instruction(attacker.pubkey(), token_account, create_asset_args());

    assert_instruction_error(
        send_tx(&mut svm, &attacker, vec![ix], &[]),
        0,
        u32::from(StateError::InvalidAuthority),
    );
}

#[test]
fn test_create_asset_fails_when_authority_does_not_sign() {
    let mut svm = new_svm().expect("svm");

    let setup = setup_governance(&mut svm).expect("governance setup");

    let authority = setup.authority(GovernanceCategory::Resources);
    let payer = airdrop_keypair(&mut svm);
    let asset = usdc_asset_address();
    let token_account = create_usdc_token_account(&mut svm, asset).expect("token account");

    let ix = create_asset_instruction(authority.pubkey(), token_account, create_asset_args());

    let result = send_without_signer(&mut svm, &payer, ix, 0);

    assert_account_not_signer(result, 0);
}

# Test groups

Instruction-level tests are written in the style of the `shinsu/multisig`
suite: LiteSVM, a large shared helper module, and one dedicated test file per
instruction. Each group is implemented and reviewed as a unit before moving to
the next.

## Group 1 — Administration: global

- `initialize_governance.rs`
- `update_authorities.rs`

## Group 2 — Administration: margin group

- `create_margin_group.rs`
- `activate_margin_group.rs`
- `close_margin_group.rs`
- `pause_margin_group.rs`
- `resume_margin_group.rs`
- `tighten_group_risk.rs`
- `relax_group_risk.rs`

## Group 3 — Administration: market

- `initialize_market.rs`
- `activate_market.rs`
- `activate_market_group_membership.rs`
- `tighten_market_risk.rs`
- `relax_market_risk.rs`
- `update_fees_and_liquidation.rs`
- `restrict_market_status.rs`
- `restore_market_status.rs`

## Group 4 — Administration: vault

- `create_asset.rs`
- `activate_asset.rs`
- `collect_protocol_fees.rs`
- `tighten_withdrawal_limits.rs`
- `raise_withdrawal_limits.rs`

## Group 5 — Cleanup and subaccounts

- `create_subaccount.rs`
- `update_delegate.rs`
- `close_subaccount.rs`
- `close_empty_markets.rs`
- `close_market.rs`
- `purge_expired_order.rs`
- `risky_order_cleanup.rs`
- `close_only_cleanup.rs`

## Group 6 — Orders

- `place_order.rs`
- `cancel_order.rs`
- `modify_order.rs`
- `close_all_orders.rs`

## Group 7 — Liquidation and deleveraging

- `liquidate.rs`
- `deleverage.rs`

## Group 8 — Funds, materialization, settlement, and source lifecycle

- `deposit_asset.rs`
- `withdraw_asset.rs`
- `materialize_subaccount.rs`
- `settle_market_into_collateral.rs`
- `settle_all_markets_into_collateral.rs`
- `settle_matured_markets.rs`
- `refresh_market.rs`
- `seed_market_source.rs`
- `finalize_market.rs`
- `finalize_markets.rs`

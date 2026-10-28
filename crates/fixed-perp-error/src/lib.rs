use anchor_lang::prelude::*;

#[error_code]
#[derive(Eq, PartialEq)]
// Explicit discriminants preserve published error numbers after obsolete variants are removed.
pub enum FixedPerpError {
    #[msg("Arithmetic overflow, underflow, or an out-of-range numeric conversion.")]
    ArithmeticOverflow,
    #[msg("The signer is not authorized to perform this operation.")]
    InvalidAuthority,
    #[msg("Invalid account info.")]
    InvalidAccountInfo,
    #[msg("A timestamp is negative, in the future, or inconsistent with protocol time.")]
    InvalidClock,
    #[msg("The supplied configuration violates a protocol invariant.")]
    InvalidConfiguration,
    #[msg("Invalid source checkpoint.")]
    InvalidSourceCheckpoint,
    #[msg("Excess source delta.")]
    ExcessSourceDelta,
    #[msg("Invalid key.")]
    InvalidKey,
    #[msg("Invalid group id.")]
    InvalidGroupID,
    #[msg("Invalid market.")]
    InvalidMarket,
    #[msg("The supplied account or order reference is missing or inconsistent.")]
    InvalidReference,
    #[msg("Invalid token account.")]
    InvalidTokenAccount,
    #[msg("This operation is not permitted in the current lifecycle state.")]
    InvalidStatus,
    #[msg("Invalid asset state.")]
    InvalidAssetState,
    #[msg("Stale observation.")]
    StaleObservation,
    #[msg("The withdrawal exceeds at least one configured rolling-window limit.")]
    WithdrawalLimitExceeded,
    #[msg("Invalid tag.")]
    InvalidTag,
    #[msg("Not found.")]
    NotFound,
    #[msg("No free market slot, order reference, or book capacity remains.")]
    CapacityExhausted,
    #[msg("Conflict.")]
    Conflict,
    #[msg("Account not found.")]
    AccountNotFound,
    #[msg("Remaining accounts are missing, duplicated, out of order, or unused.")]
    InvalidRemainingAccounts,
    #[msg("All positions, settlement balances, and orders must be cleared before closing the subaccount.")]
    MarketsNotEmpty,
    #[msg("The resource still has live subaccounts and cannot be closed.")]
    SubaccountsNotEmpty,
    #[msg("Collateral debt must be cleared before closing the subaccount.")]
    NegativeCollateralBalance,
    #[msg("The resulting portfolio does not meet its initial-margin requirement.")]
    InsufficientInitialMargin,

    #[msg("Invalid account key.")]
    InvalidAccountKey,
    #[msg("Invalid account owner.")]
    InvalidAccountOwner,
    #[msg("Invalid account length.")]
    InvalidAccountLength,
    #[msg("Invalid discriminator.")]
    InvalidDiscriminator,
    #[msg("Invalid pda.")]
    InvalidPda,
    #[msg("Invalid bump.")]
    InvalidBump,
    #[msg("Invalid pool.")]
    InvalidPool,
    #[msg("Invalid mint.")]
    InvalidMint,
    #[msg("Invalid target custody.")]
    InvalidTargetCustody,
    #[msg("Invalid collateral custody.")]
    InvalidCollateralCustody,
    #[msg("Invalid side.")]
    InvalidSide,
    #[msg("Duplicate source account.")]
    DuplicateSourceAccount,
    #[msg("Missing delegation record.")]
    MissingDelegationRecord,
    #[msg("Unexpected delegation record.")]
    UnexpectedDelegationRecord,
    #[msg("Invalid delegation record key.")]
    InvalidDelegationRecordKey,
    #[msg("Invalid delegation record owner.")]
    InvalidDelegationRecordOwner,
    #[msg("Invalid original owner.")]
    InvalidOriginalOwner,
    #[msg("Invalid protocol checkpoint timestamp.")]
    InvalidProtocolCheckpointTimestamp,
    #[msg("Invalid source anchor.")]
    InvalidSourceAnchor,
    #[msg("Source too old.")]
    SourceTooOld,
    #[msg("Protocol checkpoint too old.")]
    ProtocolCheckpointTooOld,
    #[msg("Decreasing checkpoint.")]
    DecreasingCheckpoint,
    #[msg("Source delta too large.")]
    SourceDeltaTooLarge,
    #[msg("Normalization overflow.")]
    NormalizationOverflow,
    #[msg("Invalid utilization.")]
    InvalidUtilization,
    #[msg("Invalid rate config.")]
    InvalidRateConfig,
    #[msg("Invalid map layout.")]
    InvalidMapLayout,
    #[msg("Invalid map relationship.")]
    InvalidMapRelationship,
    #[msg("Invalid canonical quote mint.")]
    InvalidCanonicalQuoteMint,
    #[msg("Invalid asset slot.")]
    InvalidAssetSlot,
    #[msg("Invalid symbol.")]
    InvalidSymbol,
    #[msg("Invalid asset id.")]
    InvalidAssetId,
    #[msg("Invalid lot metadata.")]
    InvalidLotMetadata,
    #[msg("Invalid interval.")]
    InvalidInterval,
    #[msg("Invalid source timestamp.")]
    InvalidSourceTimestamp,
    #[msg("Future funding timestamp.")]
    FutureFundingTimestamp,
    #[msg("Stale funding.")]
    StaleFunding,
    #[msg("Future mark slot.")]
    FutureMarkSlot,
    #[msg("Stale mark.")]
    StaleMark,
    #[msg("Invalid mark.")]
    InvalidMark,
    #[msg("Cannot remove a subaccount from an empty live-subaccount count.")]
    SubaccountCountUnderflow,
    #[msg("Market fees cannot be collected while open interest or pending maker reserves remain.")]
    UnsettledOpenInterest,
    #[msg("Liquidation size must reduce an existing victim position without reversing it.")]
    InvalidLiquidationSize,
    #[msg("Recorded open interest cannot cover the position being replaced or removed.")]
    OpenInterestAccountingMismatch,
    #[msg("Deleveraging must reduce both counterparties without reversing either position.")]
    DeleveragingMustReduceBothPositions,
    #[msg("The matched rate exceeds the permitted mark-update bound.")]
    TradeRateOutsideMarkBound,
    #[msg("The order or fill would increase exposure in a close-only market.")]
    CloseOnlyViolation,
    #[msg("The resulting nonzero position is outside the configured size limits.")]
    PositionSizeOutOfBounds,
    #[msg("The order rate exceeds the permitted distance from the mark.")]
    OrderRateOutsideMarkBound,
    #[msg("A subaccount cannot trade against its own resting order.")]
    SelfTrade,
    #[msg("The post-only order would immediately match a resting order.")]
    PostOnlyOrderWouldCross,
    #[msg("The subaccount already has a resting order at this tick.")]
    DuplicateOrderAtTick,
    #[msg("Pending fills must be materialized before changing this order.")]
    UnmaterializedOrderFill,
    #[msg("The transition exceeds the market gross-open-interest cap.")]
    OpenInterestCapExceeded,
    #[msg("The pending maker reserve cannot cover this materialized fill.")]
    InsufficientOpenInterestReserve,
    #[msg("This health check requires an initial-margin calculation.")]
    ExpectedInitialMargin,
    #[msg("This health check requires a maintenance-margin calculation.")]
    ExpectedMaintenanceMargin,
    #[msg("The market and margin group must use the same collateral and settlement assets.")]
    MarketAssetMismatch,
    #[msg("Oracle confidence exceeds the configured maximum ratio.")]
    OracleConfidenceTooWide,
    #[msg("The oracle spot price deviates too far from its EMA.")]
    OracleEmaDeviationExceeded,
    #[msg("The oracle price update requires full guardian verification.")]
    OracleNotFullyVerified,
    #[msg("All market order references and outstanding order risk must be cleared before closing the slot.")]
    OutstandingMarketOrders,
    #[msg("The cached order size or potential margin cannot cover the order being removed.")]
    OrderRiskAccountingMismatch,
    #[msg("The two counterparties must be different subaccounts.")]
    DuplicateSubaccount,
    #[msg("The deleveraging winner must have a strictly higher maintenance-health ratio than the loser.")]
    WinnerNotHealthier,
    #[msg("The market must be activated before its configured start timestamp.")]
    MarketAlreadyStarted,
    #[msg("An occupied book tick is missing its tick metadata.")]
    MissingTickMetadata,
    #[msg("The subaccount has no slot for the requested market.")]
    MarketNotEntered,
    #[msg("The requested order does not belong to a live subaccount order reference.")]
    OrderNotFound,
    #[msg("The proposed risk change moves against the authority permitted direction.")]
    RiskUpdateWrongDirection,
    #[msg("Only the risk-recovery authority can change this field.")]
    RecoveryAuthorityRequired,
    #[msg("The close-only maker subaccount was not supplied.")]
    MissingMakerSubaccount,
    #[msg("A fill-or-kill order could not be filled completely.")]
    FillOrKillUnfilled,
    #[msg("A portfolio market is missing its authenticated price observation.")]
    MissingMarketObservation,
    #[msg("Market start must be nonnegative and strictly before a positive maturity timestamp.")]
    InvalidMarketTiming,
    #[msg("Market sizes, open-interest cap, observation window and margin time threshold must be positive and mutually consistent.")]
    InvalidMarketLimits,
    #[msg("Market risk, fee and liquidation parameters must be nonnegative, and initial margin must not be below maintenance margin.")]
    InvalidMarketFactors,
    #[msg("Asset decimals or tick scaling exceed the supported precision or disagree with the market configuration.")]
    InvalidAssetDecimals,
    #[msg("The deleveraging allocation fraction must be between zero and one inclusive.")]
    InvalidDeleveragingFraction,
    #[msg("Order expiry must be in the future and no later than market maturity.")]
    InvalidOrderExpiry,
    #[msg("The order batch exceeds capacity, is empty, or has an inconsistent result buffer.")]
    InvalidOrderBatch,
    #[msg("Modifying an order cannot change its long or short side.")]
    OrderSideChangeNotAllowed,
    #[msg("The minimum tick cannot exceed the maximum tick.")]
    InvalidTickRange,

    #[msg("The normalized oracle price, EMA, confidence or confidence bounds are invalid.")]
    InvalidOracleObservation = 110,
    #[msg(
        "Oracle prices must be nonnegative before scaling and positive when used for conversion."
    )]
    InvalidOraclePrice,
    #[msg(
        "Discount must be greater than zero and at most one; liability factor must be at least one."
    )]
    InvalidValuationFactors,
    #[msg("Subaccount initialization requires a nonzero owner, subaccount ID and group ID.")]
    InvalidSubaccountIdentity,
    #[msg("The market rate floor cannot be negative.")]
    NegativeRateFloor,
    #[msg("A maintenance-health ratio requires a positive maintenance-margin requirement.")]
    NoMaintenanceMargin,
    #[msg("An account with negative equity requires bad-debt handling rather than ordinary liquidation.")]
    NegativeEquity,
    #[msg("The account meets maintenance margin and cannot be liquidated.")]
    NotLiquidatable,
    #[msg("The market slot index is outside the subaccount slot array.")]
    InvalidMarketSlot,
    #[msg(
        "The slot market ID or recorded live-subaccount count does not match the supplied market."
    )]
    MarketSlotMismatch,
    #[msg("An order must have a nonzero size.")]
    ZeroOrderSize,
    #[msg("The submitted order size is outside the book minimum and maximum.")]
    OrderSizeOutOfBounds,
    #[msg("Order size limits or the per-subaccount order limit are invalid.")]
    InvalidOrderBookLimits,
    #[msg("Tick step must lie between one and the maximum supported tick step.")]
    InvalidTickStep,
    #[msg("The Pyth feed ID does not match the feed configured for this asset.")]
    InvalidOracleFeed,
    #[msg("This asset requires a Pyth price-update account.")]
    MissingOraclePriceAccount,
    #[msg("The supplied account cannot be decoded as a Pyth price update.")]
    InvalidOracleAccountData,
    #[msg("The account data is already borrowed incompatibly with this operation.")]
    AccountDataBorrowFailed,
    #[msg("Withdrawal window durations must be positive and strictly increasing.")]
    InvalidWithdrawalWindowDurations,
    #[msg("Oracle confidence, EMA-deviation and peg-deviation limits must be nonnegative.")]
    InvalidOraclePolicy,

    #[msg("Every governance authority must be a nonzero public key.")]
    InvalidGovernanceAuthority = 133,
    #[msg("The creation-fee destination must match the configured nonzero public key.")]
    InvalidCreationFeeDestination,
    #[msg("Authority rotation requires both a category and its replacement authority.")]
    IncompleteAuthorityUpdate,
    #[msg("Critical health ratio must be positive and cancellation ratio must be at least critical health ratio.")]
    InvalidGroupHealthRatios,
    #[msg("The market must be finalized as matured before this operation.")]
    MarketNotMatured,
    #[msg("This operation must occur before the market maturity timestamp.")]
    MarketExpired,

    #[msg("The market funding source must be seeded before this operation.")]
    MarketSourceNotSeeded = 140,
    #[msg("Liquidation health ratio must be at least zero and strictly below one.")]
    InvalidLiquidationHealthRatio,
    #[msg("A required market account was not supplied.")]
    MissingMarketAccount,
    #[msg("A market is missing its authenticated underlying-asset observation.")]
    MissingUnderlyingObservation,
    #[msg("The adapter payload has an invalid size or alignment for its configured venue.")]
    InvalidAdapterPayload,
    #[msg("Jupiter adapter configuration requires a supported venue layout and nonzero custody, pool and mint addresses.")]
    InvalidJupiterConfiguration,
    #[msg("Phoenix adapter configuration requires nonzero source accounts, symbol, tick size, intervals and freshness limits.")]
    InvalidPhoenixConfiguration,
    #[msg("Settlement fee rate must be nonnegative.")]
    InvalidFeeRate,
    #[msg("A resting closing order requires its worst executable rate for margin calculation.")]
    MissingClosingOrderRate,
    #[msg("The requested market-status transition is not permitted for this authority.")]
    InvalidMarketStatusTransition,
    #[msg("The current timestamp has not reached market maturity.")]
    MaturityNotReached,
    #[msg("The subaccount has no free slot for another market.")]
    MarketSlotLimitExceeded,
    #[msg("The subaccount has reached its total live-order-reference limit.")]
    SubaccountOrderLimitExceeded,
    #[msg("The subaccount has reached the live-order limit for this market.")]
    MarketOrderLimitExceeded,
    #[msg("The market book has no free order node or tick-metadata slot.")]
    BookCapacityExhausted,
    #[msg("The instruction supplies more maker subaccounts than the supported maximum.")]
    TooManyMakerAccounts,
    #[msg("The order time-in-force value is not supported.")]
    InvalidTimeInForce,

    #[msg("The requested governance authority category does not exist.")]
    InvalidGovernanceCategory = 158,
    #[msg("The market funding adapter kind is not supported.")]
    InvalidAdapterKind,
    #[msg("The requested venue payload does not match the market adapter kind.")]
    AdapterKindMismatch,
    #[msg("The subaccount owner cannot also be configured as its delegate.")]
    DelegateIsOwner,
    #[msg("The custody token account must be controlled by the asset-state account.")]
    InvalidTokenAuthority,
    #[msg("The authenticated USDC price is outside the permitted peg band.")]
    UsdcOutsidePegBand,
}

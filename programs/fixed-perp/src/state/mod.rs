pub mod error;
pub mod governance;
pub mod market;
pub mod order_book;
pub mod user;
pub mod valuation;
pub mod vault;

pub use error::{StateError, StateResult};
pub use fixed_perp_math::FixedPoint;
pub use governance::{
    AssetGovernance, ChangePolicy, CircuitBreakerProposal, FeeRateProposal, GlobalGovernanceConfig,
    GlobalGovernancePolicy, GlobalGovernanceProposal, GovernanceCategory, GroupMarginProposal,
    LiquidationProposal, MarginGroupGovernance, MarketGovernance, MarketMarginProposal,
    MarketRiskProposal, MarketStatusProposal, OptionalFixedPoint, OptionalPubkey, OptionalU128,
    OptionalU64, OptionalU8, ProposalWindow, TypedProposal,
};
pub use market::{
    aggregate_is_closing_action, BatchOrder, ClosingAction, JupiterAdapterPayload, MakerSubaccount,
    MarginGroup, MarginRequirement, MarginRequirementType, MarketConfig, MarketGroupMembership,
    MarketState, MarketStatus, ModifyOrderIntent, PendingMarketGroupMembership,
    PhoenixAdapterPayload, PlaceOrderBatchIntent, PlaceOrderIntent, PlaceOrderResult,
    PortfolioHealth, PortfolioMarketObservation, VenueAdapterConfig, VenueAdapterKind, VenueLayout,
    ADAPTER_JUPITER, ADAPTER_PHOENIX, GROUP_ACTIVE, GROUP_PENDING, JUPITER_PAYLOAD_RESERVED,
    MARGIN_GROUP_RESERVED, MARKET_ACTIVE, MARKET_CLOSE_ONLY, MARKET_CONFIG_RESERVED,
    MARKET_MATURED, MARKET_PAUSED, MARKET_PENDING, MARKET_STATE_RESERVED, PHOENIX_PAYLOAD_RESERVED,
    SECONDS_PER_YEAR, VENUE_CONFIG_RESERVED,
};
pub use order_book::{
    HierarchicalBitmap, OrderBook, OrderId, OrderNode, Side, TickMetadata, TimeInForce,
    BOOK_CAPACITY, FREE_INDEX, ORDER_BOOK_RESERVED, ORDER_NODE_RESERVED, TICK_DIRECTORY_LEN,
    TICK_METADATA_RESERVED,
};
pub use user::{
    MarketSlot, OrderRef, OrderRiskAggregate, Subaccount, SubaccountStatus, MARKET_SLOT_RESERVED,
    MAX_MARKETS, MAX_ORDERS, SUBACCOUNT_ACTIVE, SUBACCOUNT_RESERVED,
};
pub use valuation::{
    OracleObservation, PriceEndpoint, PriceModel, SettlementRatio, ValuationFactors,
    PRICE_MODEL_FIXED, PRICE_MODEL_FLOATING, PRICE_MODEL_PEGGED,
};
pub use vault::{
    AssetState, PendingProtocolFeeCollection, WithdrawalWindow, ASSET_ACTIVE, ASSET_PENDING,
    ONE_HOUR_SECONDS,
};
